use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    Wasm,
    PythonWasi,
    PythonNative,
    JsWasm,
    JsNative,
    Exec,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSandboxLevel {
    InstrumentedExternal,
    EphemeralProfile,
    NetworkSuppressed,
    SealedState,
    OsIsolated,
    RemoteIsolated,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSandboxAvailability {
    Available,
    Planned,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSandboxControl {
    FreshProfile,
    NoAmbientCredentials,
    EgressPolicy,
    LocalNetworkBlock,
    SealedArtifacts,
    OsProcessIsolation,
    RemoteWorkerIsolation,
    TeardownProof,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserSandboxProfile {
    pub level: BrowserSandboxLevel,
    pub availability: BrowserSandboxAvailability,
    pub summary: String,
    #[serde(default)]
    #[schema(required = true)]
    pub controls: Vec<BrowserSandboxControl>,
    pub isolation_boundary: String,
    pub privacy_controls: Vec<String>,
    pub egress_controls: Vec<String>,
    pub credential_controls: Vec<String>,
    pub storage_controls: Vec<String>,
    pub encryption_claims: Vec<String>,
    pub non_goals: Vec<String>,
    pub downgrade_reasons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserIntegrationContract {
    pub status: BrowserSandboxAvailability,
    pub consumer: String,
    pub endpoint: String,
    pub admission_endpoint: String,
    pub selection_field: String,
    pub required_consumer_behavior: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserProfilesResponse {
    pub version: String,
    pub runnable_browser_sessions: bool,
    #[schema(required = true)]
    pub default_level: Option<BrowserSandboxLevel>,
    pub integration: BrowserIntegrationContract,
    pub profiles: Vec<BrowserSandboxProfile>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSessionActor {
    Agent,
    Human,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSensitivity {
    Public,
    Sensitive,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserCredentialMode {
    #[default]
    NoCredentials,
    UserMediated,
    ScopedSecrets,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserArtifactMode {
    #[default]
    Discard,
    ExplicitDownloads,
    SealedArtifacts,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BrowserAdmissionRequest {
    pub requested_level: BrowserSandboxLevel,
    pub actor: BrowserSessionActor,
    pub sensitivity: BrowserSensitivity,
    /// Bare public HTTP(S) origins this browser session is allowed to target.
    /// Entries must contain only scheme, host, and optional port. The runtime
    /// rejects credentials, paths, queries, fragments, localhost, private/LAN
    /// address space, link-local metadata ranges, and more than 16 entries.
    #[serde(default)]
    pub target_origins: Vec<String>,
    /// Credential posture requested for the session. Non-default modes remain
    /// fail-closed until a real browser substrate implements scoped handling.
    #[serde(default)]
    pub credential_mode: BrowserCredentialMode,
    /// Artifact persistence posture requested for the session. Non-default
    /// modes remain fail-closed until storage and sealing are implemented.
    #[serde(default)]
    pub artifact_mode: BrowserArtifactMode,
    #[serde(default)]
    pub required_controls: Vec<BrowserSandboxControl>,
    #[serde(default)]
    pub allow_downgrade: bool,
    #[serde(default)]
    pub task_label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAdmissionDecision {
    Accepted,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BrowserAdmissionResponse {
    pub decision: BrowserAdmissionDecision,
    pub runnable_browser_sessions: bool,
    pub requested_level: BrowserSandboxLevel,
    #[schema(required = true)]
    pub selected_level: Option<BrowserSandboxLevel>,
    pub actor: BrowserSessionActor,
    pub sensitivity: BrowserSensitivity,
    /// Echo of the validated target origins from the admission request.
    #[serde(default)]
    #[schema(required = true)]
    pub target_origins: Vec<String>,
    /// Echo of the requested credential posture.
    #[serde(default)]
    #[schema(required = true)]
    pub credential_mode: BrowserCredentialMode,
    /// Echo of the requested artifact persistence posture.
    #[serde(default)]
    #[schema(required = true)]
    pub artifact_mode: BrowserArtifactMode,
    #[serde(default)]
    #[schema(required = true)]
    pub requested_controls: Vec<BrowserSandboxControl>,
    #[serde(default)]
    #[schema(required = true)]
    pub requested_profile_controls: Vec<BrowserSandboxControl>,
    #[serde(default)]
    #[schema(required = true)]
    pub missing_controls: Vec<BrowserSandboxControl>,
    #[serde(default)]
    #[schema(required = true)]
    pub level_satisfies_requested_controls: bool,
    /// Non-fatal intent issues that matter before any future runnable browser
    /// session can be admitted.
    #[serde(default)]
    #[schema(required = true)]
    pub intent_warnings: Vec<String>,
    pub downgrade_allowed: bool,
    pub reasons: Vec<String>,
    pub required_next_steps: Vec<String>,
    pub profiles_endpoint: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityLane {
    pub lane: Lane,
    pub available: bool,
    pub substrate: String,
    pub grade: BTreeMap<String, String>,
    pub mechanisms: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityLimits {
    pub sync_wall_ms: u64,
    pub job_wall_ms: u64,
    pub default_wall_ms: u64,
    pub default_memory_bytes: u64,
    pub default_output_bytes: u64,
    pub max_request_bytes: usize,
    pub max_memory_bytes: u64,
    pub max_output_bytes: u64,
    pub max_fuel: u64,
    pub max_concurrent_sync: usize,
    pub max_concurrent_jobs: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilitiesResponse {
    pub version: String,
    pub lanes: Vec<CapabilityLane>,
    pub limits: CapabilityLimits,
    pub engines: BTreeMap<String, String>,
    pub browser_sandbox: BrowserProfilesResponse,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    #[serde(default)]
    pub fs: FsPolicy,
    #[serde(default)]
    pub net: NetPolicy,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub secrets: Vec<Secret>,
    #[serde(default)]
    pub limits: Limits,
    #[serde(default)]
    pub determinism: Determinism,
    #[serde(default)]
    pub double_jail: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FsPolicy {
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub workspace: Option<PathBuf>,
    #[serde(default)]
    pub mounts: Vec<Mount>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Mount {
    #[schema(value_type = String)]
    pub host: PathBuf,
    #[schema(value_type = String)]
    pub guest: PathBuf,
    pub mode: MountMode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MountMode {
    Ro,
    Rw,
}

// `Deny` is an empty *struct* variant (`Deny {}`), not a unit variant, because
// serde silently ignores `deny_unknown_fields` on unit variants of an
// internally-tagged enum — `{"kind":"deny","allow_domains":[...]}` would be
// accepted and the extra keys dropped. An empty struct variant is validated, so
// unknown keys on `deny` are rejected like every other variant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NetPolicy {
    Deny {},
    Proxy {
        #[serde(default)]
        allow_domains: Vec<String>,
        #[serde(default)]
        allow_ports: Vec<u16>,
    },
}

impl Default for NetPolicy {
    fn default() -> Self {
        Self::Deny {}
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Secret {
    pub name: String,
    pub value_ref: String,
    pub expose: SecretExpose,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SecretExpose {
    Env,
    File,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    #[serde(default = "default_wall_ms")]
    pub wall_ms: u64,
    #[serde(default = "default_cpu_ms")]
    pub cpu_ms: u64,
    #[serde(default = "default_memory_bytes")]
    pub memory_bytes: u64,
    #[serde(default = "default_output_bytes")]
    pub output_bytes: u64,
    #[serde(default = "default_pids")]
    pub pids: u32,
    #[serde(default = "default_disk_bytes")]
    pub disk_bytes: u64,
    #[serde(default = "default_fuel")]
    pub fuel: Option<u64>,
}

const fn default_wall_ms() -> u64 {
    5_000
}

const fn default_cpu_ms() -> u64 {
    5_000
}

const fn default_memory_bytes() -> u64 {
    64 * 1024 * 1024
}

const fn default_output_bytes() -> u64 {
    1024 * 1024
}

const fn default_pids() -> u32 {
    1
}

const fn default_disk_bytes() -> u64 {
    64 * 1024 * 1024
}

const fn default_fuel() -> Option<u64> {
    Some(10_000_000)
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            wall_ms: default_wall_ms(),
            cpu_ms: default_cpu_ms(),
            memory_bytes: default_memory_bytes(),
            output_bytes: default_output_bytes(),
            pids: default_pids(),
            disk_bytes: default_disk_bytes(),
            fuel: default_fuel(),
        }
    }
}

// `Off` is an empty struct variant for the same reason as `NetPolicy::Deny`:
// `deny_unknown_fields` is a no-op on unit variants of an internally-tagged
// enum, so `{"kind":"off","seed":5}` would be silently accepted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Determinism {
    Off {},
    Seeded { seed: u64, epoch_ms: u64 },
}

impl Default for Determinism {
    fn default() -> Self {
        Self::Off {}
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecuteRequest {
    pub lane: Lane,
    pub source: Source,
    #[serde(default)]
    pub entrypoint: Option<String>,
    #[serde(default)]
    pub input: serde_json::Value,
    #[serde(default)]
    pub stdin: String,
    #[serde(default)]
    pub policy: Policy,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Canceled,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateJobResponse {
    pub job_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct JobRecord {
    pub job_id: String,
    pub status: JobStatus,
    pub request: ExecuteRequest,
    pub result: Option<ExecutionResult>,
    pub error: Option<ErrorBody>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Source {
    Inline {
        code: String,
    },
    WasmFile {
        #[schema(value_type = String)]
        path: PathBuf,
    },
    WasmWat {
        text: String,
    },
    WasmBytesBase64 {
        bytes: String,
    },
    ModuleRef {
        sha256: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Ok,
    Error,
    Timeout,
    Oom,
    Killed,
    Denied,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ExecutionResult {
    pub status: ExecutionStatus,
    pub value: serde_json::Value,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stdout_truncated: bool,
    pub stderr: String,
    pub stderr_truncated: bool,
    pub error: Option<ErrorBody>,
    pub metrics: Metrics,
    pub lane: Lane,
    pub deterministic: bool,
    pub inputs_digest: String,
    pub engine_version: String,
    pub beatbox_version: String,
    pub effective_isolation: EffectiveIsolation,
    pub egress: Vec<EgressRecord>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Metrics {
    pub wall_time_ms: u64,
    /// CPU time in milliseconds, when the lane measures it separately from wall
    /// time. The W0 wasm lane does not, so this is `None` there — use `fuel_used`
    /// as the deterministic compute signal rather than treating wall time as CPU.
    pub cpu_time_ms: Option<u64>,
    pub fuel_used: Option<u64>,
    pub peak_memory_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
}

impl ErrorBody {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct EffectiveIsolation {
    pub os: String,
    pub mechanisms: Vec<String>,
    pub landlock_abi: Option<u32>,
    pub downgrades: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct EgressRecord {
    pub domain: String,
    pub port: u16,
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ErrorResponse {
    pub error: ErrorBody,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_round_trips_without_defaults_expanding_json() -> Result<(), serde_json::Error> {
        let policy = Policy::default();
        let encoded = serde_json::to_string(&policy)?;
        let decoded: Policy = serde_json::from_str(&encoded)?;
        assert_eq!(decoded, policy);
        Ok(())
    }

    #[test]
    fn partial_limits_merge_onto_defaults() -> Result<(), serde_json::Error> {
        let policy: Policy = serde_json::from_str(r#"{"limits": {"wall_ms": 1000}}"#)?;
        assert_eq!(policy.limits.wall_ms, 1000);
        // Untouched fields fall back to their defaults instead of failing to parse.
        assert_eq!(policy.limits.cpu_ms, Limits::default().cpu_ms);
        assert_eq!(policy.limits.memory_bytes, Limits::default().memory_bytes);
        assert_eq!(policy.limits.fuel, Limits::default().fuel);
        Ok(())
    }

    #[test]
    fn unknown_policy_fields_are_rejected() {
        // A typo'd top-level policy key must be an error, not a silent default.
        assert!(serde_json::from_str::<Policy>(r#"{"double_jale": true}"#).is_err());
        // A typo'd limits key must be rejected too.
        assert!(serde_json::from_str::<Policy>(r#"{"limits": {"wall_mss": 1}}"#).is_err());
    }

    #[test]
    fn unknown_fields_on_tagged_enum_unit_variants_are_rejected() -> Result<(), serde_json::Error> {
        // Regression: deny_unknown_fields must also fire on the empty variants
        // (`deny`, `off`), not just the struct-shaped ones.
        assert!(serde_json::from_str::<NetPolicy>(r#"{"kind":"deny"}"#).is_ok());
        assert!(
            serde_json::from_str::<NetPolicy>(r#"{"kind":"deny","allow_domains":["x"]}"#).is_err()
        );
        assert!(serde_json::from_str::<Determinism>(r#"{"kind":"off"}"#).is_ok());
        assert!(serde_json::from_str::<Determinism>(r#"{"kind":"off","seed":5}"#).is_err());
        // Defaults and round-trips still hold with the empty-struct-variant shape.
        assert_eq!(NetPolicy::default(), NetPolicy::Deny {});
        assert_eq!(Determinism::default(), Determinism::Off {});
        assert_eq!(
            serde_json::to_string(&NetPolicy::default())?,
            r#"{"kind":"deny"}"#
        );
        Ok(())
    }

    #[test]
    fn unknown_request_and_source_fields_are_rejected() {
        assert!(serde_json::from_str::<ExecuteRequest>(
            r#"{"lane": "wasm", "source": {"kind": "wasm_wat", "text": "(module)"}, "polcy": {}}"#
        )
        .is_err());
        assert!(
            serde_json::from_str::<Source>(
                r#"{"kind": "wasm_wat", "text": "(module)", "txt": "x"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn browser_profile_response_tolerates_additive_fields() -> Result<(), serde_json::Error> {
        let response: BrowserProfilesResponse = serde_json::from_str(
            r#"{
                "version": "browser-sandbox-v1",
                "runnable_browser_sessions": false,
                "default_level": null,
                "integration": {
                    "status": "planned",
                    "consumer": "tempo",
                    "endpoint": "/v1/browser/profiles",
                    "admission_endpoint": "/v1/browser/admit",
                    "selection_field": "requested_level",
                    "required_consumer_behavior": ["treat unavailable as rejected"],
                    "future_integration_note": true
                },
                "profiles": [
                    {
                        "level": "network_suppressed",
                        "availability": "planned",
                        "summary": "planned profile from an older daemon",
                        "isolation_boundary": "none yet",
                        "privacy_controls": [],
                        "egress_controls": [],
                        "credential_controls": [],
                        "storage_controls": [],
                        "encryption_claims": [],
                        "non_goals": [],
                        "downgrade_reasons": [],
                        "future_profile_note": "ignored"
                    }
                ],
                "future_response_note": "ignored"
            }"#,
        )?;
        assert_eq!(
            response.profiles[0].controls,
            Vec::<BrowserSandboxControl>::new()
        );
        Ok(())
    }

    #[test]
    fn browser_admission_response_defaults_additive_control_fields() -> Result<(), serde_json::Error>
    {
        let response: BrowserAdmissionResponse = serde_json::from_str(
            r#"{
                "decision": "rejected",
                "runnable_browser_sessions": false,
                "requested_level": "os_isolated",
                "selected_level": null,
                "actor": "agent",
                "sensitivity": "sensitive",
                "downgrade_allowed": false,
                "reasons": ["no runnable browser sandbox"],
                "required_next_steps": ["implement a browser launcher"],
                "profiles_endpoint": "/v1/browser/profiles",
                "future_response_note": "ignored"
            }"#,
        )?;
        assert_eq!(
            response.requested_controls,
            Vec::<BrowserSandboxControl>::new()
        );
        assert_eq!(
            response.requested_profile_controls,
            Vec::<BrowserSandboxControl>::new()
        );
        assert_eq!(
            response.missing_controls,
            Vec::<BrowserSandboxControl>::new()
        );
        assert_eq!(response.target_origins, Vec::<String>::new());
        assert_eq!(
            response.credential_mode,
            BrowserCredentialMode::NoCredentials
        );
        assert_eq!(response.artifact_mode, BrowserArtifactMode::Discard);
        assert_eq!(response.intent_warnings, Vec::<String>::new());
        assert!(!response.level_satisfies_requested_controls);
        Ok(())
    }

    #[test]
    fn browser_admission_request_defaults_additive_intent_fields() -> Result<(), serde_json::Error>
    {
        let request: BrowserAdmissionRequest = serde_json::from_str(
            r#"{
                "requested_level": "network_suppressed",
                "actor": "agent",
                "sensitivity": "sensitive"
            }"#,
        )?;
        assert_eq!(request.target_origins, Vec::<String>::new());
        assert_eq!(
            request.credential_mode,
            BrowserCredentialMode::NoCredentials
        );
        assert_eq!(request.artifact_mode, BrowserArtifactMode::Discard);
        assert_eq!(
            request.required_controls,
            Vec::<BrowserSandboxControl>::new()
        );
        assert!(!request.allow_downgrade);
        assert_eq!(request.task_label, None);
        assert!(
            serde_json::from_str::<BrowserAdmissionRequest>(
                r#"{
                    "requested_level": "network_suppressed",
                    "actor": "agent",
                    "sensitivity": "sensitive",
                    "ambient_cookies": true
                }"#
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn request_round_trips_with_wasm_source() -> Result<(), serde_json::Error> {
        let request = ExecuteRequest {
            lane: Lane::Wasm,
            source: Source::WasmWat {
                text: "(module)".to_string(),
            },
            entrypoint: Some("run".to_string()),
            input: serde_json::json!({"n": 10}),
            stdin: String::new(),
            policy: Policy::default(),
            idempotency_key: Some("step-1".to_string()),
        };
        let encoded = serde_json::to_value(&request)?;
        let decoded: ExecuteRequest = serde_json::from_value(encoded)?;
        assert_eq!(decoded, request);
        Ok(())
    }
}
