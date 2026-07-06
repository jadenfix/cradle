use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    Wasm,
    PythonWasi,
    PythonNative,
    JsWasm,
    JsNative,
    Exec,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FsPolicy {
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub workspace: Option<PathBuf>,
    #[serde(default)]
    pub mounts: Vec<Mount>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Mount {
    #[schema(value_type = String)]
    pub host: PathBuf,
    #[schema(value_type = String)]
    pub guest: PathBuf,
    pub mode: MountMode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MountMode {
    Ro,
    Rw,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Secret {
    pub name: String,
    pub value_ref: String,
    pub expose: SecretExpose,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SecretExpose {
    Env,
    File,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Determinism {
    #[default]
    Off,
    Seeded {
        seed: u64,
        epoch_ms: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecuteRequest {
    pub lane: Lane,
    pub source: Source,
    #[serde(default)]
    pub entrypoint: Option<String>,
    #[serde(default)]
    #[schema(value_type = Object)]
    pub input: serde_json::Value,
    #[serde(default)]
    pub stdin: String,
    #[serde(default)]
    pub policy: Policy,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct CreateJobResponse {
    pub job_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct JobRecord {
    pub job_id: String,
    pub status: JobStatus,
    pub request: ExecuteRequest,
    pub result: Option<ExecutionResult>,
    pub error: Option<ErrorBody>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Ok,
    Error,
    Timeout,
    Oom,
    Killed,
    Denied,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ExecutionResult {
    pub status: ExecutionStatus,
    #[schema(value_type = Object)]
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Metrics {
    pub wall_time_ms: u64,
    pub cpu_time_ms: u64,
    pub fuel_used: Option<u64>,
    pub peak_memory_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct EffectiveIsolation {
    pub os: String,
    pub mechanisms: Vec<String>,
    pub landlock_abi: Option<u32>,
    pub downgrades: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct EgressRecord {
    pub domain: String,
    pub port: u16,
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
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

    #[test]
    fn limits_deserialize_with_default_filled_missing_fields(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let default = Limits::default();

        let empty: Limits = serde_json::from_value(serde_json::json!({}))?;
        assert_eq!(empty, default);

        let partial: Limits = serde_json::from_value(serde_json::json!({
            "wall_ms": 250,
            "output_bytes": 512,
            "fuel": null
        }))?;
        assert_eq!(partial.wall_ms, 250);
        assert_eq!(partial.output_bytes, 512);
        assert_eq!(partial.fuel, None);
        assert_eq!(partial.cpu_ms, default.cpu_ms);
        assert_eq!(partial.memory_bytes, default.memory_bytes);
        assert_eq!(partial.pids, default.pids);
        assert_eq!(partial.disk_bytes, default.disk_bytes);

        let populated = Limits {
            wall_ms: 123,
            cpu_ms: 456,
            memory_bytes: 789,
            output_bytes: 1011,
            pids: 3,
            disk_bytes: 1213,
            fuel: Some(1415),
        };
        let encoded = serde_json::to_value(&populated)?;
        let decoded: Limits = serde_json::from_value(encoded)?;
        assert_eq!(decoded, populated);
        Ok(())
    }

    #[test]
    fn request_policy_limits_can_be_compact_without_losing_defaults(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let request: ExecuteRequest = serde_json::from_value(serde_json::json!({
            "lane": "wasm",
            "source": {"kind": "wasm_wat", "text": "(module)"},
            "policy": {
                "limits": {
                    "wall_ms": 250
                }
            }
        }))?;

        let default = Policy::default();
        assert_eq!(request.policy.limits.wall_ms, 250);
        assert_eq!(request.policy.limits.cpu_ms, default.limits.cpu_ms);
        assert_eq!(
            request.policy.limits.memory_bytes,
            default.limits.memory_bytes
        );
        assert_eq!(
            request.policy.limits.output_bytes,
            default.limits.output_bytes
        );
        assert_eq!(request.policy.limits.pids, default.limits.pids);
        assert_eq!(request.policy.limits.disk_bytes, default.limits.disk_bytes);
        assert_eq!(request.policy.limits.fuel, default.limits.fuel);
        Ok(())
    }

    #[test]
    fn request_rejects_unknown_fields() -> Result<(), Box<dyn std::error::Error>> {
        let unknown_top_level = serde_json::json!({
            "lane": "wasm",
            "source": {"kind": "wasm_wat", "text": "(module)"},
            "policy": {},
            "surprise": true
        });
        let error = unknown_request_error(unknown_top_level, "top-level request")?;
        assert!(error.to_string().contains("unknown field"));

        let unknown_policy = serde_json::json!({
            "lane": "wasm",
            "source": {"kind": "wasm_wat", "text": "(module)"},
            "policy": {"netwrok": {"kind": "deny"}}
        });
        let error = unknown_request_error(unknown_policy, "policy")?;
        assert!(error.to_string().contains("unknown field"));

        let unknown_source = serde_json::json!({
            "lane": "wasm",
            "source": {"kind": "wasm_wat", "text": "(module)", "path": "/etc/passwd"},
            "policy": {}
        });
        let error = unknown_request_error(unknown_source, "source")?;
        assert!(error.to_string().contains("unknown field"));

        let unknown_limit = serde_json::json!({
            "lane": "wasm",
            "source": {"kind": "wasm_wat", "text": "(module)"},
            "policy": {"limits": {"wall_mz": 1}}
        });
        let error = unknown_request_error(unknown_limit, "limits")?;
        assert!(error.to_string().contains("unknown field"));
        Ok(())
    }

    fn unknown_request_error(
        value: serde_json::Value,
        context: &'static str,
    ) -> Result<serde_json::Error, Box<dyn std::error::Error>> {
        match serde_json::from_value::<ExecuteRequest>(value) {
            Ok(_) => Err(format!("unknown {context} fields must be rejected").into()),
            Err(error) => Ok(error),
        }
    }
}
