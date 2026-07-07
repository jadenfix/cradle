# Security

`beatbox` treats generated code as hostile. A successful escape, undeclared host
capability access, or out-of-policy network egress is a critical vulnerability.

## defended classes

- Filesystem exfiltration: deny ambient host filesystem access; expose only
  policy-declared mounts.
- Network exfiltration: deny raw egress by default; future egress must go
  through a logging proxy.
- Resource exhaustion: enforce wall time, fuel or CPU budget, memory, output,
  process, and disk ceilings where the selected lane supports them. The W0
  `wasm` lane bounds compute via `wall_ms` + `fuel` and host memory via
  `memory_bytes` (linear memory and tables share that budget); it cannot honor
  an independent `cpu_ms`, `pids`, or `disk_bytes` ceiling and so rejects a
  request that sets any of them to a non-default value rather than silently
  ignoring it.
- Persistence and lateral movement: deny writes outside the workspace and deny
  access to localhost, LAN, cloud metadata, launch agents, hooks, and host env.
- Browser automation: Beatbox does not currently claim a runnable browser
  sandbox. `/v1/browser/profiles` is authenticated control-plane metadata for
  integration planning; MCP exposes the same contract through
  `get_browser_profiles` with structured content. `POST /v1/browser/admit` and
  MCP `admit_browser_session` are authenticated fail-closed preflight gates; the
  current decision is always rejected, even when downgrade is allowed. Callers
  may request specific isolation controls and declare target origins,
  credential mode, artifact mode, and `sensitive_activity_mode`, but those
  controls remain planned metadata until a real implementation enforces fresh
  profiles, network suppression or allowlisting, credential isolation,
  teardown, suppression of ambient browser state or unapproved persistence, and
  any stated encryption behavior in the production call path. Target origin declarations
  reject paths, credentials, localhost, private/LAN IP space, and link-local
  metadata targets so future browser adapters cannot silently turn a sensitive
  browsing preflight into local control-plane or network exploration.
  Admission responses include a `guard_plan`, including
  `guard_plan.suppression`, but it is a required future enforcement plan, not
  evidence that browser isolation or suppression is currently active.
  The `adapter_handoff` contract remains fail-closed: `launchable` is false and
  `launch_endpoint` is null until a production launcher, teardown path, and
  proof channel exist. Its `launch_request_template` is a secret-free fixture
  for adapter authors and must not be interpreted as a launch grant. Its
  template lease/replay fields are a contract for future enforcement; live
  launch-plan envelopes can additionally be stored in the bounded REST claim
  ledger, but that still is not evidence of adapter trust or production browser
  launch isolation. Its `completion_proof_contract` and
  `completion_report_template` are contract fixtures only; the booleans are not
  evidence until production teardown checks
  derive and verify them from the actual browser process, profile directory,
  artifact store, and egress log path. `POST
  /v1/browser/adapter/completion/validate` and MCP
  `validate_browser_adapter_completion` keep that same boundary: a
  `report_shape_complete` response only means the submitted JSON matched the
  expected proof ids and fields, while `verified_on_production_path` and
  `trusted_for_sensitive_work` remain false until Beatbox can bind the report
  to a real launch request and production teardown evidence.
  Direct adapter contract discovery through `/v1/browser/adapter/contract` and
  MCP `get_browser_adapter_contract` is authenticated control-plane metadata
  only. It publishes the planned contract and conformance fixtures without
  registering an adapter, trusting an endpoint, or making browser launchable.
  Launch planning through `/v1/browser/adapter/launch/plan` is REST-only and
  absent from MCP because the request carries a same-user capability candidate.
  A matched capability may set `same_user_capability_bound`, but the response
  still rejects launch and keeps `launchable`, `trusted_for_sensitive_work`,
  and `endpoint_network_policy_bound` false until production endpoint binding,
  launch, and teardown verification exist. Capabilities can optionally bind a
  `sensitive_activity_mode`; a mismatch fails closed and does not make a
  weaker or stronger mode launchable. Capability-bound launch plans are
  recorded in a bounded in-memory replay ledger, and
  `/v1/browser/adapter/launch/claim` can claim one unmodified, unexpired
  server-issued envelope exactly once. Claim success is not endpoint trust or
  permission to launch a browser.
  Same-user adapter capability issuance through
  `/v1/browser/adapter/capability` is REST-only and must never be exposed as an
  MCP/model-facing tool. The issuer requires configured daemon auth, stores
  only a bounded in-memory digest, and returns short-lived one-time bearer
  material that must stay out of logs and transcripts.
  Adapter registration preflight through `/v1/browser/adapter/register` and
  MCP `register_browser_adapter` is also fail-closed. It requires a same-user
  capability but never echoes it; a live matching issued capability can only
  set `same_user_capability_bound`. Responses still keep `registered`,
  `launchable`, trust, and endpoint binding false until the production control
  path enforces those invariants.
  Adapter manifest validation is also fail-closed. It rejects unsafe launch
  endpoint shapes, reports contract gaps, and marks endpoint network-policy
  binding false because DNS/proxy/redirect/retry binding is not implemented; a
  field-complete manifest is still untrusted metadata rather than permission to
  launch browser automation. The returned `conformance_profile`, including
  `field_complete_launch_request` and its completion-report fixture, is safe to
  use as a compatibility fixture because all cases remain fail-closed and no
  case grants adapter trust.

## current grades

| lane | Linux | macOS | status |
| --- | --- | --- | --- |
| `wasm` | prod-grade substrate | prod-grade substrate | implemented as an empty-linker Wasmtime lane with fuel, epoch interruption, and store limits. |
| `python-wasi`, `js-wasm` | planned prod-grade substrate | planned prod-grade substrate | not implemented yet. |
| `python-native`, `js-native`, `exec` | planned OS jail | planned dev-grade OS jail | not implemented yet. |

Browser sandbox profiles are not execution lanes yet. The cataloged levels
range from an explicitly non-sandboxed external-browser instrumentation mode to
planned ephemeral, network-suppressed, sealed-state, OS-isolated, and
remote-isolated profiles. Encryption is claimed only as future behavior unless
the profile response names the algorithm, key source, and plaintext lifetime.

## out of scope for v1

Hardware side channels, malicious host operating systems, and kernel zero-days
are not eliminated. The roadmap includes a microVM backend for stronger process
and kernel separation where hardware support is available.
