# Cradle

Cradle is the ecosystem sandbox execution service currently shipped through the
`beatbox` crates and binaries. It runs untrusted, agent-generated code behind
explicit capabilities, with a standalone CLI, daemon, REST API, MCP endpoint,
OpenAPI document, and client SDKs.

The runnable path today is the hermetic Wasmtime `wasm` lane. Python,
JavaScript, native exec, stateful sessions, and browser automation are exposed
only as fail-closed contracts until their isolation, network, storage, and
teardown paths are implemented.

## Quickstart

Run the local CLI against the Wasm lane:

```sh
cargo run -p beatbox -- run examples/fib.wasm --input '{"n":10}'
```

Start the daemon:

```sh
cargo run -p beatboxd -- --addr 127.0.0.1:7300
```

Call the REST API with the example request body. This computes `fib(10) = 55`:

```sh
curl -sS http://127.0.0.1:7300/v1/execute \
  -H 'content-type: application/json' \
  -d @examples/req-fib.json
```

Run tests:

```sh
cargo test
```

Remote requests upload modules as inline WAT or base64 Wasm bytes. Daemon-local
`wasm_file` paths are intentionally CLI-only and rejected by the REST API.
Limits may be partial, with unspecified fields falling back to defaults, and
unknown policy keys are rejected rather than ignored.

## Status

| Surface | Status | Notes |
| --- | --- | --- |
| Wasm execution | Runnable | Wasmtime, empty linker, fuel, epoch interruption, store limits |
| REST API | Runnable | `/v1/execute`, `/v1/jobs`, `/v1/capabilities`, `/v1/integration` |
| MCP | Runnable for Wasm | `run_wasm`, `get_capabilities`, `get_integration_contract` |
| SDKs | Runnable for core API | Generated from the committed OpenAPI contract plus hand-written clients |
| Python / JavaScript lanes | Planned | Request shape exists; execution remains fail-closed |
| Native exec lanes | Planned | Requires real OS jail policy before it can be marked runnable |
| Browser automation | Planned | Discovery, admission, adapter, and proof contracts exist; no browser is launched |

## Integration

Cradle is standalone by design. Sibling projects should integrate over protocol
boundaries, not by linking to internal crates.

- `GET /v1/health` is unauthenticated liveness.
- `GET /openapi.json` is the generated API contract.
- `GET /v1/capabilities` reports lanes, limits, engines, browser posture, and
  the ecosystem integration summary.
- `GET /v1/integration` returns the focused ecosystem contract: runnable lanes,
  planned fail-closed lanes, auth posture, REST/MCP surfaces, SDK methods, and
  intended Tempo/beater.js/beaterOS boundaries.
- MCP exposes the same integration contract through
  `get_integration_contract`.
- MCP `tools/list` descriptors are fixture-pinned at
  [`crates/beatbox-server/fixtures/mcp-tools.catalog.json`](crates/beatbox-server/fixtures/mcp-tools.catalog.json)
  and checked by `cargo test -p beatbox-server --test mcp_catalog_drift`.
  Update the fixture in the same PR as any MCP tool name, description, or input
  schema change.

Authenticated routes accept `Authorization: Bearer <token>`. The daemon also
keeps `x-beatbox-api-key` as a compatibility header. Tokens
must never be placed in URLs or echoed in errors, logs, or MCP content.

Browser automation is intentionally non-runnable. Callers can inspect
`GET /v1/browser/profiles`, `POST /v1/browser/admit`,
`GET /v1/browser/adapter/contract`, and the matching MCP tools to validate
future Tempo-style adapters, but responses keep `launchable: false` and
`trusted_for_sensitive_work: false` until a production browser launcher,
endpoint network policy, storage policy, and teardown proof path exist. See
[ARCHITECTURE.md](./ARCHITECTURE.md) and [SECURITY.md](./SECURITY.md) for the
full fail-closed browser contract.

## Ecosystem

Cradle is part of the
[ecosystem](https://github.com/jadenfix/ecosystem), a family of Rust-first,
local-first agent-infrastructure projects. Planned connection points include
sandboxed execution for
[beater.js](https://github.com/jadenfix/beater.js), tool execution for
[tempo](https://github.com/jadenfix/tempo), and auditable side-effect records
for [beaterOS](https://github.com/jadenfix/beaterOS).
