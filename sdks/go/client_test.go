package beatbox

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func TestJobURLEncoding(t *testing.T) {
	c := New("http://127.0.0.1:7300/")

	tests := []struct {
		name    string
		id      string
		want    string
		wantErr bool
	}{
		{name: "uuid", id: "1f2e3d4c-0000-1111-2222-333344445555", want: "http://127.0.0.1:7300/v1/jobs/1f2e3d4c-0000-1111-2222-333344445555"},
		{name: "path traversal", id: "../execute", want: "http://127.0.0.1:7300/v1/jobs/..%2Fexecute"},
		{name: "query injection", id: "x?k=v", want: "http://127.0.0.1:7300/v1/jobs/x%3Fk=v"},
		{name: "empty", id: "", wantErr: true},
		{name: "dot", id: ".", wantErr: true},
		{name: "dotdot", id: "..", wantErr: true},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := c.jobURL(tt.id)
			if tt.wantErr {
				if err == nil {
					t.Fatalf("jobURL(%q) = %q, want error", tt.id, got)
				}
				return
			}
			if err != nil {
				t.Fatalf("jobURL(%q) unexpected error: %v", tt.id, err)
			}
			if got != tt.want {
				t.Fatalf("jobURL(%q) = %q, want %q", tt.id, got, tt.want)
			}
		})
	}
}

func TestExecuteRequestJSONRoundTrip(t *testing.T) {
	wall := uint64(5000)
	entry := "run"
	req := ExecuteRequest{
		Lane:       LaneWasm,
		Source:     SourceWasmWat("(module)"),
		Entrypoint: &entry,
		Input:      map[string]any{"n": float64(41)},
		Policy: &Policy{
			Limits: &Limits{WallMs: &wall},
		},
	}

	data, err := json.Marshal(req)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}

	// A partial policy must only carry the fields that were set.
	got := string(data)
	for _, want := range []string{
		`"lane":"wasm"`,
		`"kind":"wasm_wat"`,
		`"text":"(module)"`,
		`"entrypoint":"run"`,
		`"wall_ms":5000`,
	} {
		if !strings.Contains(got, want) {
			t.Errorf("marshaled request missing %s\n got: %s", want, got)
		}
	}
	for _, absent := range []string{"cpu_ms", "memory_bytes", "idempotency_key", "stdin", "determinism"} {
		if strings.Contains(got, absent) {
			t.Errorf("marshaled request unexpectedly contains %q\n got: %s", absent, got)
		}
	}

	var back ExecuteRequest
	if err := json.Unmarshal(data, &back); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if back.Lane != LaneWasm || back.Source.Kind != SourceKindWasmWat || back.Source.Text != "(module)" {
		t.Errorf("round-trip mismatch: %+v", back)
	}
	if back.Entrypoint == nil || *back.Entrypoint != "run" {
		t.Errorf("entrypoint lost in round-trip: %+v", back.Entrypoint)
	}
	if back.Policy == nil || back.Policy.Limits == nil || back.Policy.Limits.WallMs == nil || *back.Policy.Limits.WallMs != 5000 {
		t.Errorf("policy lost in round-trip: %+v", back.Policy)
	}
}

func TestExecutionResultJSONRoundTrip(t *testing.T) {
	// Nullable metrics and an unknown extra field must both survive.
	body := `{
		"status":"ok",
		"value":42,
		"stdout":"","stdout_truncated":false,
		"stderr":"","stderr_truncated":false,
		"metrics":{"wall_time_ms":12,"cpu_time_ms":null,"fuel_used":99,"peak_memory_bytes":null},
		"lane":"wasm","deterministic":true,"inputs_digest":"sha256:abc",
		"engine_version":"w0","beatbox_version":"0.1.0",
		"effective_isolation":{"os":"linux","mechanisms":["seccomp"],"downgrades":[]},
		"egress":[],
		"future_field":"ignored"
	}`

	var res ExecutionResult
	if err := json.Unmarshal([]byte(body), &res); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if res.Status != ExecutionStatusOK {
		t.Errorf("status = %q", res.Status)
	}
	var value int
	if err := json.Unmarshal(res.Value, &value); err != nil || value != 42 {
		t.Errorf("value = %d (err %v), want 42", value, err)
	}
	if res.Metrics.WallTimeMs != 12 {
		t.Errorf("wall_time_ms = %d", res.Metrics.WallTimeMs)
	}
	if res.Metrics.CPUTimeMs != nil {
		t.Errorf("cpu_time_ms = %v, want nil", res.Metrics.CPUTimeMs)
	}
	if res.Metrics.FuelUsed == nil || *res.Metrics.FuelUsed != 99 {
		t.Errorf("fuel_used = %v, want 99", res.Metrics.FuelUsed)
	}
	if res.Metrics.PeakMemoryBytes != nil {
		t.Errorf("peak_memory_bytes = %v, want nil", res.Metrics.PeakMemoryBytes)
	}
}

func TestExecuteMockServer(t *testing.T) {
	var gotPath, gotMethod, gotKey, gotCT string
	var gotBody ExecuteRequest

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotPath = r.URL.Path
		gotMethod = r.Method
		gotKey = r.Header.Get(apiKeyHeader)
		gotCT = r.Header.Get("Content-Type")
		b, _ := io.ReadAll(r.Body)
		_ = json.Unmarshal(b, &gotBody)

		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		io.WriteString(w, `{
			"status":"ok","value":42,
			"stdout":"","stdout_truncated":false,
			"stderr":"","stderr_truncated":false,
			"metrics":{"wall_time_ms":3,"cpu_time_ms":null,"fuel_used":null,"peak_memory_bytes":null},
			"lane":"wasm","deterministic":true,"inputs_digest":"d",
			"engine_version":"w0","beatbox_version":"0.1.0",
			"effective_isolation":{"os":"linux","mechanisms":[],"downgrades":[]},
			"egress":[]
		}`)
	}))
	defer srv.Close()

	c := New(srv.URL, WithAPIKey("secret-key"))
	res, err := c.Execute(context.Background(), WasmWatRequest(
		`(module (func (export "run") (param i64) (result i64) local.get 0 i64.const 1 i64.add))`,
		map[string]any{"n": 41}))
	if err != nil {
		t.Fatalf("Execute: %v", err)
	}

	if gotMethod != http.MethodPost {
		t.Errorf("method = %q", gotMethod)
	}
	if gotPath != "/v1/execute" {
		t.Errorf("path = %q", gotPath)
	}
	if gotKey != "secret-key" {
		t.Errorf("api key header = %q", gotKey)
	}
	if gotCT != "application/json" {
		t.Errorf("content-type = %q", gotCT)
	}
	if gotBody.Lane != LaneWasm || gotBody.Source.Kind != SourceKindWasmWat {
		t.Errorf("server received unexpected request body: %+v", gotBody)
	}

	var value int
	if err := json.Unmarshal(res.Value, &value); err != nil || value != 42 {
		t.Errorf("value = %d (err %v), want 42", value, err)
	}
}

func TestExecuteAPIError(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusUnprocessableEntity)
		io.WriteString(w, `{"error":{"code":"invalid_source","message":"unsupported source kind"}}`)
	}))
	defer srv.Close()

	c := New(srv.URL, WithAPIKey("k"))
	_, err := c.Execute(context.Background(), WasmWatRequest("(module)", nil))
	if err == nil {
		t.Fatal("expected error")
	}
	var apiErr *APIError
	if !errors.As(err, &apiErr) {
		t.Fatalf("error is not *APIError: %v", err)
	}
	if apiErr.Status != http.StatusUnprocessableEntity {
		t.Errorf("status = %d", apiErr.Status)
	}
	if apiErr.Code != "invalid_source" {
		t.Errorf("code = %q", apiErr.Code)
	}
	if !strings.Contains(apiErr.Error(), "unsupported source kind") {
		t.Errorf("message not surfaced: %q", apiErr.Error())
	}
}

func TestHealthUnauthenticated(t *testing.T) {
	var sawKey bool
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get(apiKeyHeader) != "" {
			sawKey = true
		}
		io.WriteString(w, `{"status":"ok","version":"0.1.0","uptime_s":1}`)
	}))
	defer srv.Close()

	c := New(srv.URL, WithAPIKey("must-not-be-sent"))
	raw, err := c.Health(context.Background())
	if err != nil {
		t.Fatalf("Health: %v", err)
	}
	if sawKey {
		t.Error("Health must not send the API key header")
	}
	if !strings.Contains(string(raw), `"status":"ok"`) {
		t.Errorf("unexpected health payload: %s", raw)
	}
}

func TestNoRedirectFollow(t *testing.T) {
	// A redirect must not be followed (which would leak the key cross-origin).
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, "http://example.invalid/elsewhere", http.StatusFound)
	}))
	defer srv.Close()

	c := New(srv.URL, WithAPIKey("k"), WithTimeout(2*time.Second))
	_, err := c.Capabilities(context.Background())
	var apiErr *APIError
	if !errors.As(err, &apiErr) {
		t.Fatalf("expected *APIError from unfollowed redirect, got %v", err)
	}
	if apiErr.Status != http.StatusFound {
		t.Errorf("status = %d, want 302", apiErr.Status)
	}
}
