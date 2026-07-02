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

Run tests:

```sh
cargo test
```

The current implementation covers the first standalone path: core wire types, a
Wasmtime-backed `wasm` lane, a local CLI, a daemon router, a typed HTTP client,
OpenAPI JSON, MCP tools, and rusqlite-backed async jobs. Native Python, JS,
exec jails, stateful sessions, and the `beater.js` integration remain later
milestones.
