# beatbox — TypeScript SDK

Zero-dependency TypeScript SDK for the [beatbox](https://github.com/jadenfix/beatbox)
sandbox REST API. Built on the platform-global `fetch` / `AbortController`
(Node 18+, Node 22 recommended) — no `axios`, no `node-fetch`, no code-gen
runtime.

## Install

```sh
npm install beatbox
```

Then build the SDK from source (there is no bundled `dist/` in the repo):

```sh
npm run build
```

## Quickstart

Run a `wasm_wat` "add one" program and print the result value:

```ts
import { BeatboxClient, ExecuteRequest } from "beatbox";

const client = new BeatboxClient({
  baseUrl: "http://127.0.0.1:7300",
  apiKey: process.env.BEATBOX_API_KEY, // optional
});

const result = await client.execute(
  ExecuteRequest.wasmWat(
    '(module (func (export "run") (param i64) (result i64) local.get 0 i64.const 1 i64.add))',
    { input: { n: 41 } },
  ),
);

console.log(result.value); // 42
console.log(result.value === 42); // true
```

## Configuration

```ts
new BeatboxClient({
  baseUrl: "http://127.0.0.1:7300", // required; trailing slashes are trimmed
  apiKey: "sk-...",                  // optional
  timeoutMs: 65000,                  // optional; default 65s (AbortController)
});
```

When `apiKey` is set it is sent as the header `x-beatbox-api-key` on every
request **except** `health()` and `openapi()`, which are unauthenticated.
Redirects are never followed, so the key can't leak cross-origin. The API key
is never included in any error message or thrown object.

## Methods

| Method | HTTP | Auth | Returns |
| --- | --- | --- | --- |
| `health()` | `GET /v1/health` | no | `HealthResponse` |
| `capabilities()` | `GET /v1/capabilities` | yes | raw JSON (`unknown`) |
| `execute(req)` | `POST /v1/execute` | yes | `ExecutionResult` |
| `createJob(req)` | `POST /v1/jobs` | yes | `CreateJobResponse` (202) |
| `getJob(id)` | `GET /v1/jobs/{id}` | yes | `JobRecord` |
| `cancelJob(id)` | `DELETE /v1/jobs/{id}` | yes | `void` (204) |
| `openapi()` | `GET /openapi.json` | no | raw JSON (`unknown`) |

Job ids are percent-encoded as a single path segment; `''`, `'.'` and `'..'`
are rejected (they could retarget the request).

## Building requests

`Source` is a tagged union on `kind` with a constructor per variant:

```ts
import { Source, ExecuteRequest } from "beatbox";

Source.inline("print(1)");
Source.wasmFile("/path/to/m.wasm");
Source.wasmWat("(module ...)");
Source.wasmBytesBase64("AGFzbQ==");
Source.moduleRef("sha256hex");
```

Ergonomic `ExecuteRequest` constructors cover the common cases:

```ts
ExecuteRequest.wasmWat("(module ...)", { input: { n: 41 } });
ExecuteRequest.wasmBytesBase64("AGFzbQ==", { entrypoint: "run" });
ExecuteRequest.inline("python_wasi", "print(1)");
ExecuteRequest.of("wasm", Source.moduleRef("..."), {
  policy: { limits: { wall_ms: 5000, fuel: 10_000_000 } },
  idempotencyKey: "step-1",
});
```

## Async jobs

```ts
const { job_id } = await client.createJob(
  ExecuteRequest.wasmWat("(module ...)", { input: { n: 41 } }),
);
const job = await client.getJob(job_id);
if (job.status === "succeeded") console.log(job.result?.value);
await client.cancelJob(job_id);
```

## Error handling

```ts
import { BeatboxApiError, BeatboxTransportError } from "beatbox";

try {
  await client.execute(req);
} catch (err) {
  if (err instanceof BeatboxApiError) {
    // Non-2xx from the daemon.
    console.error(err.status, err.code, err.message);
  } else if (err instanceof BeatboxTransportError) {
    // Network failure, timeout/abort, or an unexpected redirect.
    console.error(err.message, err.cause);
  } else {
    throw err;
  }
}
```

`BeatboxApiError` carries the HTTP `status`, the machine-readable `code` from
the `{ error: { code, message } }` body, and the human `message`. Both error
types extend `BeatboxError`. Neither ever contains the API key.

## Example

[`examples/fib.ts`](./examples/fib.ts) runs `fib(10)` on the wasm lane and
asserts `result.value === 55`. Against a running daemon:

```sh
npm run example   # tsc build + node dist/examples/fib.js
```

## Development

```sh
npm run typecheck   # tsc --noEmit
npm run build       # emit dist/
npm test            # tsc build + node --test dist/test/
```

## License

Apache-2.0
