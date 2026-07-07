# Beatbox SDK — shared design brief

Every language SDK in `sdks/<lang>/` implements this identical contract against the
beatbox daemon's REST API. The canonical machine-readable spec is
[`sdks/openapi.json`](./openapi.json); this brief is the human contract the SDKs
must match. Keep the 7 SDKs (TypeScript, Python, Go, Java, Ruby, PHP, C#) as
consistent as language idioms allow — same method names, same config, same error
model — so an agent that knows one knows them all.

## Design rules

1. **Zero / minimal dependencies.** Use each language's standard-library HTTP
   client and JSON. The only exception is Java (the JDK has no JSON): use Jackson.
   No third-party HTTP client, no code-gen runtime.
2. **Idiomatic.** Follow each language's conventions (naming case, error handling,
   package layout, docs). A user should feel it was hand-written for them.
3. **Typed models** mirroring `openapi.json` components. Field names on the wire
   are snake_case (`wall_ms`, `cpu_time_ms`); expose them idiomatically per
   language but serialize to the exact wire names.
4. **No panics/uncaught crashes** on API errors — surface a typed error.

## Configuration

A client is constructed with:

- `base_url` (required), e.g. `http://127.0.0.1:7300`. Trim trailing slashes.
- `token` (optional). When set, send `Authorization: Bearer <token>` on every
  authenticated request.
- `timeout_ms` (optional, default 65000 milliseconds). Languages may also expose
  idiomatic duration helpers.

`api_key`/`apiKey` remains a legacy compatibility alias. When set and `token` is
not set, send `x-beatbox-api-key: <key>` through the same server-side verifier.
Do not document API-key fields as canonical.

## Methods (identical across languages, names adapted to case convention)

| Method | HTTP | Auth | Returns |
| --- | --- | --- | --- |
| `health()` | `GET /v1/health` | no | `{status, version, uptime_s}` (raw JSON ok) |
| `capabilities()` | `GET /v1/capabilities` | yes | raw JSON |
| `integration()` / `getIntegrationContract()` | `GET /v1/integration` | yes | raw JSON / `EcosystemIntegrationContract` |
| `browser_profiles()` | `GET /v1/browser/profiles` | yes | raw JSON |
| `browser_admit(request)` | `POST /v1/browser/admit` | yes | raw JSON |
| `browser_adapter_contract` / `browserAdapterContract` / `BrowserAdapterContract` | `GET /v1/browser/adapter/contract` | yes | raw JSON |
| `browser_adapter_capability` / `issueBrowserAdapterCapability` / `IssueBrowserAdapterCapability` | `POST /v1/browser/adapter/capability` | yes | raw JSON |
| `browser_adapter_register` / `registerBrowserAdapter` / `RegisterBrowserAdapter` | `POST /v1/browser/adapter/register` | yes | raw JSON |
| `browser_adapter_launch_plan` / `planBrowserAdapterLaunch` / `PlanBrowserAdapterLaunch` | `POST /v1/browser/adapter/launch/plan` | yes | raw JSON |
| `browser_adapter_launch_claim` / `claimBrowserAdapterLaunch` / `ClaimBrowserAdapterLaunch` | `POST /v1/browser/adapter/launch/claim` | yes | raw JSON |
| `browser_adapter_validate` / `validate_browser_adapter` / `validateBrowserAdapter` / `ValidateBrowserAdapter` | `POST /v1/browser/adapter/validate` | yes | raw JSON |
| `browser_adapter_completion_validate` / `validate_browser_adapter_completion` / `validateBrowserAdapterCompletion` / `ValidateBrowserAdapterCompletion` | `POST /v1/browser/adapter/completion/validate` | yes | raw JSON |
| `execute(request)` | `POST /v1/execute` | yes | `ExecutionResult` |
| `create_job(request)` | `POST /v1/jobs` | yes | `CreateJobResponse` (`202`) |
| `get_job(job_id)` | `GET /v1/jobs/{id}` | yes | `JobRecord` |
| `cancel_job(job_id)` | `DELETE /v1/jobs/{id}` | yes | void (`204`) |
| `openapi()` | `GET /openapi.json` | no | raw JSON |

- `POST` bodies are JSON with header `content-type: application/json`.
- **Percent-encode `job_id`** as a single path segment; reject an empty, `.`, or
  `..` id (they can retarget the request). Server ids are UUIDs.
- Do not follow redirects (so auth headers can't leak cross-origin).

## Request model (`ExecuteRequest`)

```jsonc
{
  "lane": "wasm",                       // required; enum: wasm|python_wasi|python_native|js_wasm|js_native|exec
  "source": { "kind": "wasm_wat", "text": "(module ...)" }, // required; see Source
  "entrypoint": "run",                  // optional
  "input": { "n": 10 },                 // optional; ANY json (object, int, null, ...)
  "stdin": "",                          // optional
  "policy": { "limits": { "wall_ms": 5000 } }, // optional; partial limits merge onto defaults
  "idempotency_key": "step-1"           // optional
}
```

`Source` is a tagged union on `kind`: `inline{code}`, `wasm_file{path}`,
`wasm_wat{text}`, `wasm_bytes_base64{bytes}`, `module_ref{sha256}`. Provide a
helper/constructor per variant. (Remote API only accepts `wasm_wat` and
`wasm_bytes_base64` for the wasm lane.)

Provide ergonomic constructors so the 90% case is one line, e.g.
`client.execute(ExecuteRequest.wasm_wat("(module ...)", input=...))`.

## Response models

Mirror these components from `openapi.json`: `CapabilitiesResponse`,
`EcosystemIntegrationContract`,
`BrowserProfilesResponse`, `BrowserAdmissionRequest`, `BrowserAdmissionResponse`
(including `target_origins`, `credential_mode`, `artifact_mode`,
`sensitive_activity_mode`, `required_controls`, profile `controls`,
profile-discovery `suppression_modes`,
`missing_controls`, `sensitive_activity_mode_compatible`,
`sensitive_activity_mode_compatible_levels`,
`sensitive_activity_mode_required_controls`,
`sensitive_activity_mode_missing_controls`, and `intent_warnings`, plus the
browser admission `guard_plan` with `suppression` and
`adapter_handoff` with its `launch_request_template`,
`completion_proof_contract`, and `completion_report_template`; raw JSON return
is acceptable until each language adds typed convenience models; preserve
`issued_at`, `expires_at`, `max_session_seconds`, `sensitive_activity_mode`, and
`replay_protection_required` on every launch request envelope, plus
`adapter_contract_fields_complete` and `replay_protection_bound` on launch-plan
responses),
`BrowserAdapterManifestRequest`,
`BrowserAdapterContractResponse`, `BrowserAdapterCapabilityIssueRequest`,
`BrowserAdapterCapabilityIssueResponse`, `BrowserAdapterRegistrationRequest`,
`BrowserAdapterRegistrationResponse`, `BrowserAdapterRegistrationDecision`,
`BrowserAdapterManifestResponse` (including `conformance_profile`),
`BrowserAdapterConformanceProfile`, `BrowserAdapterLaunchRequest`,
`BrowserAdapterLaunchPlanRequest`, `BrowserAdapterLaunchPlanResponse`,
`BrowserAdapterLaunchPlanDecision`, `BrowserAdapterLaunchClaimRequest`,
`BrowserAdapterLaunchClaimResponse`, `BrowserAdapterLaunchClaimDecision`,
`BrowserAdapterCompletionProofRequirement`, `BrowserAdapterCompletionReport`,
`BrowserAdapterConformanceCase`, `BrowserAdapterConformanceExpectation`,
`BrowserAdapterValidationDecision`,
`BrowserAdapterCompletionValidationResponse` (including
`server_issued_launch_request`, `launch_request_claimed`,
`launch_request_envelope_matched`, `completion_report_template_matched`, and
`completion_bound_to_claimed_launch`),
`BrowserAdapterCompletionValidationDecision`,
`ExecutionResult` (status, value,
stdout/stderr, error, `metrics`, `deterministic`, `inputs_digest`,
`effective_isolation`, ...), `Metrics` (`wall_time_ms`, **`cpu_time_ms` nullable**,
`fuel_used` nullable, `peak_memory_bytes` nullable), `JobRecord`
(job_id, status, request, result, error, created_at, updated_at),
`CreateJobResponse` (job_id), `ErrorBody` (code, message), enums
`ExecutionStatus`/`JobStatus`/`Lane`. Unknown/extra fields must not crash
deserialization (forward-compat).

MCP tooling must not expose same-user capability bearer material. REST SDKs
still expose `/v1/browser/adapter/register`; model-facing MCP
`register_browser_adapter` is manifest-only and should not be mirrored as a
capability-consuming helper.
MCP completion validation is also shape-only: keep the launch-ledger binding
fields false there, even though REST completion validation can report binding to
the current daemon's claimed launch envelope.

## Error model

On a non-2xx response, raise/return a typed `BeatboxApiError` carrying:
`status` (HTTP code), `code` (from the `{error:{code,message}}` body), and
`message`. Future shared-client error adapters should also preserve request id
and structured details if the daemon adds them. On a transport failure, raise or
return a typed `BeatboxTransportError`. Never leak the api key into error
messages.

## Each SDK directory must contain

- The client + models (idiomatic layout).
- Package manifest with name `beatbox` (or `beatbox-sdk` where the registry needs
  a scope), version `0.1.0`, license Apache-2.0, repo URL.
- `README.md`: install, quickstart (run a `wasm_wat` add-one and print the value),
  auth, error handling.
- An example that runs `fib(10)` (the wat is in `examples/fib.wat` at repo root) or
  a simple add-one and asserts the result value.
- At least one unit test (mock or against a fixture) that does not require a live
  daemon — e.g. URL building / job-id encoding / model (de)serialization.
- Build/lint/test must pass with the language's standard tool.

## Quickstart shape (pseudocode, keep parity)

```
client = Client(base_url="http://127.0.0.1:7300", token=env["CRADLE_TOKEN"], timeout_ms=65000)
result = client.execute(ExecuteRequest.wasm_wat(
    "(module (func (export \"run\") (param i64) (result i64) local.get 0 i64.const 1 i64.add))",
    input={"n": 41}))
print(result.value)  # 42
```

## Fleet consistency notes (intentional idiomatic differences)

All 7 SDKs share the same method surface, config, auth gating, error split,
job-id handling, and wire field names. A few differences are deliberate, to stay
idiomatic per language rather than uniform for its own sake:

- **Policy typing.** `Limits` is a typed, partial model in every SDK. The rarely
  used nested policy sections (`fs`, `net`, `determinism`, `secrets`) are fully
  typed model classes in the statically-typed SDKs (TypeScript, Go, Java, C#) and
  accepted as native maps/dicts/arrays in the dynamically-typed SDKs (Python,
  Ruby, PHP). Both serialize to the same wire shape.
- **Explicit `input: null`.** The dynamic SDKs plus TypeScript use a sentinel to
  distinguish an omitted `input` from an explicit JSON `null`. Go and C# omit a
  null `input` (it is optional on the wire, so no real request is affected).
- **PHP `ApiError`.** PHP's `Throwable::getCode()` is `final`, so the string API
  error code is exposed as `getErrorCode()` (with `getCode()` returning the HTTP
  status). Every other SDK exposes `code` directly.
- **C# enum forward-compat.** Like every other SDK, C# degrades an unrecognized
  future enum value instead of throwing — via a tolerant per-enum converter that
  maps an unknown wire value to an `Unknown` sentinel member. (The string-backed
  SDKs — Python/Go/Ruby/PHP/TypeScript — instead preserve the raw unknown string;
  C#'s hard enums use the sentinel.) Serializing `Unknown` throws, so the client
  can never send a value the server would not understand.

Repository/homepage in every manifest points at `https://github.com/jadenfix/beatbox`.
