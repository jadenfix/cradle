# beatbox Go SDK

A zero-dependency Go client for the [beatbox](https://github.com/jadenfix/beatbox)
sandbox REST API. Standard library only (`net/http` + `encoding/json`), Go 1.21+.

- License: Apache-2.0
- Package: `beatbox`
- Version: 0.1.0

## Install

```sh
go get github.com/jadenfix/beatbox/sdks/go
```

```go
import beatbox "github.com/jadenfix/beatbox/sdks/go"
```

## Quickstart

Run a `wasm_wat` "add one" module and assert the result is `42`:

```go
package main

import (
	"context"
	"encoding/json"
	"log"
	"os"

	beatbox "github.com/jadenfix/beatbox/sdks/go"
)

func main() {
	client := beatbox.New("http://127.0.0.1:7300",
		beatbox.WithAPIKey(os.Getenv("BEATBOX_API_KEY")))

	res, err := client.Execute(context.Background(), beatbox.WasmWatRequest(
		`(module (func (export "run") (param i64) (result i64)
			local.get 0 i64.const 1 i64.add))`,
		map[string]any{"n": 41}))
	if err != nil {
		log.Fatal(err)
	}

	var value int
	if err := json.Unmarshal(res.Value, &value); err != nil {
		log.Fatal(err)
	}
	if value != 42 {
		log.Fatalf("value = %d, want 42", value)
	}
	log.Printf("status=%s value=%d", res.Status, value) // status=ok value=42
}
```

A runnable version lives in [`examples/addone`](./examples/addone):

```sh
BEATBOX_API_KEY=... go run ./examples/addone
```

## Configuration

Construct a `Client` with `New(baseURL, opts...)`. Trailing slashes in the base
URL are trimmed.

| Option | Purpose |
| --- | --- |
| `WithAPIKey(key)` | Sent as the `x-beatbox-api-key` header on every request except `Health` and `OpenAPI`. |
| `WithTimeout(d)` | Per-request timeout (default 65s). |
| `WithHTTPClient(hc)` | Supply your own `*http.Client`. |

The client never follows redirects, so the API key can't leak to another host,
and the key is never placed in a URL or error message.

## Methods

All methods are context-first.

| Method | HTTP | Auth | Returns |
| --- | --- | --- | --- |
| `Health(ctx)` | `GET /v1/health` | no | `json.RawMessage` |
| `Capabilities(ctx)` | `GET /v1/capabilities` | yes | `json.RawMessage` |
| `BrowserProfiles(ctx)` | `GET /v1/browser/profiles` | yes | `json.RawMessage` |
| `AdmitBrowserSession(ctx, req)` | `POST /v1/browser/admit` | yes | `json.RawMessage` |
| `BrowserAdapterContract(ctx)` | `GET /v1/browser/adapter/contract` | yes | `json.RawMessage` |
| `ValidateBrowserAdapter(ctx, req)` | `POST /v1/browser/adapter/validate` | yes | `json.RawMessage` |
| `Execute(ctx, req)` | `POST /v1/execute` | yes | `*ExecutionResult` |
| `CreateJob(ctx, req)` | `POST /v1/jobs` | yes | `*CreateJobResponse` |
| `GetJob(ctx, id)` | `GET /v1/jobs/{id}` | yes | `*JobRecord` |
| `CancelJob(ctx, id)` | `DELETE /v1/jobs/{id}` | yes | `error` |
| `OpenAPI(ctx)` | `GET /openapi.json` | no | `json.RawMessage` |

Job ids are percent-encoded as a single path segment; `""`, `"."` and `".."`
are rejected before any request is sent.

## Building requests

`ExecuteRequest` requires a `Lane` and a `Source`. Use a source constructor and,
for the common case, `WasmWatRequest`:

```go
beatbox.WasmWatRequest(wat, input)              // lane=wasm, source=wasm_wat
```

Source variants:

```go
beatbox.SourceInline(code)
beatbox.SourceWasmFile(path)
beatbox.SourceWasmWat(text)
beatbox.SourceWasmBytesBase64(rawBytes) // base64-encodes for the wire
beatbox.SourceModuleRef(sha256)
```

Policies are partial — only the fields you set are sent (pointers + `omitempty`),
and the daemon merges them onto its defaults:

```go
wall := uint64(5000)
req := beatbox.ExecuteRequest{
	Lane:   beatbox.LaneWasm,
	Source: beatbox.SourceWasmWat(wat),
	Policy: &beatbox.Policy{Limits: &beatbox.Limits{WallMs: &wall}},
}
```

## Error handling

Non-2xx responses return a typed `*APIError` carrying the HTTP `Status`, the
`Code` and `Message` from the `{"error": {...}}` body. Transport failures are
wrapped errors.

```go
res, err := client.Execute(ctx, req)
if err != nil {
	var apiErr *beatbox.APIError
	if errors.As(err, &apiErr) {
		log.Printf("status=%d code=%s: %s", apiErr.Status, apiErr.Code, apiErr.Message)
	} else {
		log.Printf("transport error: %v", err)
	}
	return
}
```

## Development

```sh
go vet ./...
go test ./...
```

Tests are hermetic (they use `net/http/httptest`) and need no running daemon.
