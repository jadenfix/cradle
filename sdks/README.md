# beatbox SDKs

Hand-written, idiomatic client SDKs for the [beatbox](https://github.com/jadenfix/beatbox)
sandbox daemon, in seven languages. Every SDK implements the same contract —
same methods, same config, same auth gating, same error model, same wire field
names — so an agent that knows one knows them all.

| Language | Directory | Package | Install |
| --- | --- | --- | --- |
| TypeScript | [`typescript/`](./typescript) | `beatbox` (npm) | `npm install beatbox` |
| Python | [`python/`](./python) | `beatbox` (PyPI) | `pip install beatbox` |
| Go | [`go/`](./go) | `github.com/jadenfix/beatbox/sdks/go` | `go get github.com/jadenfix/beatbox/sdks/go` |
| Java | [`java/`](./java) | `ai.beatbox:beatbox` (Maven) | Maven/Gradle coordinate |
| Ruby | [`ruby/`](./ruby) | `beatbox` (RubyGems) | `gem install beatbox` |
| PHP | [`php/`](./php) | `beatbox/beatbox` (Packagist) | `composer require beatbox/beatbox` |
| C# | [`csharp/`](./csharp) | `Beatbox` (NuGet) | `dotnet add package Beatbox` |

All SDKs are **zero / minimal dependency** (only Java pulls in Jackson, because
the JDK ships no JSON). None uses a code-gen runtime.

## The contract

The human-readable design contract is [`BRIEF.md`](./BRIEF.md). The canonical
machine-readable API spec is [`openapi.json`](./openapi.json). Every SDK exposes
the same methods:

| Method | HTTP | Auth |
| --- | --- | --- |
| `health` | `GET /v1/health` | no |
| `capabilities` | `GET /v1/capabilities` | yes |
| `integration` / `getIntegrationContract` | `GET /v1/integration` | yes |
| `browser_profiles` / `browserProfiles` | `GET /v1/browser/profiles` | yes |
| `browser_admit` / `admitBrowserSession` | `POST /v1/browser/admit` | yes |
| `browser_adapter_contract` / `browserAdapterContract` | `GET /v1/browser/adapter/contract` | yes |
| `browser_adapter_capability` / `issueBrowserAdapterCapability` | `POST /v1/browser/adapter/capability` | yes |
| `browser_adapter_register` / `registerBrowserAdapter` | `POST /v1/browser/adapter/register` | yes |
| `browser_adapter_launch_plan` / `planBrowserAdapterLaunch` | `POST /v1/browser/adapter/launch/plan` | yes |
| `browser_adapter_launch_claim` / `claimBrowserAdapterLaunch` | `POST /v1/browser/adapter/launch/claim` | yes |
| `validate_browser_adapter` / `validateBrowserAdapter` | `POST /v1/browser/adapter/validate` | yes |
| `browser_adapter_completion_validate` / `validateBrowserAdapterCompletion` | `POST /v1/browser/adapter/completion/validate` | yes |
| `execute` | `POST /v1/execute` | yes |
| `create_job` | `POST /v1/jobs` | yes |
| `get_job` | `GET /v1/jobs/{id}` | yes |
| `cancel_job` | `DELETE /v1/jobs/{id}` | yes |
| `openapi` | `GET /openapi.json` | no |

Current SDKs prefer `token` and send it as `Authorization: Bearer <token>`.
They never send auth on the unauthenticated `health`/`openapi` routes, never put
auth in a URL, and never include it in an error message. `api_key`/`apiKey` and
`x-beatbox-api-key` remain compatibility aliases only when a Bearer token is not
set.

Browser admission requests are raw JSON today. Pass through
`target_origins`, `credential_mode`, `artifact_mode`,
`sensitive_activity_mode`, and `required_controls` exactly as described by
`openapi.json`; beatbox validates unsafe target origins before returning the
fail-closed admission decision. Profile discovery responses publish
`suppression_modes` for the supported sensitive-activity postures, with each
mode's compatible levels, required controls, guard-plan effects, runnable flag,
and required next steps. Admission responses carry
`sensitive_activity_mode_compatible`,
`sensitive_activity_mode_compatible_levels`,
`sensitive_activity_mode_required_controls`, and
`sensitive_activity_mode_missing_controls`, plus `guard_plan` and
`adapter_handoff` blocks; SDKs that return raw JSON must preserve all of these,
including `guard_plan.suppression`, `adapter_handoff.launch_request_template`,
`adapter_handoff.completion_proof_contract`, and the launch template's
`completion_report_template`, so Tempo-style adapters can bind the future
launch and teardown contracts without guessing. Preserve launch-envelope
lease/replay fields (`issued_at`, `expires_at`, `max_session_seconds`,
`sensitive_activity_mode`, and `replay_protection_required`) as opaque raw JSON
fields until a typed model is added.

Browser adapter manifests are also raw JSON today. Pass them through to
`POST /v1/browser/adapter/validate`; beatbox validates the manifest shape and
syntax-checks the launch endpoint, but does not resolve or bind that endpoint
to DNS/proxy/redirect/retry policy. SDKs also expose
`GET /v1/browser/adapter/contract` for direct discovery of the planned adapter
contract and conformance profile without submitting a manifest, and
`POST /v1/browser/adapter/capability` for the REST-only same-user capability
issuer. The issuer requires configured daemon auth, stores only a digest, and
returns short-lived one-time bearer material that must not be exposed to MCP or
model transcripts. A capability can optionally bind `sensitive_activity_mode`;
mode-bound capabilities match only launch-plan admissions with the same mode.
A capability can be consumed by either a matching registration preflight or a
matching launch-plan preflight, so clients should issue a fresh token for each
consuming operation. SDKs also expose
`POST /v1/browser/adapter/register` for the
future registration preflight with actor, sensitivity, an issued same-user
capability, and manifest in one request. Beatbox consumes a matching live
capability at most once and never echoes it. MCP `register_browser_adapter` is
manifest-only and intentionally does not accept that capability; do not mirror
the REST registration secret into model-facing tools.
All adapter registration/validation responses still return
`endpoint_network_policy_bound: false` and `launchable: false` until a trusted
adapter registration and launch path exists. Preserve the `conformance_profile`
field in raw JSON responses; it contains the canonical field-complete manifest,
`field_complete_launch_request`, typed completion-proof requirements,
completion-report fixtures, expected missing-gap reports, and protocol-specific
REST/MCP negative cases Tempo adapters should run.
Launch planning requests are also raw JSON and REST-only:
`POST /v1/browser/adapter/launch/plan` combines a same-user capability,
admission intent, and manifest into a server-issued launch envelope and
completion report template. The envelope includes current server lease
timestamps and a replay-protection requirement. A capability-bound response also
sets `adapter_contract_fields_complete` and `replay_protection_bound`; the
daemon records only field-complete adapter manifests in its bounded replay
ledger. `POST /v1/browser/adapter/launch/claim` accepts the full
`launch_request`, rejects omitted server-issued fields and unknown nested
fields, and can bind exactly one unmodified, unexpired claim before a future
Tempo adapter invocation. SDKs must never expose launch planning or
claiming as MCP/model-visible tooling, and callers must still treat both
responses as non-launchable and untrusted until production endpoint binding,
launch, and teardown checks exist.
Completion reports are raw JSON too. Pass them through to
`POST /v1/browser/adapter/completion/validate`; beatbox checks the submitted
shape, proof ids, and teardown evidence booleans against the same proof
contract. The response also reports whether the request id exists in the launch
ledger, whether it was claimed, whether the envelope identity matches, and
whether the report exactly matches the launch envelope's completion template.
MCP completion validation returns those binding fields as false and remains
shape-only. It still returns a rejected, untrusted response because no
production browser process, profile, artifact store, or egress log has been
verified.

Language-specific method names are idiomatic: Rust and Python expose
`browser_adapter_contract`, `browser_adapter_capability`,
`browser_adapter_register`, `browser_adapter_launch_plan`,
`browser_adapter_launch_claim`, `browser_adapter_validate`, and
`browser_adapter_completion_validate`; Ruby exposes
`browser_adapter_contract`, `browser_adapter_capability`,
`browser_adapter_register`, `browser_adapter_launch_plan`,
`browser_adapter_launch_claim`, `validate_browser_adapter`, and
`validate_browser_adapter_completion`; TypeScript, Java, PHP, and C# expose
`browserAdapterContract`, `issueBrowserAdapterCapability`,
`registerBrowserAdapter`, `planBrowserAdapterLaunch`,
`claimBrowserAdapterLaunch`, `validateBrowserAdapter`, and
`validateBrowserAdapterCompletion`; and Go exposes
`BrowserAdapterContract`, `IssueBrowserAdapterCapability`,
`RegisterBrowserAdapter`, `PlanBrowserAdapterLaunch`,
`ClaimBrowserAdapterLaunch`, `ValidateBrowserAdapter`, and
`ValidateBrowserAdapterCompletion`.

## How the fleet stays correct (the rollout pipeline)

This is a Stainless-style pipeline: one source of truth, drift-checked, and a
gated release.

1. **Source of truth.** `openapi.json` is generated from the Rust server's
   `utoipa` annotations. The server test `beatbox-server::tests/openapi_drift`
   regenerates it and asserts the committed copy matches **byte-for-byte**, so
   the spec can never silently diverge from the daemon that implements it. Runs
   in the main `ci` workflow. Re-bless after an intentional API change with:

   ```bash
   BEATBOX_BLESS_OPENAPI=1 cargo test -p beatbox-server --test openapi_drift
   ```

2. **Per-language CI.** [`.github/workflows/sdk-ci.yml`](../.github/workflows/sdk-ci.yml)
   builds, lints, and tests each SDK on its native toolchain, plus a
   version-consistency gate ([`scripts/check-sdk-versions.sh`](../scripts/check-sdk-versions.sh))
   that requires every manifest to declare the same version as `openapi.json`.

3. **Gated release.** [`.github/workflows/sdk-release.yml`](../.github/workflows/sdk-release.yml)
   is a manual, dry-run-by-default rollout to the language registries, gated
   behind a `release` GitHub Environment. See [`RELEASING.md`](./RELEASING.md).

## Quickstart (shape is identical in every language)

```python
from beatbox import Client, ExecuteRequest

client = Client(base_url="http://127.0.0.1:7300", token="…", timeout_ms=65000)
result = client.execute(ExecuteRequest.wasm_wat(
    '(module (func (export "run") (param i64) (result i64) '
    'local.get 0 i64.const 1 i64.add))',
    input={"n": 41}))
print(result.value)  # 42
```

Each SDK's own `README.md` has the language-native version, install, auth, and
error-handling details.
