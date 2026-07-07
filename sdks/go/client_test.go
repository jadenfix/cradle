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

type roundTripFunc func(*http.Request) (*http.Response, error)

func (f roundTripFunc) RoundTrip(req *http.Request) (*http.Response, error) {
	return f(req)
}

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

func TestJobURLEscapesIDWithEscapedBasePrefix(t *testing.T) {
	c := New("https://daemon.example/proxy/a%20b")
	got, err := c.jobURL("../execute")
	if err != nil {
		t.Fatalf("jobURL: %v", err)
	}
	want := "https://daemon.example/proxy/a%20b/v1/jobs/..%2Fexecute"
	if got != want {
		t.Fatalf("jobURL = %q, want %q", got, want)
	}
}

func TestClientBaseURLValidationAllowsSecureAndLoopbackLiteral(t *testing.T) {
	tests := []struct {
		name string
		raw  string
		want string
	}{
		{name: "https origin", raw: "https://daemon.example", want: "https://daemon.example"},
		{name: "https prefix", raw: "https://daemon.example/proxy/beatbox/", want: "https://daemon.example/proxy/beatbox"},
		{name: "ipv4 loopback http", raw: "http://127.0.0.1:7300", want: "http://127.0.0.1:7300"},
		{name: "ipv6 loopback http", raw: "http://[::1]:7300", want: "http://[::1]:7300"},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			c := New(tt.raw)
			if c.baseURLErr != nil {
				t.Fatalf("New(%q) rejected base URL: %v", tt.raw, c.baseURLErr)
			}
			if c.baseURL != tt.want {
				t.Fatalf("baseURL = %q, want %q", c.baseURL, tt.want)
			}
		})
	}
}

func TestClientBaseURLValidationRejectsUnsafeOrigins(t *testing.T) {
	for _, raw := range []string{
		" http://127.0.0.1:7300",
		"http://127.0.0.1:7300 ",
		"http://localhost:7300",
		"http://127.1:7300",
		"http://10.0.0.1:7300",
		"http://192.168.1.10:7300",
		"http://example.com",
		"ftp://127.0.0.1:7300",
		"https://user@example.com",
		"https://user:pass@example.com",
		"https://example.com?api=v1",
		"https://example.com#fragment",
		"/relative",
	} {
		t.Run(raw, func(t *testing.T) {
			c := New(raw)
			if c.baseURLErr == nil {
				t.Fatalf("New(%q) accepted unsafe base URL", raw)
			}
		})
	}
}

func TestClientBaseURLValidationRejectsRetargetingPathPrefixes(t *testing.T) {
	for _, raw := range []string{
		"https://example.com/base/../admin",
		"https://example.com/base/%2e%2e/admin",
		"https://example.com/base/%2E/admin",
		"https://example.com/base/%2Fadmin",
		"https://example.com/base/%5Cadmin",
		"https://example.com/base\\admin",
		"https://example.com/base/%",
	} {
		t.Run(raw, func(t *testing.T) {
			c := New(raw)
			if c.baseURLErr == nil {
				t.Fatalf("New(%q) accepted retargeting path prefix", raw)
			}
		})
	}
}

func TestInvalidBaseURLPreventsRequestAndKeyLeak(t *testing.T) {
	var calls int
	c := New("http://example.com", WithAPIKey("secret-key"), WithHTTPClient(&http.Client{
		Transport: roundTripFunc(func(req *http.Request) (*http.Response, error) {
			calls++
			return nil, errors.New("transport should not run")
		}),
	}))

	_, err := c.Capabilities(context.Background())
	if err == nil {
		t.Fatal("Capabilities succeeded with invalid base URL")
	}
	if calls != 0 {
		t.Fatalf("transport calls = %d, want 0", calls)
	}
	if strings.Contains(err.Error(), "secret-key") {
		t.Fatalf("error leaked api key: %v", err)
	}
}

func TestClientPreservesValidatedPathPrefix(t *testing.T) {
	var gotURL, gotKey string
	c := New("https://daemon.example/proxy/beatbox/", WithAPIKey("secret-key"), WithHTTPClient(&http.Client{
		Transport: roundTripFunc(func(req *http.Request) (*http.Response, error) {
			gotURL = req.URL.String()
			gotKey = req.Header.Get(apiKeyHeader)
			return &http.Response{
				StatusCode: http.StatusOK,
				Header:     http.Header{"Content-Type": []string{"application/json"}},
				Body:       io.NopCloser(strings.NewReader(`{"lanes":[]}`)),
				Request:    req,
			}, nil
		}),
	}))

	_, err := c.Capabilities(context.Background())
	if err != nil {
		t.Fatalf("Capabilities: %v", err)
	}
	if gotURL != "https://daemon.example/proxy/beatbox/v1/capabilities" {
		t.Fatalf("url = %q", gotURL)
	}
	if gotKey != "secret-key" {
		t.Fatalf("api key = %q", gotKey)
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

func TestAdmitBrowserSessionMockServer(t *testing.T) {
	var gotPath, gotMethod, gotKey, gotCT string
	var gotBody map[string]any

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
			"decision":"rejected",
			"runnable_browser_sessions":false,
			"requested_level":"os_isolated",
			"selected_level":null,
			"actor":"agent",
			"sensitivity":"sensitive",
			"target_origins":["https://example.com"],
			"credential_mode":"no_credentials",
			"artifact_mode":"discard",
			"requested_controls":["egress_policy","remote_worker_isolation"],
			"requested_profile_controls":["fresh_profile","no_ambient_credentials","egress_policy","local_network_block","os_process_isolation","teardown_proof"],
			"missing_controls":["remote_worker_isolation"],
			"level_satisfies_requested_controls":false,
			"intent_warnings":[],
			"guard_plan":{
				"network":{
					"allowed_origins":["https://example.com"],
					"deny_private_networks":true,
					"deny_localhost":true,
					"deny_metadata_endpoints":true,
					"require_dns_rebinding_protection":true,
					"require_redirect_revalidation":true,
					"require_proxy_enforcement":true,
					"outbound_network_disabled_without_proxy":true
				},
				"credentials":{
					"mode":"no_credentials",
					"ambient_credentials_allowed":false,
					"user_mediation_required":false,
					"scoped_secret_channel_required":false
				},
				"storage":{
					"mode":"discard",
					"plaintext_persistence_allowed":false,
					"explicit_artifact_allowlist_required":false,
					"encryption_required_for_persistence":false,
					"teardown_proof_required":true
				},
				"required_runtime_guards":[
					"browser launcher bound to the selected sandbox profile",
					"production-path admission check before launch",
					"teardown proof before reporting session completion",
					"fresh profile directory with no host browser state",
					"deny-by-default egress proxy that revalidates DNS, redirects, and final socket targets",
					"loopback, LAN, shared, link-local, and metadata address block",
					"OS jail or microVM boundary around the browser process"
				]
			},
			"adapter_handoff":{
				"contract_version":"browser-adapter-v1",
				"launch_endpoint":null,
				"launchable":false,
				"handoff_fields":["requested_level","actor","sensitivity","target_origins","credential_mode","artifact_mode","requested_controls","guard_plan"],
				"required_completion_proofs":[
					"browser process exited or was killed",
					"temporary profile directory removed",
					"plaintext artifacts outside the explicit allowlist removed",
					"egress proxy log sealed or discarded according to artifact_mode"
				],
				"unavailable_reason":"no browser adapter launch endpoint is implemented by this daemon"
			},
			"downgrade_allowed":false,
			"reasons":["no runnable browser sandbox"],
			"required_next_steps":["implement a browser launcher"],
			"profiles_endpoint":"/v1/browser/profiles"
		}`)
	}))
	defer srv.Close()

	c := New(srv.URL, WithAPIKey("secret-key"))
	raw, err := c.AdmitBrowserSession(context.Background(), map[string]any{
		"requested_level": "os_isolated",
		"actor":           "agent",
		"sensitivity":     "sensitive",
		"target_origins":  []any{"https://example.com"},
		"credential_mode": "no_credentials",
		"artifact_mode":   "discard",
		"required_controls": []any{
			"egress_policy",
			"remote_worker_isolation",
		},
	})
	if err != nil {
		t.Fatalf("AdmitBrowserSession: %v", err)
	}

	if gotMethod != http.MethodPost {
		t.Errorf("method = %q", gotMethod)
	}
	if gotPath != "/v1/browser/admit" {
		t.Errorf("path = %q", gotPath)
	}
	if gotKey != "secret-key" {
		t.Errorf("api key header = %q", gotKey)
	}
	if gotCT != "application/json" {
		t.Errorf("content-type = %q", gotCT)
	}
	if gotBody["requested_level"] != "os_isolated" || gotBody["actor"] != "agent" || gotBody["sensitivity"] != "sensitive" {
		t.Errorf("server received unexpected admission body: %+v", gotBody)
	}
	if origins, ok := gotBody["target_origins"].([]any); !ok || len(origins) != 1 || origins[0] != "https://example.com" {
		t.Errorf("server received unexpected origins: %+v", gotBody["target_origins"])
	}
	if gotBody["credential_mode"] != "no_credentials" || gotBody["artifact_mode"] != "discard" {
		t.Errorf("server received unexpected intent modes: %+v", gotBody)
	}
	if controls, ok := gotBody["required_controls"].([]any); !ok || len(controls) != 2 || controls[1] != "remote_worker_isolation" {
		t.Errorf("server received unexpected controls: %+v", gotBody["required_controls"])
	}
	if !strings.Contains(string(raw), `"decision":"rejected"`) {
		t.Errorf("unexpected admission response: %s", raw)
	}
	if !strings.Contains(string(raw), `"missing_controls":["remote_worker_isolation"]`) {
		t.Errorf("missing controls not surfaced: %s", raw)
	}
	if !strings.Contains(string(raw), `"target_origins":["https://example.com"]`) {
		t.Errorf("target origins not surfaced: %s", raw)
	}
	if !strings.Contains(string(raw), `"require_proxy_enforcement":true`) {
		t.Errorf("guard plan not surfaced: %s", raw)
	}
	if !strings.Contains(string(raw), "final socket targets") {
		t.Errorf("egress runtime guard not surfaced: %s", raw)
	}
	if !strings.Contains(string(raw), "OS jail") {
		t.Errorf("OS runtime guard not surfaced: %s", raw)
	}
	if !strings.Contains(string(raw), `"launchable":false`) {
		t.Errorf("adapter handoff did not stay fail-closed: %s", raw)
	}
	if !strings.Contains(string(raw), "temporary profile directory removed") {
		t.Errorf("adapter proof contract not surfaced: %s", raw)
	}
}

func TestBrowserAdapterContractMockServer(t *testing.T) {
	var gotMethod, gotPath, gotKey, gotCT string
	var sawBody bool
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotMethod = r.Method
		gotPath = r.URL.Path
		gotKey = r.Header.Get(apiKeyHeader)
		gotCT = r.Header.Get("Content-Type")
		b, _ := io.ReadAll(r.Body)
		sawBody = len(b) > 0

		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		io.WriteString(w, `{
			"adapter_contract":{"version":"browser-adapter-v1","status":"planned","launch_endpoint":null,"handoff_fields":["guard_plan"],"required_guard_fields":["guard_plan.network.deny_metadata_endpoints"],"required_completion_proofs":["temporary profile directory removed"],"unavailable_reason":"no browser adapter launch endpoint is implemented by this daemon"},
			"conformance_profile":{"profile_version":"browser-adapter-conformance-v1","field_complete_manifest":{"adapter_id":"tempo-conformance-adapter-v1","contract_version":"browser-adapter-v1","launch_endpoint":"https://adapter.example/launch","supported_levels":["os_isolated"],"supported_controls":["os_process_isolation"],"guard_fields":["guard_plan.network.deny_metadata_endpoints"],"completion_proofs":["temporary profile directory removed"]},"field_complete_expectation":{"decision":"rejected","manifest_complete":false,"launchable":false,"trusted_for_sensitive_work":false,"endpoint_network_policy_bound":false,"missing_levels":[],"missing_controls":[],"missing_guard_fields":[],"missing_completion_proofs":[]},"required_cases":[],"notes":["not a launch grant"]},
			"required_levels":["os_isolated"],
			"required_controls":["os_process_isolation"],
			"launchable":false,
			"trusted_for_sensitive_work":false,
			"endpoint_network_policy_bound":false,
			"notes":["not adapter registration"]
		}`)
	}))
	defer srv.Close()

	c := New(srv.URL, WithAPIKey("secret-key"))
	raw, err := c.BrowserAdapterContract(context.Background())
	if err != nil {
		t.Fatalf("BrowserAdapterContract: %v", err)
	}
	if gotMethod != http.MethodGet || gotPath != "/v1/browser/adapter/contract" {
		t.Fatalf("request = %s %s", gotMethod, gotPath)
	}
	if gotKey != "secret-key" || gotCT != "" {
		t.Fatalf("headers key=%q content-type=%q", gotKey, gotCT)
	}
	if sawBody {
		t.Fatal("GET /v1/browser/adapter/contract should not send a request body")
	}
	if !strings.Contains(string(raw), `"launchable":false`) || !strings.Contains(string(raw), `"browser-adapter-conformance-v1"`) {
		t.Fatalf("contract response not surfaced: %s", raw)
	}
}

func TestIssueBrowserAdapterCapabilityMockServer(t *testing.T) {
	var gotMethod, gotPath, gotKey, gotCT string
	var gotBody map[string]any
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotMethod = r.Method
		gotPath = r.URL.Path
		gotKey = r.Header.Get(apiKeyHeader)
		gotCT = r.Header.Get("Content-Type")
		b, _ := io.ReadAll(r.Body)
		_ = json.Unmarshal(b, &gotBody)

		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		io.WriteString(w, `{
			"same_user_capability":"bbx-browser-adapter-cap-v1.fixture.not-a-secret",
			"expires_at":"2026-07-06T20:00:00Z",
			"ttl_seconds":60,
			"actor":"agent",
			"sensitivity":"sensitive",
			"adapter_id":"tempo-os-jail-v1",
			"registration_endpoint":"/v1/browser/adapter/register",
			"notes":["keep it out of logs"]
		}`)
	}))
	defer srv.Close()

	req := map[string]any{
		"actor":       "agent",
		"sensitivity": "sensitive",
		"adapter_id":  "tempo-os-jail-v1",
		"ttl_seconds": float64(60),
	}
	c := New(srv.URL, WithAPIKey("secret-key"))
	raw, err := c.IssueBrowserAdapterCapability(context.Background(), req)
	if err != nil {
		t.Fatalf("IssueBrowserAdapterCapability: %v", err)
	}
	if gotMethod != http.MethodPost || gotPath != "/v1/browser/adapter/capability" {
		t.Fatalf("request = %s %s", gotMethod, gotPath)
	}
	if gotKey != "secret-key" || gotCT != "application/json" {
		t.Fatalf("headers key=%q content-type=%q", gotKey, gotCT)
	}
	if gotBody["actor"] != "agent" || gotBody["adapter_id"] != "tempo-os-jail-v1" {
		t.Fatalf("server received unexpected capability body: %+v", gotBody)
	}
	if !strings.Contains(string(raw), `"same_user_capability"`) || !strings.Contains(string(raw), `"ttl_seconds":60`) {
		t.Fatalf("capability response not surfaced: %s", raw)
	}
}

func TestRegisterBrowserAdapterMockServer(t *testing.T) {
	var gotMethod, gotPath, gotKey, gotCT string
	var gotBody map[string]any
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotMethod = r.Method
		gotPath = r.URL.Path
		gotKey = r.Header.Get(apiKeyHeader)
		gotCT = r.Header.Get("Content-Type")
		b, _ := io.ReadAll(r.Body)
		_ = json.Unmarshal(b, &gotBody)

		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		io.WriteString(w, `{
			"decision":"rejected",
			"adapter_id":"tempo-os-jail-v1",
			"actor":"agent",
			"sensitivity":"sensitive",
			"registered":false,
			"launchable":false,
			"trusted_for_sensitive_work":false,
			"endpoint_network_policy_bound":false,
			"same_user_capability_bound":false,
			"manifest_validation":{"decision":"rejected","manifest_complete":false,"launchable":false,"trusted_for_sensitive_work":false,"adapter_id":"tempo-os-jail-v1","launch_endpoint":"https://adapter.example/launch","endpoint_network_policy_bound":false,"missing_levels":[],"missing_controls":[],"missing_guard_fields":[],"missing_completion_proofs":[],"reasons":["validation metadata only"],"required_next_steps":["implement registration"],"adapter_contract":{"version":"browser-adapter-v1","status":"planned","launch_endpoint":null,"handoff_fields":["guard_plan"],"required_guard_fields":["guard_plan.network.deny_metadata_endpoints"],"required_completion_proofs":["temporary profile directory removed"],"unavailable_reason":"no browser adapter launch endpoint is implemented by this daemon"},"conformance_profile":{"profile_version":"browser-adapter-conformance-v1","field_complete_manifest":{"adapter_id":"tempo-conformance-adapter-v1","contract_version":"browser-adapter-v1","launch_endpoint":"https://adapter.example/launch","supported_levels":["os_isolated"],"supported_controls":["os_process_isolation"],"guard_fields":["guard_plan.network.deny_metadata_endpoints"],"completion_proofs":["temporary profile directory removed"]},"field_complete_expectation":{"decision":"rejected","manifest_complete":false,"launchable":false,"trusted_for_sensitive_work":false,"endpoint_network_policy_bound":false,"missing_levels":[],"missing_controls":[],"missing_guard_fields":[],"missing_completion_proofs":[]},"required_cases":[],"notes":["not a launch grant"]}},
			"reasons":["does not persist or trust adapters yet"],
			"required_next_steps":["issue a same-user capability"]
		}`)
	}))
	defer srv.Close()

	req := map[string]any{
		"actor":                "agent",
		"sensitivity":          "sensitive",
		"same_user_capability": "test-capability-fixture",
		"manifest": map[string]any{
			"adapter_id":       "tempo-os-jail-v1",
			"contract_version": "browser-adapter-v1",
			"launch_endpoint":  "https://adapter.example/launch",
		},
	}
	c := New(srv.URL, WithAPIKey("secret-key"))
	raw, err := c.RegisterBrowserAdapter(context.Background(), req)
	if err != nil {
		t.Fatalf("RegisterBrowserAdapter: %v", err)
	}
	if gotMethod != http.MethodPost || gotPath != "/v1/browser/adapter/register" {
		t.Fatalf("request = %s %s", gotMethod, gotPath)
	}
	if gotKey != "secret-key" || gotCT != "application/json" {
		t.Fatalf("headers key=%q content-type=%q", gotKey, gotCT)
	}
	if gotBody["same_user_capability"] != "test-capability-fixture" {
		t.Fatalf("server received unexpected registration body: %+v", gotBody)
	}
	if manifest, ok := gotBody["manifest"].(map[string]any); !ok || manifest["adapter_id"] != "tempo-os-jail-v1" {
		t.Fatalf("server received unexpected manifest: %+v", gotBody["manifest"])
	}
	if !strings.Contains(string(raw), `"registered":false`) || !strings.Contains(string(raw), `"same_user_capability_bound":false`) {
		t.Fatalf("registration response not surfaced: %s", raw)
	}
}

func TestPlanBrowserAdapterLaunchMockServer(t *testing.T) {
	var gotMethod, gotPath, gotKey, gotCT string
	var gotBody map[string]any
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotMethod = r.Method
		gotPath = r.URL.Path
		gotKey = r.Header.Get(apiKeyHeader)
		gotCT = r.Header.Get("Content-Type")
		b, _ := io.ReadAll(r.Body)
		_ = json.Unmarshal(b, &gotBody)

		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		io.WriteString(w, `{
			"decision":"rejected",
			"request_id":"bbx-browser-launch-plan-v1.fixture",
			"adapter_id":"tempo-os-jail-v1",
			"actor":"agent",
			"sensitivity":"sensitive",
			"launchable":false,
			"trusted_for_sensitive_work":false,
			"endpoint_network_policy_bound":false,
			"same_user_capability_bound":true,
			"launch_request":{"request_id":"bbx-browser-launch-plan-v1.fixture"},
			"completion_validation_endpoint":"/v1/browser/adapter/completion/validate"
		}`)
	}))
	defer srv.Close()

	req := map[string]any{
		"same_user_capability": "test-capability-fixture",
		"admission": map[string]any{
			"actor":       "agent",
			"sensitivity": "sensitive",
		},
		"manifest": map[string]any{
			"adapter_id": "tempo-os-jail-v1",
		},
	}
	c := New(srv.URL, WithAPIKey("secret-key"))
	raw, err := c.PlanBrowserAdapterLaunch(context.Background(), req)
	if err != nil {
		t.Fatalf("PlanBrowserAdapterLaunch: %v", err)
	}
	if gotMethod != http.MethodPost || gotPath != "/v1/browser/adapter/launch/plan" {
		t.Fatalf("request = %s %s", gotMethod, gotPath)
	}
	if gotKey != "secret-key" || gotCT != "application/json" {
		t.Fatalf("headers key=%q content-type=%q", gotKey, gotCT)
	}
	if gotBody["same_user_capability"] != "test-capability-fixture" {
		t.Fatalf("server received unexpected launch plan body: %+v", gotBody)
	}
	if manifest, ok := gotBody["manifest"].(map[string]any); !ok || manifest["adapter_id"] != "tempo-os-jail-v1" {
		t.Fatalf("server received unexpected manifest: %+v", gotBody["manifest"])
	}
	if !strings.Contains(string(raw), `"launchable":false`) || !strings.Contains(string(raw), `"same_user_capability_bound":true`) {
		t.Fatalf("launch plan response not surfaced: %s", raw)
	}
}

func TestClaimBrowserAdapterLaunchMockServer(t *testing.T) {
	var gotMethod, gotPath, gotKey, gotCT string
	var gotBody map[string]any
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotMethod = r.Method
		gotPath = r.URL.Path
		gotKey = r.Header.Get(apiKeyHeader)
		gotCT = r.Header.Get("Content-Type")
		b, _ := io.ReadAll(r.Body)
		_ = json.Unmarshal(b, &gotBody)

		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		io.WriteString(w, `{
			"decision":"claimed",
			"request_id":"bbx-browser-launch-plan-v1.fixture",
			"adapter_id":"tempo-os-jail-v1",
			"server_issued_launch_request":true,
			"canonical_request_matched":true,
			"launch_request_unexpired":true,
			"launch_request_claim_bound":true,
			"launch_request_replay_detected":false,
			"launchable":false,
			"trusted_for_sensitive_work":false,
			"endpoint_network_policy_bound":false
		}`)
	}))
	defer srv.Close()

	req := map[string]any{
		"launch_request": map[string]any{
			"request_id": "bbx-browser-launch-plan-v1.fixture",
			"adapter_id": "tempo-os-jail-v1",
		},
	}
	c := New(srv.URL, WithAPIKey("secret-key"))
	raw, err := c.ClaimBrowserAdapterLaunch(context.Background(), req)
	if err != nil {
		t.Fatalf("ClaimBrowserAdapterLaunch: %v", err)
	}
	if gotMethod != http.MethodPost || gotPath != "/v1/browser/adapter/launch/claim" {
		t.Fatalf("request = %s %s", gotMethod, gotPath)
	}
	if gotKey != "secret-key" || gotCT != "application/json" {
		t.Fatalf("headers key=%q content-type=%q", gotKey, gotCT)
	}
	if launchRequest, ok := gotBody["launch_request"].(map[string]any); !ok || launchRequest["adapter_id"] != "tempo-os-jail-v1" {
		t.Fatalf("server received unexpected claim body: %+v", gotBody)
	}
	if !strings.Contains(string(raw), `"decision":"claimed"`) || !strings.Contains(string(raw), `"launch_request_claim_bound":true`) {
		t.Fatalf("launch claim response not surfaced: %s", raw)
	}
}

func TestValidateBrowserAdapterMockServer(t *testing.T) {
	var gotMethod, gotPath, gotKey, gotCT string
	var gotBody map[string]any
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotMethod = r.Method
		gotPath = r.URL.Path
		gotKey = r.Header.Get(apiKeyHeader)
		gotCT = r.Header.Get("Content-Type")
		b, _ := io.ReadAll(r.Body)
		_ = json.Unmarshal(b, &gotBody)

		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		io.WriteString(w, `{
			"decision":"rejected",
			"manifest_complete":false,
			"launchable":false,
			"trusted_for_sensitive_work":false,
			"adapter_id":"tempo-os-jail-v1",
			"launch_endpoint":"https://adapter.example/launch",
			"endpoint_network_policy_bound":false,
			"missing_levels":[],
			"missing_controls":[],
			"missing_guard_fields":[],
			"missing_completion_proofs":[],
			"reasons":["no trusted adapter registration, endpoint binding, or launch path is implemented"],
			"required_next_steps":["implement authenticated adapter registration"],
			"adapter_contract":{"version":"browser-adapter-v1","status":"planned","launch_endpoint":null,"handoff_fields":["guard_plan"],"required_guard_fields":["guard_plan.network.deny_metadata_endpoints"],"required_completion_proofs":["temporary profile directory removed"],"unavailable_reason":"no browser adapter launch endpoint is implemented by this daemon"},
			"conformance_profile":{"profile_version":"browser-adapter-conformance-v1","field_complete_manifest":{"adapter_id":"tempo-conformance-adapter-v1","contract_version":"browser-adapter-v1","launch_endpoint":"https://adapter.example/launch","supported_levels":["os_isolated"],"supported_controls":["os_process_isolation"],"guard_fields":["guard_plan.network.deny_metadata_endpoints"],"completion_proofs":["temporary profile directory removed"]},"field_complete_expectation":{"decision":"rejected","manifest_complete":false,"launchable":false,"trusted_for_sensitive_work":false,"endpoint_network_policy_bound":false,"missing_levels":[],"missing_controls":[],"missing_guard_fields":[],"missing_completion_proofs":[]},"required_cases":[{"name":"dns_rebinding_hostname_stays_incomplete","expected_rest_status":200,"expected_rest_error_code":null,"expected_mcp_error_code":null,"expected_mcp_error_message_contains":[],"expected_validation":{"decision":"rejected","manifest_complete":false,"launchable":false,"trusted_for_sensitive_work":false,"endpoint_network_policy_bound":false,"missing_levels":[],"missing_controls":[],"missing_guard_fields":[],"missing_completion_proofs":[]}}],"notes":["not a launch grant"]}
		}`)
	}))
	defer srv.Close()

	c := New(srv.URL, WithAPIKey("secret-key"))
	raw, err := c.ValidateBrowserAdapter(context.Background(), map[string]any{
		"adapter_id":       "tempo-os-jail-v1",
		"contract_version": "browser-adapter-v1",
		"launch_endpoint":  "https://adapter.example/launch",
	})
	if err != nil {
		t.Fatalf("ValidateBrowserAdapter: %v", err)
	}
	if gotMethod != http.MethodPost || gotPath != "/v1/browser/adapter/validate" {
		t.Fatalf("request = %s %s", gotMethod, gotPath)
	}
	if gotKey != "secret-key" || gotCT != "application/json" {
		t.Fatalf("headers key=%q content-type=%q", gotKey, gotCT)
	}
	if gotBody["adapter_id"] != "tempo-os-jail-v1" {
		t.Fatalf("server received unexpected adapter body: %+v", gotBody)
	}
	if !strings.Contains(string(raw), `"manifest_complete":false`) || !strings.Contains(string(raw), `"launchable":false`) || !strings.Contains(string(raw), `"browser-adapter-conformance-v1"`) {
		t.Fatalf("validation response not surfaced: %s", raw)
	}
}

func TestValidateBrowserAdapterCompletionMockServer(t *testing.T) {
	var gotMethod, gotPath, gotKey, gotCT string
	var gotBody map[string]any
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotMethod = r.Method
		gotPath = r.URL.Path
		gotKey = r.Header.Get(apiKeyHeader)
		gotCT = r.Header.Get("Content-Type")
		b, _ := io.ReadAll(r.Body)
		_ = json.Unmarshal(b, &gotBody)

		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		io.WriteString(w, `{
			"decision":"rejected",
			"report_shape_complete":true,
			"verified_on_production_path":false,
			"trusted_for_sensitive_work":false,
			"request_id":"browser-adapter-conformance-launch-v1",
			"adapter_id":"tempo-conformance-adapter-v1",
			"contract_version":"browser-adapter-v1",
			"missing_proof_ids":[],
			"unexpected_proof_ids":[],
			"failed_evidence_fields":[],
			"required_completion_proofs":["temporary profile directory removed"],
			"completion_proof_contract":[],
			"reasons":["shape only"],
			"required_next_steps":["verify production teardown"],
			"adapter_contract":{"version":"browser-adapter-v1","status":"planned","launch_endpoint":null,"handoff_fields":["completion_report_template"],"required_guard_fields":[],"required_completion_proofs":["temporary profile directory removed"],"completion_proof_contract":[],"unavailable_reason":"no browser adapter launch endpoint is implemented by this daemon"}
		}`)
	}))
	defer srv.Close()

	c := New(srv.URL, WithAPIKey("secret-key"))
	raw, err := c.ValidateBrowserAdapterCompletion(context.Background(), map[string]any{
		"request_id":                     "browser-adapter-conformance-launch-v1",
		"adapter_id":                     "tempo-conformance-adapter-v1",
		"contract_version":               "browser-adapter-v1",
		"process_terminated":             true,
		"temporary_profile_removed":      true,
		"plaintext_artifacts_removed":    true,
		"egress_log_sealed_or_discarded": true,
		"sealed_artifact_handles":        []any{},
		"proof_ids":                      []any{"temporary_profile_removed"},
		"notes":                          []any{"shape fixture only"},
	})
	if err != nil {
		t.Fatalf("ValidateBrowserAdapterCompletion: %v", err)
	}
	if gotMethod != http.MethodPost || gotPath != "/v1/browser/adapter/completion/validate" {
		t.Fatalf("request = %s %s", gotMethod, gotPath)
	}
	if gotKey != "secret-key" || gotCT != "application/json" {
		t.Fatalf("headers key=%q content-type=%q", gotKey, gotCT)
	}
	if gotBody["request_id"] != "browser-adapter-conformance-launch-v1" || gotBody["adapter_id"] != "tempo-conformance-adapter-v1" {
		t.Fatalf("server received unexpected completion body: %+v", gotBody)
	}
	if !strings.Contains(string(raw), `"report_shape_complete":true`) || !strings.Contains(string(raw), `"verified_on_production_path":false`) {
		t.Fatalf("completion validation response not surfaced: %s", raw)
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

func TestCustomHTTPClientRedirectPolicyIsOverridden(t *testing.T) {
	var targetSawKey bool
	target := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		targetSawKey = r.Header.Get(apiKeyHeader) != ""
		io.WriteString(w, `{"ok":true}`)
	}))
	defer target.Close()

	origin := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, target.URL+"/collect", http.StatusFound)
	}))
	defer origin.Close()

	allowRedirects := &http.Client{
		CheckRedirect: func(*http.Request, []*http.Request) error {
			return nil
		},
		Timeout: 2 * time.Second,
	}
	c := New(origin.URL, WithAPIKey("k"), WithHTTPClient(allowRedirects))
	_, err := c.Capabilities(context.Background())
	var apiErr *APIError
	if !errors.As(err, &apiErr) {
		t.Fatalf("expected *APIError from unfollowed redirect, got %v", err)
	}
	if apiErr.Status != http.StatusFound {
		t.Errorf("status = %d, want 302", apiErr.Status)
	}
	if targetSawKey {
		t.Fatal("redirect target received api key")
	}
}
