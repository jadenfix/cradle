use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    Wasm,
    PythonWasi,
    PythonNative,
    JsWasm,
    JsNative,
    Exec,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsPolicy {
    #[serde(default)]
    pub workspace: Option<PathBuf>,
    #[serde(default)]
    pub mounts: Vec<Mount>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mount {
    pub host: PathBuf,
    pub guest: PathBuf,
    pub mode: MountMode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MountMode {
    Ro,
    Rw,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NetPolicy {
    #[default]
    Deny,
    Proxy {
        #[serde(default)]
        allow_domains: Vec<String>,
        #[serde(default)]
        allow_ports: Vec<u16>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Secret {
    pub name: String,
    pub value_ref: String,
    pub expose: SecretExpose,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretExpose {
    Env,
    File,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub wall_ms: u64,
    pub cpu_ms: u64,
    pub memory_bytes: u64,
    pub output_bytes: u64,
    pub pids: u32,
    pub disk_bytes: u64,
    pub fuel: Option<u64>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            wall_ms: 5_000,
            cpu_ms: 5_000,
            memory_bytes: 64 * 1024 * 1024,
            output_bytes: 1024 * 1024,
            pids: 1,
            disk_bytes: 64 * 1024 * 1024,
            fuel: Some(10_000_000),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Determinism {
    #[default]
    Off,
    Seeded {
        seed: u64,
        epoch_ms: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreateJobResponse {
    pub job_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobRecord {
    pub job_id: String,
    pub status: JobStatus,
    pub request: ExecuteRequest,
    pub result: Option<ExecutionResult>,
    pub error: Option<ErrorBody>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    Inline { code: String },
    WasmFile { path: PathBuf },
    WasmWat { text: String },
    WasmBytesBase64 { bytes: String },
    ModuleRef { sha256: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Ok,
    Error,
    Timeout,
    Oom,
    Killed,
    Denied,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metrics {
    pub wall_time_ms: u64,
    pub cpu_time_ms: u64,
    pub fuel_used: Option<u64>,
    pub peak_memory_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveIsolation {
    pub os: String,
    pub mechanisms: Vec<String>,
    pub landlock_abi: Option<u32>,
    pub downgrades: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressRecord {
    pub domain: String,
    pub port: u16,
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
