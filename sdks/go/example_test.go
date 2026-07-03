package beatbox_test

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"

	beatbox "github.com/jadenfix/beatbox/sdks/go"
)

// ExampleClient_Execute runs an "add one" wasm module and reads the result
// value. A mock server stands in for the daemon so the example is self-checking.
func ExampleClient_Execute() {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		io.WriteString(w, `{
			"status":"ok","value":42,
			"stdout":"","stdout_truncated":false,
			"stderr":"","stderr_truncated":false,
			"metrics":{"wall_time_ms":1,"cpu_time_ms":null,"fuel_used":null,"peak_memory_bytes":null},
			"lane":"wasm","deterministic":true,"inputs_digest":"d",
			"engine_version":"w0","beatbox_version":"0.1.0",
			"effective_isolation":{"os":"linux","mechanisms":[],"downgrades":[]},
			"egress":[]
		}`)
	}))
	defer srv.Close()

	client := beatbox.New(srv.URL, beatbox.WithAPIKey("BEATBOX_API_KEY"))

	res, err := client.Execute(context.Background(), beatbox.WasmWatRequest(
		`(module (func (export "run") (param i64) (result i64)
			local.get 0 i64.const 1 i64.add))`,
		map[string]any{"n": 41}))
	if err != nil {
		fmt.Println("execute failed:", err)
		return
	}

	var value int
	if err := json.Unmarshal(res.Value, &value); err != nil {
		fmt.Println("decode value:", err)
		return
	}
	fmt.Println("status:", res.Status)
	fmt.Println("value:", value)
	// Output:
	// status: ok
	// value: 42
}
