# beatbox architecture

`beatbox` is a secure agent sandbox. Its purpose is to run untrusted,
agent-generated code without giving that code ambient filesystem, network,
process, or environment access.

## role in the Beater ecosystem

The repository is standalone first. It exposes a CLI, `beatboxd` daemon, REST
API, and MCP endpoint that can be used without `beater.js` or `beater-agents`
present. Integrations with those siblings are protocol integrations, not source
coupling.

The one planned source-level exception is `beatbox-client`, a tiny typed client
over the HTTP API that re-exports `beatbox-core` wire types. It is intended to
be published as a normal crate when the API stabilizes.

## workspace

| path | responsibility |
| --- | --- |
| `crates/beatbox-core` | serde wire contract: `Policy`, `ExecuteRequest`, `ExecutionResult`, `Lane`, and shared error bodies. |
| `crates/beatbox-engine` | isolation lanes and policy admission checks. The initial lane is Wasmtime with an empty linker, fuel, epoch interruption, and store limits. |
| `crates/beatbox-server` | `axum` router for `/v1`, `/openapi.json`, and `/mcp`, plus auth and a rusqlite-backed job store. |
| `crates/beatbox-client` | near-zero-abstraction `reqwest` client for `/v1` with safe URL construction, no ambient proxy use, disabled redirects, and capped response reads. |
| `bins/beatbox` | local CLI. It can execute directly in-process or call a remote `beatboxd`. |
| `bins/beatboxd` | daemon wrapper around `beatbox-server`. |

## isolation model

Two substrates are planned:

1. In-process Wasmtime for `wasm`, `python-wasi`, and `js-wasm`.
2. OS jails for native Python, native JS, and generic exec.

The initial implementation covers the `wasm` lane for core Wasm modules. It
rejects imports from the module bytes before Wasmtime compilation and again
through an empty linker, consumes fuel, interrupts long wall-clock runs with
Wasmtime epoch deadlines, rejects oversized Wasm sources against the advertised
`max_wasm_module_bytes` cap before parsing or compilation, reads local source
files through a capped reader, and enforces one aggregate `StoreLimits` budget
for linear memory plus table elements. It records the actual mechanisms in
every `ExecutionResult`.
This lane accepts JSON `input` for the small supported entrypoint ABI but rejects
`stdin` until WASI command/component stdin/stdout support is added.

Native CPython is not the production isolation strategy. When compiled with
`lane-python` on macOS, beatbox can run a CPython subprocess under a Seatbelt
profile through `sandbox-exec`; this is a dev-grade compatibility lane for
local evaluation. The selected interpreter must resolve to a regular file under
known Homebrew or Command Line Tools Python runtime paths; arbitrary executables
merely under `/usr/local` or `/opt/homebrew` are not trusted. The profile allows
only Python runtime reads plus a randomly named, exclusively created private
workspace, denies broad Mach service lookup and broad sysctl reads, and reports
downgrades for unenforced memory and CPU quotas. It enforces wall time, output
bytes, a polling private-workspace disk quota, and a fixed inline source cap
advertised as `max_python_source_bytes`; source delivery to the child is tied to
the wall/cancel watchdog so partial stdin delivery is reported as execution
failure. Non-default CPU-time, process-memory, pid, and fuel limits are rejected
because the lane cannot enforce them as resource ceilings. It is not a
replacement for Wasmtime/WASI or future microVM-backed execution. Until a
structured Python ABI lands, this lane
runs inline source only and rejects `entrypoint`, structured `input`, and
`stdin` request fields instead of silently ignoring them.

## policy contract

Every execution receives one `Policy`. Lanes must reject policies they cannot
enforce when the unsupported field would widen exposure. Safer-by-absence
behavior, such as no process spawning in an in-process Wasmtime lane, is
reported as enforced by construction. Remote REST and MCP submissions run the
lane policy admission checks before acquiring worker permits, queueing jobs, or
persisting job requests. REST request structs deny unknown fields and OpenAPI
request components advertise that closed shape, matching the stricter MCP tool
argument parser. Omitted policy and nested limit fields are default-filled from
the same `Policy::default()` values reported by `/v1/capabilities`, so compact
requests remain compatible without accepting misspelled fields.

The daemon also gates remote submissions by lane availability. `/v1/execute`,
`/v1/jobs`, and MCP tool calls reject unavailable or planned lanes before
worker execution or job queueing; callers should treat `/v1/capabilities` as
the source of truth for currently admitted lanes. `/v1/capabilities` and
`/openapi.json` are authenticated control-plane metadata; `/v1/health` remains
the intentionally public static liveness check. When authentication is enabled,
MCP requests, including `GET /mcp`, `initialize`, and `ping`, are authenticated
before body parsing or method dispatch. REST and MCP control-plane routes also
reject malformed or non-local `Host`, absolute request-target authority, and
browser `Origin` values before parsing or dispatch. Header fields that affect
parsing, auth, or the control-plane boundary must be singular; duplicated
`Host`, `Origin`, `Content-Type`, `x-beatbox-api-key`, or `Authorization`
headers are rejected as ambiguous. Empty or whitespace-only API keys fail closed
even when callers build `ServerConfig` or `beatbox-client` values directly.

No lane should inherit host environment variables or raw network access. Future
egress will be routed through a logging localhost proxy with domain and port
allowlists.

Synchronous REST and MCP executions keep their concurrency permit inside the
blocking worker, not just in the request future. If a client disconnects or the
request task is dropped, the server signals the engine cancellation token and
does not release the permit until the blocking execution has actually exited.

## milestones

M0: workspace scaffold, toolchain pin, core serde types, tests, and CI.

M1: WASI/Wasmtime lane through the CLI with fuel, wall-clock, memory, and output
limits plus escape regression tests.

M2: `beatboxd` REST/MCP API, auth, OpenAPI, and job persistence.

Asynchronous job idempotency trims the idempotency key and excludes the key's
original spelling from payload comparison. Retrying the same request with the
same normalized key reuses the stored job; changing any execution-affecting
request field returns a conflict.

Durable job recovery is fail-closed. When the daemon opens a persistent job
store, any non-terminal jobs left `queued` or `running` by a previous process
are marked `failed` with a `daemon_restart` error instead of remaining live
without an attached worker.

Async job storage has a daemon-level record cap independent of request-size and
worker concurrency limits. New `/v1/jobs` requests are rejected when the cap is
full, while idempotent retries that reuse an existing normalized key remain
available because they do not grow durable state. The daemon prepares the
SQLite job store as private local state: new parent directories are created
with private permissions, DB files and SQLite sidecars are chmodded private on
Unix, and symlink, non-regular, or SQLite URI DB paths are rejected before
SQLite opens them. The shared `JobStore::open` path uses literal filesystem
paths with SQLite no-follow semantics and creates or hardens the primary DB
file and SQLite sidecars as private local state so embedders get the same
symlink/URI boundary.

Current job cancellation is active for the Wasmtime lane and the optional
macOS native Python lane: `DELETE /v1/jobs/{id}` marks a queued or running
record as canceled and signals the running engine cancellation token so
execution releases its worker permit. Repeating cancellation for an already
canceled job is idempotent; trying to cancel a succeeded or failed terminal job
returns a conflict instead of reporting a cancellation that did not happen.
Future lanes must add equivalent per-process interruption before they can claim
the same cancellation behavior.

M3: `beater.js` Tier-4 integration through `beatbox-client`.

M4: WASI Python, JavaScript lanes, stronger native OS jails, and honest per-OS
capability grades.

M5: stateful sessions over REST and MCP.
