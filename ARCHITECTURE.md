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
| `crates/beatbox-client` | near-zero-abstraction `reqwest` client for `/v1`. |
| `bins/beatbox` | local CLI. It can execute directly in-process or call a remote `beatboxd`. |
| `bins/beatboxd` | daemon wrapper around `beatbox-server`. |

## isolation model

Two substrates are planned:

1. In-process Wasmtime for `wasm`, `python-wasi`, and `js-wasm`.
2. OS jails for native Python, native JS, and generic exec.

The initial implementation covers the `wasm` lane for core Wasm modules. It
rejects imports through an empty linker, consumes fuel, interrupts long
wall-clock runs with Wasmtime epoch deadlines, caps linear-memory growth with
`StoreLimits`, and records the actual mechanisms in every `ExecutionResult`.
WASI command/component stdin/stdout support is the next expansion of this lane.

## policy contract

Every execution receives one `Policy`. Lanes must reject policies they cannot
enforce when the unsupported field would widen exposure. Safer-by-absence
behavior, such as no process spawning in an in-process Wasmtime lane, is
reported as enforced by construction.

No lane should inherit host environment variables or raw network access. Future
egress will be routed through a logging localhost proxy with domain and port
allowlists.

## browser sandbox contract

Browser automation is a separate integration surface from code execution lanes.
`GET /v1/browser/profiles` returns the typed profile catalog that Tempo or
another browser-capable caller can use to decide whether Beatbox currently has a
usable browser sandbox for sensitive work. The same payload is embedded under
`browser_sandbox` in `/v1/capabilities` and exposed to model-facing callers
through the MCP `get_browser_profiles` tool.

The MCP tool returns one authoritative structured payload in
`structuredContent`; the text content is only a short label. Model-facing
callers should not parse a second serialized JSON copy out of text.

`POST /v1/browser/admit` and the MCP `admit_browser_session` tool are the
fail-closed preflight path for browser work. Callers submit the requested
sandbox level, actor, sensitivity, target origins, credential mode, artifact
mode, required isolation controls, and any explicit downgrade allowance before
starting browser automation. Target origins are validated as bare public
HTTP(S) origins: no paths, credentials, localhost, private/LAN IP space, or
link-local metadata targets. Profiles publish their planned controls
(`fresh_profile`, `egress_policy`, `local_network_block`, `sealed_artifacts`,
OS/remote isolation, and teardown proof) so Tempo can reason about what a level
would satisfy without guessing from display text. The response echoes the
requested intent, reports the requested profile's planned controls, lists
missing controls, and surfaces intent warnings. It is the authoritative
admission decision and currently always rejects because Beatbox has no runnable
browser launcher or isolation substrate.

The current catalog is intentionally non-runnable: `runnable_browser_sessions`
is false, `default_level` is serialized as `null`, and no profile is marked
`available`. Profiles describe planned levels rather than enforced behavior:
instrumented external browsers, ephemeral profiles, network-suppressed
profiles, sealed persisted state, OS-isolated browsers, and remote isolated
workers. A consumer must not silently downgrade sensitive work to a weaker
profile; it must treat `planned` and `unavailable` as non-runnable until a
future implementation supplies a browser launcher, teardown, egress boundary,
storage policy, and tests that exercise the production path.

## milestones

M0: workspace scaffold, toolchain pin, core serde types, tests, and CI.

M1: WASI/Wasmtime lane through the CLI with fuel, wall-clock, memory, and output
limits plus escape regression tests.

M2: `beatboxd` REST/MCP API, auth, OpenAPI, and job persistence.

Current job cancellation is best-effort: `DELETE /v1/jobs/{id}` marks a queued
or running record as canceled, and a running worker's later result is ignored.
The underlying compute is still bounded by the execution policy until per-job
engine interruption handles are added.

M3: Tempo/browser integration contract and `beater.js` Tier-4 integration
through `beatbox-client`.

M4: Python and JavaScript lanes, browser profiles, native OS jails, and honest
per-OS capability grades.

M5: stateful sessions over REST and MCP.
