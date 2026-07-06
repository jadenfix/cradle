# beatbox

`beatbox` is a standalone sandbox service for running untrusted,
agent-generated code behind explicit capabilities. It is designed to run on its
own through a CLI, daemon, REST API, and MCP endpoint, then plug into sibling
Beater projects over protocol boundaries.

## Quickstart

Run the local CLI against a hermetic Wasmtime lane:

```sh
cargo run -p beatbox -- run examples/fib.wasm --input '{"n":10}'
```

Start the daemon:

```sh
cargo run -p beatboxd -- --addr 127.0.0.1:7300
```

Call the REST API with the example request body (computes `fib(10) = 55`):

```sh
curl -sS http://127.0.0.1:7300/v1/execute \
  -H 'content-type: application/json' \
  -d @examples/req-fib.json
```

Remote requests upload the module as inline WAT or base64 Wasm bytes;
daemon-local `wasm_file` paths are rejected by design. Limits may be given
partially (unspecified fields fall back to defaults), and unknown policy keys
are rejected rather than silently ignored.

Run tests:

```sh
cargo test
```

The current implementation covers the first standalone path: core wire types, a
Wasmtime-backed `wasm` lane, a local CLI, a daemon router, a typed HTTP client,
OpenAPI JSON, MCP tools, and rusqlite-backed async jobs. Native Python, JS,
exec jails, stateful sessions, and the `beater.js` integration remain later
milestones.

Browser automation is exposed only as an explicit discovery contract today, not
as a runnable browser lane. Authenticated callers can read
`GET /v1/browser/profiles`, call the MCP `get_browser_profiles` tool, or inspect
the `browser_sandbox` section of `/v1/capabilities` to see the profile levels
Beatbox intends to support for Tempo-style integrations: external
instrumentation, ephemeral profiles, network-suppressed browsing, sealed
persisted state, OS-isolated browsing, and remote isolated workers. The response
deliberately reports
`runnable_browser_sessions: false`, has no default level, and marks every
profile as `planned` or `unavailable` until a real browser launcher, egress
boundary, storage policy, and teardown path enforce the claim.
