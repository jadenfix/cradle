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
mode, `sensitive_activity_mode`, required isolation controls, and any explicit
downgrade allowance before starting browser automation. The activity mode is a
separate privacy/suppression intent, not a sandbox level: `standard`,
`private`, `network_suppressed`, and `sealed` derive
`guard_plan.suppression` requirements for ambient state, credentials,
unapproved egress, persistence, and operator downgrade confirmation. Target
origins are validated as bare public
HTTP(S) origins: no paths, credentials, localhost, private/LAN IP space, or
link-local metadata targets. Profiles publish their planned controls
(`fresh_profile`, `egress_policy`, `local_network_block`, `sealed_artifacts`,
OS/remote isolation, and teardown proof) so Tempo can reason about what a level
would satisfy without guessing from display text. The response echoes the
requested intent, reports the requested profile's planned controls, lists
missing controls, surfaces intent warnings, and returns a `guard_plan` for the
network, credential, storage, suppression, DNS/redirect revalidation, and
runtime guards a future browser adapter must enforce. It also returns
`adapter_handoff`, a
non-launchable handoff contract that names the exact admission fields and
completion proofs a Tempo-side adapter must bind before any future launch path
can be trusted. The handoff includes `launch_request_template`, a concrete
future adapter request envelope derived from the validated admission intent and
guard plan; it is a compatibility fixture, not permission to launch. It also
publishes a typed `adapter_handoff.completion_proof_contract`, while
`adapter_handoff.launch_request_template.completion_report_template` shows the
matching report shape, binding each proof label to a stable proof id, evidence
field, and invariant expected on the eventual teardown path. The launch
envelope also carries lease/replay fields (`issued_at`, `expires_at`,
`max_session_seconds`, and `replay_protection_required`) so a future launcher
has an explicit stale-request and replay boundary to enforce. Admission is the
authoritative decision and currently always rejects because Beatbox has no
runnable browser launcher or isolation substrate.

`GET /v1/browser/adapter/contract` and MCP `get_browser_adapter_contract`
publish the planned adapter contract and conformance profile directly for
Tempo-style discovery. The response is authenticated control-plane metadata and
remains fail-closed: it is not a manifest submission or registration grant, and
it reports `launchable: false`, `trusted_for_sensitive_work: false`, and
`endpoint_network_policy_bound: false`.

`POST /v1/browser/adapter/capability` is the REST-only same-user capability
issuer. It is deliberately absent from MCP, requires configured daemon auth
rather than no-op auth mode, stores only a SHA-256 digest in bounded in-memory
state, prunes expired entries, and returns a short-lived one-time bearer
candidate for the local control plane to submit to registration or launch-plan
preflight. Capabilities may optionally bind a `sensitive_activity_mode`; such
capabilities can be consumed only by launch-plan admission with the same mode.
Issuance is not model-facing and does not make any adapter trusted or
launchable.

`POST /v1/browser/adapter/register` is the fail-closed capability-bound
registration preflight. It requires a caller-supplied same-user capability plus
actor, sensitivity, and manifest fields. Beatbox consumes a live matching issued
capability once, never echoes it, and otherwise keeps one authoritative manifest
validation payload. A successful capability match only reports
`same_user_capability_bound: true`; `registered`, `launchable`,
`trusted_for_sensitive_work`, and `endpoint_network_policy_bound` remain false
until the production control plane can bind concrete endpoint network policy,
storage, teardown proofs, and browser launch path. MCP `register_browser_adapter`
is manifest-only and never accepts bearer capability material, so model-facing
callers can inspect adapter shape without carrying same-user secrets.

`POST /v1/browser/adapter/validate` and MCP `validate_browser_adapter` validate
a proposed adapter manifest against the published Tempo handoff contract. The
validator syntax-checks the claimed launch endpoint and checks supported
sandbox levels, controls, guard-plan fields, and completion proofs, then reports
missing pieces. It is intentionally not a registration endpoint: even a
field-complete manifest still returns `decision: rejected`, `manifest_complete:
false`, `endpoint_network_policy_bound: false`, `launchable: false`, and
`trusted_for_sensitive_work: false` until Beatbox has production trust,
endpoint binding, and launch paths. The same response includes a
`conformance_profile` with a canonical field-complete manifest,
`field_complete_launch_request`, typed completion proof requirements, a
completion report fixture, and required accepted-but-rejected and
parser-rejected cases, including separate REST and MCP expectations. That
profile is the adapter author test fixture for protocol compatibility, not a
registration credential.

`POST /v1/browser/adapter/launch/plan` is the REST-only bridge between
admission intent, adapter manifest, and same-user capability binding. It
validates the nested admission and manifest through the production parsers,
consumes a matching one-time capability, and emits a server-issued
`BrowserAdapterLaunchRequest` plus completion report template that Tempo can
carry into a future adapter launcher. Live launch-plan envelopes use current
server `issued_at`/`expires_at` values, require request-id replay protection,
and record capability-bound envelopes in a bounded in-memory replay ledger only
when the manifest satisfies the published adapter field contract.
The launch request carries `sensitive_activity_mode` and the derived
`guard_plan.suppression` section, so Tempo-side adapters must preserve those
fields for claim-time canonical comparison.
`POST /v1/browser/adapter/launch/claim` is the REST-only Tempo-side claim
preflight for that ledger: callers submit the full launch request, Beatbox
requires every server-issued field, rejects unknown nested fields, compares it
with the canonical stored envelope, and exactly one unexpired match can be
claimed. Discovery and conformance templates keep deterministic
placeholder lease values. Launch planning and claiming are not exposed through
MCP because the flow includes bearer capability material and launch authority.
The response is still rejected and non-launchable; capability binding and claim
binding only prove that this local control-plane preflight saw a live token and
an unmodified server-issued envelope for the same actor, sensitivity, and
adapter id.

`POST /v1/browser/adapter/completion/validate` and MCP
`validate_browser_adapter_completion` validate a submitted completion report
against the same typed proof contract. The validator checks stable proof ids,
teardown evidence booleans, and bounded list fields, then returns structured
missing, unexpected, and failed proof feedback. It also remains fail-closed:
`decision` is `rejected`, `verified_on_production_path` is false, and
`trusted_for_sensitive_work` is false. REST completion validation also reports
whether the request id is in the bounded launch ledger, whether that launch
request was claimed, whether request id, adapter id, and contract version match
the recorded envelope, and whether the report exactly matches the embedded
completion template. MCP completion validation returns those binding flags as
false and remains shape-only so model-facing tools cannot probe live launch
state; the binding flags are still not production process, profile, artifact,
or egress verification.

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
