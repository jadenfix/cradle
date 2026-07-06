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

The `run --remote` upload path and `compile` helper read local Wasm/WAT inputs
through the same capped source reader used by the engine.

Start the daemon:

```sh
BEATBOX_API_KEY=dev-secret cargo run -p beatboxd -- --addr 127.0.0.1:7300
```

The unauthenticated daemon override is accepted only for loopback bind
addresses and is intended for isolated local tests. When an API key is required,
MCP clients must authenticate before `GET /mcp`, `initialize`, `ping`, tool
listing, or tool calls receive protocol responses. Empty or whitespace-only API
keys are rejected instead of becoming an empty shared secret.
Control-plane routes also reject duplicate or malformed `Host`, `Origin`, and
`Content-Type` boundary headers before request parsing.

Remote API keys are sent only to HTTPS origins by default. The CLI uses an
explicit loopback-HTTP opt-in for local loopback IP daemons such as
`127.0.0.1` or `[::1]`; it still refuses to send a key to hostnames or
non-loopback `http://` origins. Beatbox clients also ignore ambient proxy
configuration so key-bearing control-plane traffic does not silently route
through a proxy process. Client base URLs must not include query strings or
fragments, and dynamic path segments such as job ids are encoded before the API
key is attached. Successful client responses must advertise a JSON media type,
empty-success client operations require the documented no-content status, and
response bodies are read through a configurable byte cap instead of being
buffered without bound.

Run tests:

```sh
cargo test
```

Run the daemon/network e2e gate:

```sh
bash scripts/e2e-daemon.sh
```

The current implementation covers the first standalone path: core wire types, a
Wasmtime-backed `wasm` lane with fuel, wall-clock, and one aggregate store
budget for linear memory and table elements, plus a fixed `max_wasm_module_bytes`
cap for source/module bytes before compilation. It also includes a local CLI, a
daemon router, a typed HTTP client, OpenAPI JSON, MCP tools, and
rusqlite-backed async jobs. The production-grade direction is Rust host code
plus Wasmtime/WASI substrates; native CPython is available only as an
explicitly compiled macOS dev-grade lane behind `lane-python`, with honest
downgrades for unenforced CPU and memory limits. It enforces wall time, output,
a polling private-workspace disk quota, and a fixed inline source cap advertised
as `max_python_source_bytes`, ties source delivery to the wall/cancel watchdog,
and rejects non-default CPU-time, process-memory, pid, and fuel limits instead of
pretending to enforce them. The selected interpreter must resolve to a regular
file in a known Homebrew or Command Line Tools Python runtime path; arbitrary
executables merely under `/usr/local` or `/opt/homebrew` are not trusted. The
Seatbelt profile also denies broad Mach service lookup and broad sysctl reads.
The daemon accepts
remote execution only for lanes reported available by `/v1/capabilities`;
planned lanes such as JS and generic exec are rejected before worker execution
or job queueing. REST request JSON rejects unknown fields, matching the closed
OpenAPI request schemas, while omitted policy and nested limit fields are
filled from the same runtime defaults reported by `/v1/capabilities`. Async job
records are capped by `max_stored_jobs` in `/v1/capabilities` and can be
configured for `beatboxd` with `--max-stored-jobs` or
`BEATBOX_MAX_STORED_JOBS`; idempotent retries reuse the existing record instead
of consuming quota. The daemon creates the SQLite
job store as private local state, and the shared job-store open path rejects
symlink, non-regular, and SQLite URI DB paths while opening literal files with
SQLite no-follow semantics and hardening SQLite sidecars. JS, exec jails,
stateful sessions, and the `beater.js` integration remain later milestones.
