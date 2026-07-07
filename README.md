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
`runnable_browser_sessions: false`, serializes `default_level` as `null`, and
marks every profile as `planned` or `unavailable` until a real browser launcher,
egress boundary, storage policy, and teardown path enforce the claim. Call
`POST /v1/browser/admit` or the MCP `admit_browser_session` tool before starting
browser work; callers can include `required_controls` such as `fresh_profile`,
`egress_policy`, `local_network_block`, `sealed_artifacts`, or OS/remote
isolation controls. They can also declare `target_origins`,
`credential_mode`, `artifact_mode`, and `sensitive_activity_mode` so Tempo can
bind a user or agent's intent to an origin allowlist, credential posture,
persistence posture, and explicit suppression level before any browser starts.
The activity modes are `standard`, `private`, `network_suppressed`, and
`sealed`; Beatbox echoes the mode and derives `guard_plan.suppression` booleans
for ambient browser state, credentials, unapproved network, and persistence,
but still treats them as planned enforcement until a real launcher binds them.
Target origins must be public HTTP(S) origins only:
paths, credentials, localhost, private/LAN addresses, and link-local metadata
targets are rejected at preflight. Admission responses include a `guard_plan`
that spells out the network, credential, storage, DNS/redirect revalidation,
and runtime guards a future browser adapter must enforce before the request can
become runnable. They also include an `adapter_handoff` block with the
canonical fields and teardown proofs a future Tempo-side adapter must bind; its
`launch_request_template` is a typed, secret-free envelope showing the exact
future adapter launch request shape for the validated intent. The handoff also
publishes `adapter_handoff.completion_proof_contract`; the nested
`adapter_handoff.launch_request_template.completion_report_template` shows the
matching completion report shape so adapter authors can wire teardown and
storage evidence to stable machine-readable proof ids instead of guessing from
display labels. Template launch envelopes also name the future lease and replay
contract through `issued_at`, `expires_at`, `max_session_seconds`, and
`replay_protection_required`. Its `launch_endpoint` is currently `null` and
`launchable` is `false`. The current implementation always rejects admission
and explains which production pieces or requested controls are still missing.
`GET /v1/browser/adapter/contract` and MCP `get_browser_adapter_contract`
return the same planned adapter contract plus the `conformance_profile` without
requiring a submitted manifest. This is authenticated compatibility metadata
for Tempo and adapter authors; it is not registration, trust, or permission to
launch, and the response keeps `launchable`, `trusted_for_sensitive_work`, and
`endpoint_network_policy_bound` set to `false`.
`POST /v1/browser/adapter/capability` is the authenticated REST-only issuer for
short-lived one-time same-user adapter capabilities. It requires daemon auth to
be configured, stores only a digest in memory, never appears as an MCP tool, and
returns bearer material that callers must keep out of model-visible transcripts.
A capability can optionally bind an adapter id and `sensitive_activity_mode`;
mode-bound capabilities are accepted only by launch-plan admissions with the
same mode. A capability can be consumed by either a matching registration
preflight or a matching launch-plan preflight, so callers should issue a fresh
capability for each consuming operation.
`POST /v1/browser/adapter/register` and MCP `register_browser_adapter` define
the future Tempo adapter registration preflight. Callers submit actor,
sensitivity, a same-user capability, and the adapter manifest in one request.
Beatbox validates the shape and manifest contract, consumes a live matching
issued capability at most once, never echoes the capability, and still returns
`registered: false`, `endpoint_network_policy_bound: false`, and `launchable:
false` until endpoint binding, storage/teardown verification, and browser launch
paths are implemented. A bound capability only flips
`same_user_capability_bound`; it is not registration or launch trust.
`POST /v1/browser/adapter/launch/plan` is a REST-only launch-envelope
preflight for Tempo control-plane code. It submits a same-user capability,
browser admission intent, and adapter manifest together; Beatbox consumes a
matching live capability at most once and returns a server-issued
`launch_request` envelope plus completion report template without echoing the
capability. The live envelope includes `issued_at`, `expires_at`,
`max_session_seconds`, `sensitive_activity_mode`, `replay_protection_required`, and
`replay_protection_bound` so Tempo can tell whether this daemon recorded the
request id in its bounded replay ledger. `POST
/v1/browser/adapter/launch/claim` is the follow-up REST-only preflight Tempo
calls with the full `launch_request` immediately before any adapter invocation;
Beatbox compares it with the canonical stored envelope, rejects mutations,
rejects unknown or already-claimed ids, and marks exactly one matching claim.
Claim success is only replay-state binding, not launch trust. The launch-plan
response remains `rejected`, `launchable: false`,
`trusted_for_sensitive_work: false`, and `endpoint_network_policy_bound: false`
because no production launcher or endpoint request-builder binding exists yet.
There is intentionally no MCP tool for launch planning or claiming because the
flow carries bearer capability material and launch authority.
`POST /v1/browser/adapter/validate` and MCP `validate_browser_adapter` let
Tempo validate a proposed adapter manifest against the same contract. Validation
reports missing levels, controls, guard fields, and completion proofs, but it
only syntax-checks launch endpoints; DNS, proxy, redirect, retry, and request
builder binding remain unimplemented, so it still returns `manifest_complete:
false` and `launchable: false` until a trusted registration and launch path
exists. Validation responses include a `conformance_profile` with a canonical
field-complete manifest, a `field_complete_launch_request` fixture, typed
completion-proof requirements, a completion-report fixture, plus
protocol-specific REST/MCP expectations and required negative cases so Tempo and
adapter authors can test compatibility without guessing.
`POST /v1/browser/adapter/completion/validate` and MCP
`validate_browser_adapter_completion` validate submitted completion reports
against the typed proof contract. They check the report shape, stable
machine-readable `proof_ids`, and teardown evidence booleans, then return
structured `missing_proof_ids`, `unexpected_proof_ids`, and
`failed_evidence_fields`. This path is deliberately fail-closed:
`verified_on_production_path` and `trusted_for_sensitive_work` stay `false`
because Beatbox has not yet bound reports to a server-issued launch request,
real browser process, temporary profile, artifact store, or egress log.

## Ecosystem

beatbox is part of the [ecosystem](https://github.com/jadenfix/ecosystem) — a family of Rust-first, local-first agent-infrastructure projects. It is fully standalone by design: the CLI, daemon, REST API, and MCP endpoint run on their own, and sibling integrations should plug in only over those protocol boundaries. Planned connection points include:

- the sandboxed-execution lane for [beater.js](https://github.com/jadenfix/beater.js) untrusted code, [tempo](https://github.com/jadenfix/tempo) tool execution, and [beaterOS](https://github.com/jadenfix/beaterOS) auditable side effects
