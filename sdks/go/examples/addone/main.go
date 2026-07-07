// Command addone runs a wasm "add one" module against a live beatbox daemon and
// asserts that the result value is 42.
//
// Usage:
//
//	CRADLE_TOKEN=... go run ./examples/addone            # http://127.0.0.1:7300
//	CRADLE_BASE_URL=http://host:7300 go run ./examples/addone
package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"os"
	"time"

	beatbox "github.com/jadenfix/beatbox/sdks/go"
)

// addOneWAT exports run(i64) -> i64 that returns its argument plus one.
const addOneWAT = `(module
  (func (export "run") (param i64) (result i64)
    local.get 0
    i64.const 1
    i64.add))`

func main() {
	baseURL := os.Getenv("CRADLE_BASE_URL")
	if baseURL == "" {
		baseURL = "http://127.0.0.1:7300"
	}

	client := beatbox.New(baseURL,
		beatbox.WithToken(os.Getenv("CRADLE_TOKEN")),
		beatbox.WithTimeout(30*time.Second),
	)

	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	res, err := client.Execute(ctx, beatbox.WasmWatRequest(addOneWAT, map[string]any{"n": 41}))
	if err != nil {
		var apiErr *beatbox.APIError
		if errors.As(err, &apiErr) {
			log.Fatalf("beatbox rejected the request: status=%d code=%s message=%s",
				apiErr.Status, apiErr.Code, apiErr.Message)
		}
		log.Fatalf("execute failed: %v", err)
	}

	if res.Status != beatbox.ExecutionStatusOK {
		log.Fatalf("unexpected status %q: %+v", res.Status, res.Error)
	}

	var value int
	if err := json.Unmarshal(res.Value, &value); err != nil {
		log.Fatalf("decode value: %v", err)
	}

	fmt.Printf("status=%s value=%d wall_time_ms=%d\n", res.Status, value, res.Metrics.WallTimeMs)
	if value != 42 {
		log.Fatalf("assertion failed: value = %d, want 42", value)
	}
	fmt.Println("OK: value == 42")
}
