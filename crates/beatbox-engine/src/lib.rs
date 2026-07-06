use std::time::{Duration, Instant};

use beatbox_core::{
    EffectiveIsolation, ErrorBody, ExecuteRequest, ExecutionResult, ExecutionStatus, Lane, Metrics,
    NetPolicy, Source,
};
use thiserror::Error;

pub const MAX_WASM_MODULE_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_PYTHON_SOURCE_BYTES: u64 = 1024 * 1024;

#[derive(Clone)]
pub struct BeatboxEngine {
    #[cfg(feature = "lane-wasi")]
    wasm: wasm::WasmLane,
    #[cfg(feature = "lane-python")]
    python_native: python_native::PythonNativeLane,
}

impl BeatboxEngine {
    pub fn new() -> Result<Self, EngineError> {
        Ok(Self {
            #[cfg(feature = "lane-wasi")]
            wasm: wasm::WasmLane::new()?,
            #[cfg(feature = "lane-python")]
            python_native: python_native::PythonNativeLane::new(),
        })
    }

    pub fn execute(&self, request: ExecuteRequest) -> Result<ExecutionResult, EngineError> {
        self.execute_inner(request, None)
    }

    pub fn execute_with_cancellation(
        &self,
        request: ExecuteRequest,
        cancellation: CancellationToken,
    ) -> Result<ExecutionResult, EngineError> {
        self.execute_inner(request, Some(cancellation))
    }

    fn execute_inner(
        &self,
        request: ExecuteRequest,
        cancellation: Option<CancellationToken>,
    ) -> Result<ExecutionResult, EngineError> {
        if cancellation
            .as_ref()
            .is_some_and(CancellationToken::is_canceled)
        {
            return Ok(result(
                &request,
                ExecutionStatus::Killed,
                serde_json::Value::Null,
                Some(ErrorBody::new("canceled", "execution canceled")),
                Metrics::default(),
                EffectiveIsolation::for_current_os(Vec::new(), Vec::new()),
                digest_fallback(&request),
            ));
        }
        match request.lane.clone() {
            Lane::Wasm => {
                #[cfg(feature = "lane-wasi")]
                {
                    self.wasm.execute(request, cancellation)
                }
                #[cfg(not(feature = "lane-wasi"))]
                {
                    Ok(denied_result(
                        request,
                        ErrorBody::new("lane_unavailable", "wasm lane is not compiled in"),
                        EffectiveIsolation::for_current_os(Vec::new(), Vec::new()),
                    ))
                }
            }
            Lane::PythonNative => {
                #[cfg(feature = "lane-python")]
                {
                    self.python_native.execute(request, cancellation)
                }
                #[cfg(not(feature = "lane-python"))]
                {
                    Ok(denied_result(
                        request,
                        ErrorBody::new("lane_unavailable", "python_native lane is not compiled in"),
                        EffectiveIsolation::for_current_os(Vec::new(), Vec::new()),
                    ))
                }
            }
            lane => Ok(denied_result(
                request,
                ErrorBody::new(
                    "lane_unavailable",
                    format!("{lane:?} is not implemented in this milestone"),
                ),
                EffectiveIsolation::for_current_os(Vec::new(), Vec::new()),
            )),
        }
    }
}

impl Default for BeatboxEngine {
    fn default() -> Self {
        match Self::new() {
            Ok(engine) => engine,
            Err(error) => panic!("default BeatboxEngine must construct: {error}"),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    canceled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.canceled
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn is_canceled(&self) -> bool {
        self.canceled.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("policy field {field} cannot be enforced for lane {lane:?} on {os}: {reason}")]
    PolicyUnenforceable {
        field: &'static str,
        lane: Lane,
        os: String,
        reason: String,
    },
    #[error("invalid source for lane {lane:?}: {reason}")]
    InvalidSource { lane: Lane, reason: String },
    #[error("request field {field} cannot be used for lane {lane:?}: {reason}")]
    UnsupportedRequestField {
        field: &'static str,
        lane: Lane,
        reason: String,
    },
    #[error("failed to read source {path}: {source}")]
    ReadSource {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to build sandbox profile: {reason}")]
    SandboxProfile { reason: String },
    #[error("source field {field} is too large: {actual} bytes exceeds limit {limit} bytes")]
    SourceTooLarge {
        field: &'static str,
        actual: u64,
        limit: u64,
    },
    #[error(
        "source field {field} may decode too large: estimated {estimate} bytes exceeds limit {limit} bytes"
    )]
    SourceEstimateTooLarge {
        field: &'static str,
        estimate: u64,
        limit: u64,
    },
    #[error("failed to decode base64 wasm bytes: {0}")]
    DecodeBase64(String),
    #[error("failed to parse wasm text: {0}")]
    ParseWat(String),
    #[error("failed to build wasmtime engine: {0}")]
    WasmtimeEngine(String),
    #[error("failed to serialize execution inputs: {0}")]
    Serialize(#[from] serde_json::Error),
}

impl EngineError {
    pub fn error_body(&self) -> ErrorBody {
        match self {
            Self::PolicyUnenforceable { field, reason, .. } => ErrorBody::new(
                "policy_unenforceable",
                format!("policy field {field} cannot be enforced: {reason}"),
            ),
            Self::InvalidSource { reason, .. } => ErrorBody::new("invalid_source", reason),
            Self::UnsupportedRequestField { field, reason, .. } => ErrorBody::new(
                "unsupported_request_field",
                format!("request field {field} cannot be used: {reason}"),
            ),
            Self::ReadSource { path, source } => {
                ErrorBody::new("read_source", format!("failed to read {path}: {source}"))
            }
            Self::SandboxProfile { reason } => ErrorBody::new("sandbox_profile", reason),
            Self::SourceTooLarge {
                field,
                actual,
                limit,
            } => ErrorBody::new(
                "source_limit",
                format!(
                    "source field {field} is too large: {actual} bytes exceeds limit {limit} bytes"
                ),
            ),
            Self::SourceEstimateTooLarge {
                field,
                estimate,
                limit,
            } => ErrorBody::new(
                "source_limit",
                format!(
                    "source field {field} may decode too large: estimated {estimate} bytes exceeds limit {limit} bytes"
                ),
            ),
            Self::DecodeBase64(message) => ErrorBody::new("decode_base64", message),
            Self::ParseWat(message) => ErrorBody::new("parse_wat", message),
            Self::WasmtimeEngine(message) => ErrorBody::new("wasmtime_engine", message),
            Self::Serialize(error) => ErrorBody::new("serialize_inputs", error.to_string()),
        }
    }
}

pub trait EffectiveIsolationExt {
    fn for_current_os(mechanisms: Vec<String>, downgrades: Vec<String>) -> Self;
}

impl EffectiveIsolationExt for EffectiveIsolation {
    fn for_current_os(mechanisms: Vec<String>, downgrades: Vec<String>) -> Self {
        Self {
            os: std::env::consts::OS.to_string(),
            mechanisms,
            landlock_abi: None,
            downgrades,
        }
    }
}

fn denied_result(
    request: ExecuteRequest,
    error: ErrorBody,
    isolation: EffectiveIsolation,
) -> ExecutionResult {
    let started = Instant::now();
    result(
        &request,
        ExecutionStatus::Denied,
        serde_json::Value::Null,
        Some(error),
        Metrics {
            wall_time_ms: elapsed_ms(started),
            ..Metrics::default()
        },
        isolation,
        digest_fallback(&request),
    )
}

fn result(
    request: &ExecuteRequest,
    status: ExecutionStatus,
    value: serde_json::Value,
    error: Option<ErrorBody>,
    metrics: Metrics,
    effective_isolation: EffectiveIsolation,
    inputs_digest: String,
) -> ExecutionResult {
    let (error, stderr, stderr_truncated) =
        truncate_error(error, request.policy.limits.output_bytes);
    result_with_stdio(
        request,
        status,
        value,
        String::new(),
        false,
        stderr,
        stderr_truncated,
        error,
        metrics,
        effective_isolation,
        inputs_digest,
    )
}

#[allow(clippy::too_many_arguments)]
fn result_with_stdio(
    request: &ExecuteRequest,
    status: ExecutionStatus,
    value: serde_json::Value,
    stdout: String,
    stdout_truncated: bool,
    stderr: String,
    stderr_truncated: bool,
    error: Option<ErrorBody>,
    metrics: Metrics,
    effective_isolation: EffectiveIsolation,
    inputs_digest: String,
) -> ExecutionResult {
    // The initial wasm lane is W0: an empty linker denies every host import, so
    // core modules are deterministic by construction. Revisit this when W1
    // capability-scoped WASI is added under Lane::Wasm.
    let deterministic = matches!(request.lane, Lane::Wasm);
    ExecutionResult {
        status,
        value,
        exit_code: None,
        stdout,
        stdout_truncated,
        stderr,
        stderr_truncated,
        error,
        metrics,
        lane: request.lane.clone(),
        deterministic,
        inputs_digest,
        engine_version: engine_version(),
        beatbox_version: env!("CARGO_PKG_VERSION").to_string(),
        effective_isolation,
        egress: Vec::new(),
    }
}

fn truncate_error(
    error: Option<ErrorBody>,
    output_bytes: u64,
) -> (Option<ErrorBody>, String, bool) {
    let Some(mut body) = error else {
        return (None, String::new(), false);
    };
    let (message, truncated) = truncate(body.message, output_bytes);
    body.message = message.clone();
    (Some(body), message, truncated)
}

fn engine_version() -> String {
    "wasmtime-45".to_string()
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn truncate(value: String, max_bytes: u64) -> (String, bool) {
    let max = usize::try_from(max_bytes).unwrap_or(usize::MAX);
    if value.len() <= max {
        return (value, false);
    }
    let mut end = max;
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    (value[..end].to_string(), true)
}

fn ensure_source_limit(field: &'static str, actual: u64, limit: u64) -> Result<(), EngineError> {
    if actual > limit {
        Err(EngineError::SourceTooLarge {
            field,
            actual,
            limit,
        })
    } else {
        Ok(())
    }
}

fn bytes_len_u64(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

fn digest_fallback(request: &ExecuteRequest) -> String {
    #[cfg(feature = "lane-wasi")]
    {
        digest_json(request).unwrap_or_else(|_| "sha256:unavailable".to_string())
    }
    #[cfg(not(feature = "lane-wasi"))]
    {
        let _ = request;
        "sha256:unavailable".to_string()
    }
}

#[cfg(feature = "lane-wasi")]
fn digest_json<T: serde::Serialize>(value: &T) -> Result<String, serde_json::Error> {
    use sha2::{Digest, Sha256};

    let bytes = serde_json::to_vec(value)?;
    let digest = Sha256::digest(bytes);
    Ok(format!("sha256:{}", hex::encode(digest)))
}

#[cfg(feature = "lane-python")]
pub fn python_native_available() -> bool {
    python_native::python_available()
}

#[cfg(not(feature = "lane-python"))]
pub fn python_native_available() -> bool {
    false
}

#[cfg(feature = "lane-python")]
mod python_native {
    use std::fs;
    use std::io::{self, Read, Write};
    use std::path::{Path, PathBuf};
    use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    use beatbox_core::{Determinism, MountMode, Policy};

    use super::*;

    #[derive(Clone)]
    pub struct PythonNativeLane;

    struct CapturedOutput {
        text: String,
        truncated: bool,
    }

    pub(super) enum ChildOutcome {
        Exited(ExitStatus),
        Timeout,
        Canceled,
        StdinError(String),
        DiskLimitExceeded { used_bytes: u64, max_bytes: u64 },
        DiskLimitProbeError(String),
    }

    pub(super) struct TempWorkspace {
        pub(super) path: PathBuf,
    }

    pub(super) struct WorkspaceDiskLimit<'a> {
        path: &'a Path,
        max_bytes: u64,
    }

    #[derive(Clone)]
    enum SandboxFilter {
        Literal(PathBuf),
        Subpath(PathBuf),
    }

    const PYTHON_DISK_CHECK_INTERVAL_MS: u64 = 25;

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    impl PythonNativeLane {
        pub fn new() -> Self {
            Self
        }

        pub fn execute(
            &self,
            request: ExecuteRequest,
            cancellation: Option<CancellationToken>,
        ) -> Result<ExecutionResult, EngineError> {
            admit_python_native_policy(&request.policy)?;
            admit_python_native_request(&request)?;
            let source_limit = python_source_byte_limit(request.policy.limits.memory_bytes);
            let source = python_source(&request, source_limit)?;
            let started = Instant::now();
            let isolation = python_isolation();
            let inputs_digest = digest_python_inputs(&request)?;
            let Some(python) = python_binary() else {
                return Ok(denied_result(
                    request,
                    ErrorBody::new(
                        "lane_unavailable",
                        "python_native requires python3 and sandbox-exec on macOS",
                    ),
                    isolation,
                ));
            };
            let workspace = make_workspace().map_err(|source| EngineError::ReadSource {
                path: std::env::temp_dir().display().to_string(),
                source,
            })?;
            let profile = sandbox_profile(&workspace.path, &python)?;
            let profile_path = workspace.path.join("beatbox-python.sb");
            fs::write(&profile_path, profile).map_err(|source| EngineError::ReadSource {
                path: profile_path.display().to_string(),
                source,
            })?;

            let mut child = match Command::new("/usr/bin/sandbox-exec")
                .arg("-f")
                .arg(&profile_path)
                .arg(&python)
                .arg("-I")
                .arg("-B")
                .arg("-S")
                .arg("-")
                .current_dir(&workspace.path)
                .env_clear()
                .env("TMPDIR", &workspace.path)
                .env("PYTHONNOUSERSITE", "1")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
            {
                Ok(child) => child,
                Err(error) => {
                    return Ok(result(
                        &request,
                        ExecutionStatus::Denied,
                        serde_json::Value::Null,
                        Some(ErrorBody::new(
                            "python_spawn",
                            format!("failed to spawn sandboxed python: {error}"),
                        )),
                        Metrics {
                            wall_time_ms: elapsed_ms(started),
                            ..Metrics::default()
                        },
                        isolation,
                        inputs_digest,
                    ));
                }
            };

            let stdout = child
                .stdout
                .take()
                .map(|stdout| read_capped(stdout, request.policy.limits.output_bytes));
            let stderr = child
                .stderr
                .take()
                .map(|stderr| read_capped(stderr, request.policy.limits.output_bytes));
            let stdin = write_stdin_async(child.stdin.take(), source.as_bytes().to_vec());
            let outcome = wait_with_wall_limit(
                &mut child,
                request.policy.limits.wall_ms,
                cancellation,
                stdin,
                Some(WorkspaceDiskLimit {
                    path: &workspace.path,
                    max_bytes: request.policy.limits.disk_bytes,
                }),
            );
            let stdout = join_output(stdout);
            let stderr = join_output(stderr);
            let metrics = Metrics {
                wall_time_ms: elapsed_ms(started),
                cpu_time_ms: elapsed_ms(started),
                fuel_used: None,
                peak_memory_bytes: None,
            };

            let (status, exit_code, error) = match outcome {
                ChildOutcome::Timeout => (
                    ExecutionStatus::Timeout,
                    None,
                    Some(ErrorBody::new(
                        "wall_timeout",
                        format!(
                            "python_native exceeded wall time limit of {} ms",
                            request.policy.limits.wall_ms
                        ),
                    )),
                ),
                ChildOutcome::Canceled => (
                    ExecutionStatus::Killed,
                    None,
                    Some(ErrorBody::new("canceled", "execution canceled")),
                ),
                ChildOutcome::StdinError(error) => (
                    ExecutionStatus::Error,
                    None,
                    Some(ErrorBody::new(
                        "python_stdin",
                        format!("failed to deliver python source to sandboxed stdin: {error}"),
                    )),
                ),
                ChildOutcome::DiskLimitExceeded {
                    used_bytes,
                    max_bytes,
                } => (
                    ExecutionStatus::Killed,
                    None,
                    Some(ErrorBody::new(
                        "disk_limit",
                        format!(
                            "python_native workspace used {used_bytes} bytes, exceeding disk limit of {max_bytes} bytes"
                        ),
                    )),
                ),
                ChildOutcome::DiskLimitProbeError(error) => (
                    ExecutionStatus::Killed,
                    None,
                    Some(ErrorBody::new(
                        "disk_limit",
                        format!("failed to measure python_native workspace disk usage: {error}"),
                    )),
                ),
                ChildOutcome::Exited(status) if status.success() => {
                    (ExecutionStatus::Ok, status.code(), None)
                }
                ChildOutcome::Exited(status) => (
                    ExecutionStatus::Error,
                    status.code(),
                    Some(ErrorBody::new(
                        "python_exit",
                        format!("python exited with status {status}"),
                    )),
                ),
            };

            let mut result = result_with_stdio(
                &request,
                status,
                serde_json::Value::Null,
                stdout.text,
                stdout.truncated,
                stderr.text,
                stderr.truncated,
                error,
                metrics,
                isolation,
                inputs_digest,
            );
            result.exit_code = exit_code;
            Ok(result)
        }
    }

    pub fn python_available() -> bool {
        cfg!(target_os = "macos")
            && Path::new("/usr/bin/sandbox-exec").exists()
            && python_binary().is_some()
    }

    fn admit_python_native_policy(policy: &Policy) -> Result<(), EngineError> {
        let os = std::env::consts::OS.to_string();
        if !cfg!(target_os = "macos") {
            return Err(EngineError::PolicyUnenforceable {
                field: "lane",
                lane: Lane::PythonNative,
                os,
                reason: "python_native is only implemented as a macOS dev-grade Seatbelt lane"
                    .to_string(),
            });
        }
        if policy.fs.workspace.is_some() {
            return Err(EngineError::PolicyUnenforceable {
                field: "fs.workspace",
                lane: Lane::PythonNative,
                os,
                reason: "python_native currently creates a fresh private workspace per run"
                    .to_string(),
            });
        }
        if let Some(mount) = policy.fs.mounts.first() {
            let mode = match mount.mode {
                MountMode::Ro => "ro",
                MountMode::Rw => "rw",
            };
            return Err(EngineError::PolicyUnenforceable {
                field: "fs.mounts",
                lane: Lane::PythonNative,
                os,
                reason: format!(
                    "python_native does not expose host mounts yet; requested {mode} mount at {}",
                    mount.guest.display()
                ),
            });
        }
        if !matches!(policy.net, NetPolicy::Deny) {
            return Err(EngineError::PolicyUnenforceable {
                field: "net",
                lane: Lane::PythonNative,
                os,
                reason: "python_native exposes no network or proxy egress".to_string(),
            });
        }
        if !policy.env.is_empty() {
            return Err(EngineError::PolicyUnenforceable {
                field: "env",
                lane: Lane::PythonNative,
                os,
                reason: "python_native starts with an empty environment".to_string(),
            });
        }
        if !policy.secrets.is_empty() {
            return Err(EngineError::PolicyUnenforceable {
                field: "secrets",
                lane: Lane::PythonNative,
                os,
                reason: "python_native does not expose secrets".to_string(),
            });
        }
        if !matches!(policy.determinism, Determinism::Off) {
            return Err(EngineError::PolicyUnenforceable {
                field: "determinism",
                lane: Lane::PythonNative,
                os,
                reason: "python_native is not deterministic".to_string(),
            });
        }
        if policy.double_jail {
            return Err(EngineError::PolicyUnenforceable {
                field: "double_jail",
                lane: Lane::PythonNative,
                os,
                reason: "double_jail applies only to wasm lanes".to_string(),
            });
        }
        admit_python_native_unenforced_limits(policy, &os)?;
        Ok(())
    }

    fn admit_python_native_unenforced_limits(policy: &Policy, os: &str) -> Result<(), EngineError> {
        let default = Policy::default().limits;
        for (field, requested, default, reason) in [
            (
                "limits.cpu_ms",
                policy.limits.cpu_ms,
                default.cpu_ms,
                "python_native does not enforce CPU-time limits; use wall_ms for the watchdog",
            ),
            (
                "limits.memory_bytes",
                policy.limits.memory_bytes,
                default.memory_bytes,
                "python_native does not enforce process memory limits; max_python_source_bytes is the inline source cap",
            ),
            (
                "limits.pids",
                u64::from(policy.limits.pids),
                u64::from(default.pids),
                "python_native denies fork in Seatbelt but does not expose a configurable pid quota",
            ),
        ] {
            if requested != default {
                return Err(EngineError::PolicyUnenforceable {
                    field,
                    lane: Lane::PythonNative,
                    os: os.to_string(),
                    reason: reason.to_string(),
                });
            }
        }
        if policy.limits.fuel != default.fuel {
            return Err(EngineError::PolicyUnenforceable {
                field: "limits.fuel",
                lane: Lane::PythonNative,
                os: os.to_string(),
                reason: "fuel applies only to Wasmtime-backed lanes".to_string(),
            });
        }
        Ok(())
    }

    fn admit_python_native_request(request: &ExecuteRequest) -> Result<(), EngineError> {
        if !matches!(request.source, Source::Inline { .. }) {
            return Err(EngineError::InvalidSource {
                lane: Lane::PythonNative,
                reason: format!(
                    "python_native requires inline source, got {}",
                    source_kind(&request.source)
                ),
            });
        }
        if request.entrypoint.is_some() {
            return Err(EngineError::UnsupportedRequestField {
                field: "entrypoint",
                lane: Lane::PythonNative,
                reason:
                    "python_native runs inline source as a script and exposes no entrypoint ABI"
                        .to_string(),
            });
        }
        if !request.input.is_null() {
            return Err(EngineError::UnsupportedRequestField {
                field: "input",
                lane: Lane::PythonNative,
                reason: "python_native does not expose structured input yet".to_string(),
            });
        }
        if !request.stdin.is_empty() {
            return Err(EngineError::UnsupportedRequestField {
                field: "stdin",
                lane: Lane::PythonNative,
                reason: "python_native uses process stdin to deliver source code".to_string(),
            });
        }
        Ok(())
    }

    fn python_source(request: &ExecuteRequest, max_bytes: u64) -> Result<&str, EngineError> {
        match &request.source {
            Source::Inline { code } => {
                ensure_source_limit("source", bytes_len_u64(code.len()), max_bytes)?;
                Ok(code)
            }
            source => Err(EngineError::InvalidSource {
                lane: Lane::PythonNative,
                reason: format!(
                    "python_native requires inline source, got {}",
                    source_kind(source)
                ),
            }),
        }
    }

    fn python_source_byte_limit(policy_memory_bytes: u64) -> u64 {
        policy_memory_bytes.min(MAX_PYTHON_SOURCE_BYTES)
    }

    fn source_kind(source: &Source) -> &'static str {
        match source {
            Source::Inline { .. } => "inline",
            Source::WasmFile { .. } => "wasm_file",
            Source::WasmWat { .. } => "wasm_wat",
            Source::WasmBytesBase64 { .. } => "wasm_bytes_base64",
            Source::ModuleRef { .. } => "module_ref",
        }
    }

    fn digest_python_inputs(request: &ExecuteRequest) -> Result<String, EngineError> {
        use sha2::{Digest, Sha256};

        let source = match &request.source {
            Source::Inline { code } => code.as_bytes().to_vec(),
            _ => Vec::new(),
        };
        let source_digest = Sha256::digest(source);
        let canonical = serde_json::json!({
            "lane": &request.lane,
            "source_sha256": format!("sha256:{}", hex::encode(source_digest)),
            "entrypoint": &request.entrypoint,
            "input": &request.input,
            "stdin_sha256": format!("sha256:{}", hex::encode(Sha256::digest(request.stdin.as_bytes()))),
            "policy": &request.policy,
        });
        digest_json(&canonical).map_err(EngineError::from)
    }

    fn python_binary() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("BEATBOX_PYTHON")
            && !path.is_empty()
            && let Some(path) = trusted_python_binary(PathBuf::from(path))
        {
            return Some(path);
        }
        [
            "/opt/homebrew/bin/python3.11",
            "/opt/homebrew/bin/python3",
            "/usr/local/bin/python3",
            "/usr/bin/python3",
        ]
        .iter()
        .map(PathBuf::from)
        .find_map(trusted_python_binary)
    }

    pub(super) fn trusted_python_binary(path: PathBuf) -> Option<PathBuf> {
        let resolved = resolve_python_binary(path);
        if !python_binary_path_allowed(&resolved) {
            return None;
        }
        let metadata = fs::metadata(&resolved).ok()?;
        if metadata.is_file() {
            Some(resolved)
        } else {
            None
        }
    }

    fn resolve_python_binary(path: PathBuf) -> PathBuf {
        if path == Path::new("/usr/bin/python3") {
            let command_line_tools =
                Path::new("/Library/Developer/CommandLineTools/usr/bin/python3");
            if command_line_tools.exists() {
                return canonical_or_original(command_line_tools);
            }
        }
        canonical_or_original(&path)
    }

    pub(super) fn python_binary_path_allowed(path: &Path) -> bool {
        homebrew_python_runtime_path(path) || command_line_tools_python_runtime_path(path)
    }

    fn homebrew_python_runtime_path(path: &Path) -> bool {
        for root in [
            Path::new("/opt/homebrew/Cellar"),
            Path::new("/usr/local/Cellar"),
        ] {
            let Ok(rest) = path.strip_prefix(root) else {
                continue;
            };
            let Some((_, _, tail)) = homebrew_python_cellar_tail(rest) else {
                return false;
            };
            return python_runtime_tail_allowed(tail);
        }
        false
    }

    fn command_line_tools_python_runtime_path(path: &Path) -> bool {
        path == Path::new("/Library/Developer/CommandLineTools/usr/bin/python3")
            || path
                .strip_prefix(
                    "/Library/Developer/CommandLineTools/Library/Frameworks/Python3.framework/Versions",
                )
                .ok()
                .is_some_and(command_line_tools_python_tail_allowed)
    }

    fn command_line_tools_python_tail_allowed(path: &Path) -> bool {
        let Some(components) = path_components(path) else {
            return false;
        };
        let Some((version, tail)) = components.split_first() else {
            return false;
        };
        python_version_component_allowed(version) && python_runtime_tail_components_allowed(tail)
    }

    fn python_runtime_tail_allowed(path: &Path) -> bool {
        path_components(path)
            .is_some_and(|components| python_runtime_tail_components_allowed(&components))
    }

    fn python_runtime_tail_components_allowed(components: &[&str]) -> bool {
        match components {
            ["bin", executable] => python_executable_name_allowed(executable),
            ["Frameworks", "Python.framework", "Versions", version, "bin", executable] => {
                python_version_component_allowed(version)
                    && python_executable_name_allowed(executable)
            }
            ["Frameworks", "Python.framework", "Versions", version, "Resources", "Python.app", "Contents", "MacOS", "Python"] => {
                python_version_component_allowed(version)
            }
            ["Resources", "Python.app", "Contents", "MacOS", "Python"] => true,
            _ => false,
        }
    }

    fn path_components(path: &Path) -> Option<Vec<&str>> {
        path.components()
            .map(|component| component.as_os_str().to_str())
            .collect()
    }

    fn homebrew_python_cellar_tail(path: &Path) -> Option<(&str, &str, &Path)> {
        let mut components = path.components();
        let package = components.next()?.as_os_str().to_str()?;
        if !homebrew_python_package_allowed(package) {
            return None;
        }
        let version = components.next()?.as_os_str().to_str()?;
        if !homebrew_python_version_allowed(version) {
            return None;
        }
        Some((package, version, components.as_path()))
    }

    fn homebrew_python_cellar_version_root(path: &Path) -> bool {
        for root in [
            Path::new("/opt/homebrew/Cellar"),
            Path::new("/usr/local/Cellar"),
        ] {
            let Ok(rest) = path.strip_prefix(root) else {
                continue;
            };
            let Some((_, _, tail)) = homebrew_python_cellar_tail(rest) else {
                return false;
            };
            return tail.as_os_str().is_empty();
        }
        false
    }

    fn homebrew_python_package_allowed(package: &str) -> bool {
        package == "python"
            || package
                .strip_prefix("python@")
                .is_some_and(python_version_component_allowed)
    }

    fn homebrew_python_version_allowed(version: &str) -> bool {
        !version.is_empty()
            && version
                .chars()
                .all(|ch| ch.is_ascii_digit() || matches!(ch, '.' | '_'))
    }

    fn python_executable_name_allowed(name: &str) -> bool {
        name == "Python"
            || name == "python3"
            || name
                .strip_prefix("python3.")
                .is_some_and(|suffix| suffix.chars().all(|ch| ch.is_ascii_digit() || ch == '.'))
    }

    fn python_version_component_allowed(version: &str) -> bool {
        !version.is_empty() && version.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
    }

    pub(super) fn make_workspace() -> io::Result<TempWorkspace> {
        let base = std::env::temp_dir();
        for _ in 0..16 {
            let path = base.join(format!("beatbox-python-{}", uuid::Uuid::new_v4()));
            match create_private_workspace_dir(&path) {
                Ok(()) => return Ok(TempWorkspace { path }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "failed to allocate a unique beatbox python workspace",
        ))
    }

    #[cfg(unix)]
    fn create_private_workspace_dir(path: &Path) -> io::Result<()> {
        use std::os::unix::fs::DirBuilderExt;

        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder.create(path)
    }

    #[cfg(not(unix))]
    fn create_private_workspace_dir(path: &Path) -> io::Result<()> {
        fs::create_dir(path)
    }

    pub(super) fn sandbox_profile(workspace: &Path, python: &Path) -> Result<String, EngineError> {
        let workspace = canonical_or_original(workspace);
        let exec_filters = executable_paths(python)
            .into_iter()
            .map(|path| Ok(format!("    (literal \"{}\")", sandbox_string(&path)?)))
            .collect::<Result<Vec<_>, EngineError>>()?
            .join("\n");
        let read_filters = python_runtime_read_filters(python)
            .into_iter()
            .map(|filter| sandbox_path_filter(&filter))
            .collect::<Result<Vec<_>, EngineError>>()?
            .join("\n");
        Ok(format!(
            r#"(version 1)
(deny default)
(allow process-exec
  (require-any
{}))
(deny process-fork)
(allow file-read*
  (require-any
{}))
(allow file-read* file-write* (subpath "{}"))
(deny network*)
"#,
            exec_filters,
            read_filters,
            sandbox_string(&workspace)?,
        ))
    }

    fn executable_paths(python: &Path) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        push_unique_path(&mut paths, python.to_path_buf());
        let canonical = canonical_or_original(python);
        push_unique_path(&mut paths, canonical.clone());
        for path in apple_command_line_tools_python_binaries() {
            push_unique_path(&mut paths, path);
        }
        if let Some(framework) = framework_python_binary(&canonical)
            && framework.exists()
        {
            push_unique_path(&mut paths, framework);
        }
        paths
    }

    fn apple_command_line_tools_python_binaries() -> Vec<PathBuf> {
        let mut paths = Vec::new();
        push_unique_path(
            &mut paths,
            PathBuf::from("/Library/Developer/CommandLineTools/usr/bin/python3"),
        );
        let versions = Path::new(
            "/Library/Developer/CommandLineTools/Library/Frameworks/Python3.framework/Versions",
        );
        for path in command_line_tools_framework_python_binaries(versions) {
            push_unique_path(&mut paths, path);
        }
        paths.into_iter().filter(|path| path.exists()).collect()
    }

    pub(super) fn command_line_tools_framework_python_binaries(versions: &Path) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if let Ok(entries) = fs::read_dir(versions) {
            for entry in entries.flatten() {
                let version = entry.path();
                let Some(name) = version.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                if !python_version_component_allowed(name) {
                    continue;
                }
                push_unique_path(&mut paths, version.join("bin").join("python3"));
                push_unique_path(
                    &mut paths,
                    version.join("bin").join(format!("python{name}")),
                );
                push_unique_path(
                    &mut paths,
                    version
                        .join("Resources")
                        .join("Python.app")
                        .join("Contents")
                        .join("MacOS")
                        .join("Python"),
                );
            }
        }
        paths.into_iter().filter(|path| path.exists()).collect()
    }

    pub(super) fn framework_python_binary(canonical_python: &Path) -> Option<PathBuf> {
        let version_dir = canonical_python.parent()?.parent()?;
        let framework = version_dir
            .join("Resources")
            .join("Python.app")
            .join("Contents")
            .join("MacOS")
            .join("Python");
        python_binary_path_allowed(&framework).then_some(framework)
    }

    fn python_runtime_read_filters(python: &Path) -> Vec<SandboxFilter> {
        let mut filters = Vec::new();
        for path in executable_paths(python) {
            push_existing_read_filter(&mut filters, path);
        }
        for path in python_install_roots(python) {
            push_existing_read_filter(&mut filters, path);
        }
        for path in [
            "/usr/lib",
            "/System/Library",
            "/System/Cryptexes/OS/System/Library",
            "/System/Volumes/Preboot/Cryptexes/OS/System/Library",
            "/System/Volumes/Preboot/Cryptexes/OS/usr/lib",
            "/Library/Developer/CommandLineTools/Library/Frameworks/Python3.framework",
            "/Library/Developer/CommandLineTools/usr/lib",
        ] {
            push_existing_read_filter(&mut filters, PathBuf::from(path));
        }
        filters
    }

    pub(super) fn python_install_roots(python: &Path) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        let canonical = canonical_or_original(python);
        for path in [python.to_path_buf(), canonical] {
            push_python_prefix_roots(&mut roots, &path);
        }
        roots
    }

    fn push_python_prefix_roots(roots: &mut Vec<PathBuf>, python: &Path) {
        for ancestor in python.ancestors() {
            if ancestor
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| matches!(name, "Python.framework" | "Python3.framework"))
            {
                push_unique_path(roots, ancestor.to_path_buf());
            }
            if ancestor
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == "Versions")
            {
                push_unique_path(roots, ancestor.to_path_buf());
            }
        }
        if let Some(parent) = python.parent()
            && parent.file_name().and_then(|name| name.to_str()) == Some("bin")
            && let Some(prefix) = parent.parent()
            && python_runtime_prefix_allowed(prefix)
        {
            push_unique_path(roots, prefix.to_path_buf());
        }
    }

    fn python_runtime_prefix_allowed(prefix: &Path) -> bool {
        homebrew_python_cellar_version_root(prefix)
            || command_line_tools_python_version_root(prefix)
    }

    fn command_line_tools_python_version_root(path: &Path) -> bool {
        let Ok(rest) = path.strip_prefix(
            "/Library/Developer/CommandLineTools/Library/Frameworks/Python3.framework/Versions",
        ) else {
            return false;
        };
        let Some(components) = path_components(rest) else {
            return false;
        };
        matches!(
            components.as_slice(),
            [version] if python_version_component_allowed(version)
        )
    }

    fn push_existing_read_filter(filters: &mut Vec<SandboxFilter>, path: PathBuf) {
        if path.exists() {
            push_read_filter(filters, path.clone());
            push_read_filter(filters, canonical_or_original(&path));
        }
    }

    fn push_read_filter(filters: &mut Vec<SandboxFilter>, path: PathBuf) {
        let filter = if path.is_dir() {
            SandboxFilter::Subpath(path.clone())
        } else {
            SandboxFilter::Literal(path.clone())
        };
        push_unique_filter(filters, filter);
        for ancestor in path.ancestors().skip(1) {
            push_unique_filter(filters, SandboxFilter::Literal(ancestor.to_path_buf()));
        }
    }

    fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
        if !paths.iter().any(|existing| existing == &path) {
            paths.push(path);
        }
    }

    fn push_unique_filter(filters: &mut Vec<SandboxFilter>, filter: SandboxFilter) {
        if !filters
            .iter()
            .any(|existing| sandbox_filter_eq(existing, &filter))
        {
            filters.push(filter);
        }
    }

    fn sandbox_filter_eq(left: &SandboxFilter, right: &SandboxFilter) -> bool {
        match (left, right) {
            (SandboxFilter::Literal(left), SandboxFilter::Literal(right))
            | (SandboxFilter::Subpath(left), SandboxFilter::Subpath(right)) => left == right,
            _ => false,
        }
    }

    fn canonical_or_original(path: &Path) -> PathBuf {
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
    }

    fn sandbox_string(path: &Path) -> Result<String, EngineError> {
        let Some(path) = path.to_str() else {
            return Err(EngineError::SandboxProfile {
                reason: "sandbox profile path is not valid UTF-8".to_string(),
            });
        };
        if let Some(control) = path.chars().find(|ch| ch.is_control()) {
            return Err(EngineError::SandboxProfile {
                reason: format!(
                    "sandbox profile path contains control character U+{:04X}",
                    u32::from(control)
                ),
            });
        }
        Ok(path.replace('\\', "\\\\").replace('"', "\\\""))
    }

    fn sandbox_path_filter(filter: &SandboxFilter) -> Result<String, EngineError> {
        match filter {
            SandboxFilter::Literal(path) => {
                Ok(format!("    (literal \"{}\")", sandbox_string(path)?))
            }
            SandboxFilter::Subpath(path) => {
                Ok(format!("    (subpath \"{}\")", sandbox_string(path)?))
            }
        }
    }

    pub(super) fn write_stdin_async(
        stdin: Option<ChildStdin>,
        bytes: Vec<u8>,
    ) -> thread::JoinHandle<io::Result<()>> {
        thread::spawn(move || {
            let Some(mut stdin) = stdin else {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "child stdin was not available",
                ));
            };
            stdin.write_all(&bytes)
        })
    }

    pub(super) fn wait_with_wall_limit(
        child: &mut Child,
        wall_ms: u64,
        cancellation: Option<CancellationToken>,
        stdin: thread::JoinHandle<io::Result<()>>,
        disk_limit: Option<WorkspaceDiskLimit<'_>>,
    ) -> ChildOutcome {
        let deadline = Instant::now() + Duration::from_millis(wall_ms.max(1));
        let mut stdin = Some(stdin);
        let mut next_disk_check = Instant::now();
        loop {
            if stdin.as_ref().is_some_and(thread::JoinHandle::is_finished)
                && let Some(handle) = stdin.take()
            {
                match join_stdin_writer(handle) {
                    Ok(()) => {}
                    Err(error) => {
                        kill_child_and_join_stdin(child, &mut stdin);
                        return ChildOutcome::StdinError(error);
                    }
                }
            }
            if let Some(limit) = disk_limit.as_ref()
                && Instant::now() >= next_disk_check
            {
                if let Some(outcome) = check_workspace_disk_limit(limit) {
                    kill_child_and_join_stdin(child, &mut stdin);
                    return outcome;
                }
                next_disk_check =
                    Instant::now() + Duration::from_millis(PYTHON_DISK_CHECK_INTERVAL_MS);
            }
            if cancellation
                .as_ref()
                .is_some_and(CancellationToken::is_canceled)
            {
                kill_child_and_join_stdin(child, &mut stdin);
                return ChildOutcome::Canceled;
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    if let Some(stdin) = stdin.take()
                        && let Err(error) = join_stdin_writer(stdin)
                    {
                        return ChildOutcome::StdinError(error);
                    }
                    if let Some(limit) = disk_limit.as_ref()
                        && let Some(outcome) = check_workspace_disk_limit(limit)
                    {
                        return outcome;
                    }
                    return ChildOutcome::Exited(status);
                }
                Ok(None) if Instant::now() >= deadline => {
                    kill_child_and_join_stdin(child, &mut stdin);
                    return ChildOutcome::Timeout;
                }
                Ok(None) => thread::sleep(Duration::from_millis(10)),
                Err(_) => {
                    kill_child_and_join_stdin(child, &mut stdin);
                    return ChildOutcome::Timeout;
                }
            }
        }
    }

    fn kill_child_and_join_stdin(
        child: &mut Child,
        stdin: &mut Option<thread::JoinHandle<io::Result<()>>>,
    ) {
        let _ = child.kill();
        let _ = child.wait();
        if let Some(stdin) = stdin.take() {
            let _ = join_stdin_writer(stdin);
        }
    }

    fn check_workspace_disk_limit(limit: &WorkspaceDiskLimit<'_>) -> Option<ChildOutcome> {
        match workspace_disk_usage(limit.path, limit.max_bytes) {
            Ok(used_bytes) if used_bytes > limit.max_bytes => {
                Some(ChildOutcome::DiskLimitExceeded {
                    used_bytes,
                    max_bytes: limit.max_bytes,
                })
            }
            Ok(_) => None,
            Err(error) => Some(ChildOutcome::DiskLimitProbeError(error.to_string())),
        }
    }

    pub(super) fn workspace_disk_usage(path: &Path, max_bytes: u64) -> io::Result<u64> {
        let mut used_bytes = 0_u64;
        accumulate_workspace_disk_usage(path, max_bytes, &mut used_bytes)?;
        Ok(used_bytes)
    }

    fn accumulate_workspace_disk_usage(
        path: &Path,
        max_bytes: u64,
        used_bytes: &mut u64,
    ) -> io::Result<()> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        *used_bytes = used_bytes.saturating_add(metadata.len());
        if *used_bytes > max_bytes || !metadata.file_type().is_dir() {
            return Ok(());
        }

        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            accumulate_workspace_disk_usage(&entry.path(), max_bytes, used_bytes)?;
            if *used_bytes > max_bytes {
                break;
            }
        }
        Ok(())
    }

    fn join_stdin_writer(stdin: thread::JoinHandle<io::Result<()>>) -> Result<(), String> {
        match stdin.join() {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(error.to_string()),
            Err(_) => Err("stdin writer panicked".to_string()),
        }
    }

    fn read_capped<R>(mut reader: R, max_bytes: u64) -> thread::JoinHandle<CapturedOutput>
    where
        R: Read + Send + 'static,
    {
        thread::spawn(move || {
            let max = usize::try_from(max_bytes).unwrap_or(usize::MAX);
            let mut captured = Vec::new();
            let mut truncated = false;
            let mut buffer = [0_u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        if captured.len() < max {
                            let remaining = max - captured.len();
                            let take = remaining.min(read);
                            captured.extend_from_slice(&buffer[..take]);
                            if take < read {
                                truncated = true;
                            }
                        } else if read > 0 {
                            truncated = true;
                        }
                    }
                    Err(_) => break,
                }
            }
            CapturedOutput {
                text: String::from_utf8_lossy(&captured).to_string(),
                truncated,
            }
        })
    }

    fn join_output(handle: Option<thread::JoinHandle<CapturedOutput>>) -> CapturedOutput {
        handle
            .and_then(|handle| handle.join().ok())
            .unwrap_or(CapturedOutput {
                text: String::new(),
                truncated: false,
            })
    }

    fn python_isolation() -> EffectiveIsolation {
        EffectiveIsolation::for_current_os(
            vec![
                "sandbox-exec".to_string(),
                "seatbelt-profile".to_string(),
                "env-clear".to_string(),
                "trusted-python-binary".to_string(),
                "runtime-read-allowlist".to_string(),
                "network-deny".to_string(),
                "process-fork-deny".to_string(),
                "mach-lookup-deny".to_string(),
                "sysctl-read-deny".to_string(),
                "wall-time-watchdog".to_string(),
                "source-byte-limit".to_string(),
                "stdin-delivery-watchdog".to_string(),
                "workspace-disk-quota".to_string(),
                "output-cap".to_string(),
            ],
            vec![
                "macos_native_lane_dev_grade".to_string(),
                "memory_limit_not_enforced".to_string(),
                "cpu_limit_not_enforced".to_string(),
            ],
        )
    }
}

#[cfg(feature = "lane-wasi")]
mod wasm {
    use std::io::Read;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;

    use base64::Engine as _;
    use beatbox_core::{MountMode, Policy};
    use wasmtime::{Config, Engine, Linker, Module, ResourceLimiter, Store, UpdateDeadline};

    use super::*;

    #[derive(Clone)]
    pub struct WasmLane {
        engine: Engine,
        _clock: Arc<EpochClock>,
    }

    const EPOCH_TICK_MS: u64 = 10;

    struct EpochClock {
        stop: Arc<AtomicBool>,
        handle: Mutex<Option<thread::JoinHandle<()>>>,
    }

    impl EpochClock {
        fn start(engine: Engine) -> Arc<Self> {
            let stop = Arc::new(AtomicBool::new(false));
            let thread_stop = Arc::clone(&stop);
            let handle = thread::spawn(move || {
                let tick = Duration::from_millis(EPOCH_TICK_MS);
                while !thread_stop.load(Ordering::SeqCst) {
                    thread::sleep(tick);
                    engine.increment_epoch();
                }
            });
            Arc::new(Self {
                stop,
                handle: Mutex::new(Some(handle)),
            })
        }
    }

    impl Drop for EpochClock {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Ok(mut handle) = self.handle.lock()
                && let Some(handle) = handle.take()
            {
                let _ = handle.join();
            }
        }
    }

    struct WasmState {
        limits: WasmStoreLimits,
    }

    struct WasmStoreLimits {
        max_store_bytes: usize,
        linear_memory_bytes: usize,
        table_bytes: usize,
        instances: usize,
        memories: usize,
        tables: usize,
    }

    impl ResourceLimiter for WasmStoreLimits {
        fn memory_growing(
            &mut self,
            _current: usize,
            desired: usize,
            _maximum: Option<usize>,
        ) -> wasmtime::Result<bool> {
            self.ensure_store_budget(
                desired,
                self.table_bytes,
                format_args!("linear memory desired {desired} bytes"),
            )?;
            self.linear_memory_bytes = desired;
            Ok(true)
        }

        fn table_growing(
            &mut self,
            current: usize,
            desired: usize,
            _maximum: Option<usize>,
        ) -> wasmtime::Result<bool> {
            let current_bytes = table_element_bytes(current)?;
            let desired_bytes = table_element_bytes(desired)?;
            let delta = desired_bytes.checked_sub(current_bytes).ok_or_else(|| {
                wasmtime::format_err!(
                    "beatbox memory limit exceeded: table desired {desired} elements is below current {current} elements"
                )
            })?;
            let table_bytes = self.table_bytes.checked_add(delta).ok_or_else(|| {
                wasmtime::format_err!(
                    "beatbox memory limit exceeded: table desired {desired} elements overflows aggregate host byte accounting"
                )
            })?;
            let element_size = std::mem::size_of::<usize>();
            self.ensure_store_budget(self.linear_memory_bytes, table_bytes, format_args!(
                "table desired {desired} elements ({desired_bytes} bytes at {element_size} bytes/element)"
            ))?;
            self.table_bytes = table_bytes;
            Ok(true)
        }

        fn instances(&self) -> usize {
            self.instances
        }

        fn memories(&self) -> usize {
            self.memories
        }

        fn tables(&self) -> usize {
            self.tables
        }
    }

    impl WasmStoreLimits {
        fn ensure_store_budget(
            &self,
            linear_memory_bytes: usize,
            table_bytes: usize,
            attempted: std::fmt::Arguments<'_>,
        ) -> wasmtime::Result<()> {
            let attempted = attempted.to_string();
            let total = linear_memory_bytes
                .checked_add(table_bytes)
                .ok_or_else(|| {
                    wasmtime::format_err!(
                        "beatbox memory limit exceeded: {attempted} overflows aggregate host byte accounting"
                    )
                })?;
            if total > self.max_store_bytes {
                Err(wasmtime::format_err!(
                    "beatbox memory limit exceeded: {attempted} would use {total} bytes across linear memory and tables, exceeding policy limit {} bytes",
                    self.max_store_bytes
                ))
            } else {
                Ok(())
            }
        }
    }

    fn table_element_bytes(elements: usize) -> wasmtime::Result<usize> {
        let element_size = std::mem::size_of::<usize>();
        elements.checked_mul(element_size).ok_or_else(|| {
            wasmtime::format_err!(
                "beatbox memory limit exceeded: table desired {elements} elements overflows host byte accounting"
            )
        })
    }

    impl WasmLane {
        pub fn new() -> Result<Self, EngineError> {
            let mut config = Config::new();
            config.wasm_component_model(true);
            config.consume_fuel(true);
            config.epoch_interruption(true);
            config.cranelift_nan_canonicalization(true);
            config.relaxed_simd_deterministic(true);
            let engine = Engine::new(&config)
                .map_err(|error| EngineError::WasmtimeEngine(error.to_string()))?;
            let clock = EpochClock::start(engine.clone());
            Ok(Self {
                engine,
                _clock: clock,
            })
        }

        pub fn execute(
            &self,
            request: ExecuteRequest,
            cancellation: Option<CancellationToken>,
        ) -> Result<ExecutionResult, EngineError> {
            admit_wasm_policy(&request.policy)?;
            admit_wasm_request(&request)?;
            let started = Instant::now();
            let module_bytes = load_wasm_source(
                &request.source,
                wasm_module_byte_limit(request.policy.limits.memory_bytes),
            )?;
            let inputs_digest = digest_wasm_inputs(&request, &module_bytes)?;
            let isolation = wasm_isolation();

            let imports = match module_imports_from_bytes(&module_bytes) {
                Ok(imports) => imports,
                Err(error) => {
                    return Ok(result(
                        &request,
                        ExecutionStatus::Error,
                        serde_json::Value::Null,
                        Some(ErrorBody::new("wasm_compile", error)),
                        Metrics {
                            wall_time_ms: elapsed_ms(started),
                            ..Metrics::default()
                        },
                        isolation,
                        inputs_digest,
                    ));
                }
            };
            if !imports.is_empty() {
                return Ok(result(
                    &request,
                    ExecutionStatus::Denied,
                    serde_json::Value::Null,
                    Some(ErrorBody::new(
                        "host_import_denied",
                        format!("component imports are disabled: {}", imports.join(", ")),
                    )),
                    Metrics {
                        wall_time_ms: elapsed_ms(started),
                        ..Metrics::default()
                    },
                    isolation,
                    inputs_digest,
                ));
            }

            let module = match Module::new(&self.engine, &module_bytes) {
                Ok(module) => module,
                Err(error) => {
                    return Ok(result(
                        &request,
                        ExecutionStatus::Error,
                        serde_json::Value::Null,
                        Some(ErrorBody::new("wasm_compile", error.to_string())),
                        Metrics {
                            wall_time_ms: elapsed_ms(started),
                            ..Metrics::default()
                        },
                        isolation,
                        inputs_digest,
                    ));
                }
            };

            let imports = module_imports(&module);
            if !imports.is_empty() {
                return Ok(result(
                    &request,
                    ExecutionStatus::Denied,
                    serde_json::Value::Null,
                    Some(ErrorBody::new(
                        "host_import_denied",
                        format!("component imports are disabled: {}", imports.join(", ")),
                    )),
                    Metrics {
                        wall_time_ms: elapsed_ms(started),
                        ..Metrics::default()
                    },
                    isolation,
                    inputs_digest,
                ));
            }

            let memory_limit =
                usize::try_from(request.policy.limits.memory_bytes).unwrap_or(usize::MAX);
            let mut store = Store::new(
                &self.engine,
                WasmState {
                    limits: WasmStoreLimits {
                        max_store_bytes: memory_limit,
                        linear_memory_bytes: 0,
                        table_bytes: 0,
                        instances: 1,
                        memories: 1,
                        tables: 4,
                    },
                },
            );
            store.limiter(|state| &mut state.limits);
            let requested_fuel = request.policy.limits.fuel.unwrap_or(10_000_000);
            if let Err(error) = store.set_fuel(requested_fuel) {
                return Ok(result(
                    &request,
                    ExecutionStatus::Error,
                    serde_json::Value::Null,
                    Some(ErrorBody::new("fuel_setup", error.to_string())),
                    Metrics {
                        wall_time_ms: elapsed_ms(started),
                        ..Metrics::default()
                    },
                    isolation,
                    inputs_digest,
                ));
            }

            configure_epoch_deadline(
                &mut store,
                request.policy.limits.wall_ms,
                cancellation.clone(),
            );
            let linker = Linker::new(&self.engine);
            let value = run_entrypoint(&mut store, &linker, &module, &request);

            let remaining_fuel = store.get_fuel().ok();
            let fuel_exhausted = remaining_fuel == Some(0);
            let fuel_used =
                remaining_fuel.map(|remaining| requested_fuel.saturating_sub(remaining));
            let metrics = Metrics {
                wall_time_ms: elapsed_ms(started),
                cpu_time_ms: elapsed_ms(started),
                fuel_used,
                peak_memory_bytes: None,
            };

            match value {
                Ok(value) => Ok(result(
                    &request,
                    ExecutionStatus::Ok,
                    value,
                    None,
                    metrics,
                    isolation,
                    inputs_digest,
                )),
                Err(error) => {
                    let canceled = cancellation
                        .as_ref()
                        .is_some_and(CancellationToken::is_canceled);
                    let (status, code, message) = if canceled {
                        (
                            ExecutionStatus::Killed,
                            "canceled",
                            "execution canceled".to_string(),
                        )
                    } else if fuel_exhausted {
                        (ExecutionStatus::Timeout, "fuel_exhausted", error.message)
                    } else {
                        let (status, code) = classify_wasm_error(&error);
                        (status, code, error.message)
                    };
                    Ok(result(
                        &request,
                        status,
                        serde_json::Value::Null,
                        Some(ErrorBody::new(code, message)),
                        metrics,
                        isolation,
                        inputs_digest,
                    ))
                }
            }
        }
    }

    fn admit_wasm_policy(policy: &Policy) -> Result<(), EngineError> {
        let os = std::env::consts::OS.to_string();
        if policy.fs.workspace.is_some() {
            return Err(EngineError::PolicyUnenforceable {
                field: "fs.workspace",
                lane: Lane::Wasm,
                os,
                reason: "the initial wasm lane is W0 hermetic and exposes no filesystem"
                    .to_string(),
            });
        }
        if let Some(mount) = policy.fs.mounts.first() {
            let mode = match mount.mode {
                MountMode::Ro => "ro",
                MountMode::Rw => "rw",
            };
            return Err(EngineError::PolicyUnenforceable {
                field: "fs.mounts",
                lane: Lane::Wasm,
                os,
                reason: format!(
                    "the initial wasm lane exposes no mounts; requested {mode} mount at {}",
                    mount.guest.display()
                ),
            });
        }
        if !matches!(policy.net, NetPolicy::Deny) {
            return Err(EngineError::PolicyUnenforceable {
                field: "net",
                lane: Lane::Wasm,
                os,
                reason: "raw network and proxy egress are not exposed in W0".to_string(),
            });
        }
        if !policy.env.is_empty() {
            return Err(EngineError::PolicyUnenforceable {
                field: "env",
                lane: Lane::Wasm,
                os,
                reason: "the initial wasm lane exposes no environment".to_string(),
            });
        }
        if !policy.secrets.is_empty() {
            return Err(EngineError::PolicyUnenforceable {
                field: "secrets",
                lane: Lane::Wasm,
                os,
                reason: "the initial wasm lane exposes no secrets".to_string(),
            });
        }
        if policy.double_jail {
            return Err(EngineError::PolicyUnenforceable {
                field: "double_jail",
                lane: Lane::Wasm,
                os,
                reason: "the initial wasm lane cannot add a second OS or VM isolation boundary"
                    .to_string(),
            });
        }
        Ok(())
    }

    fn admit_wasm_request(request: &ExecuteRequest) -> Result<(), EngineError> {
        if !request.stdin.is_empty() {
            return Err(EngineError::UnsupportedRequestField {
                field: "stdin",
                lane: Lane::Wasm,
                reason: "the initial wasm lane exposes no stdin; use input for scalar entrypoint arguments until WASI command support lands".to_string(),
            });
        }
        Ok(())
    }

    fn load_wasm_source(source: &Source, max_bytes: u64) -> Result<Vec<u8>, EngineError> {
        match source {
            Source::Inline { code } | Source::WasmWat { text: code } => parse_wat_source(
                "source",
                code.as_bytes(),
                || wat::parse_str(code).map_err(|error| EngineError::ParseWat(error.to_string())),
                max_bytes,
            ),
            Source::WasmFile { path } => {
                let bytes = read_capped_source_file(path, max_bytes)?;
                if path.extension().and_then(|ext| ext.to_str()) == Some("wat") {
                    parse_wat_source(
                        "source",
                        &bytes,
                        || {
                            wat::parse_bytes(&bytes)
                                .map(|cow| cow.into_owned())
                                .map_err(|error| EngineError::ParseWat(error.to_string()))
                        },
                        max_bytes,
                    )
                } else {
                    ensure_source_limit("module", bytes_len_u64(bytes.len()), max_bytes)?;
                    Ok(bytes)
                }
            }
            Source::WasmBytesBase64 { bytes } => {
                ensure_decoded_source_estimate(
                    "module",
                    bytes_len_u64(base64::decoded_len_estimate(bytes.len())),
                    max_bytes,
                )?;
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(bytes)
                    .map_err(|error| EngineError::DecodeBase64(error.to_string()))?;
                ensure_source_limit("module", bytes_len_u64(decoded.len()), max_bytes)?;
                Ok(decoded)
            }
            Source::ModuleRef { .. } => Err(EngineError::InvalidSource {
                lane: Lane::Wasm,
                reason: "module_ref storage is planned for M2.5 and is not implemented yet"
                    .to_string(),
            }),
        }
    }

    fn read_capped_source_file(path: &Path, max_bytes: u64) -> Result<Vec<u8>, EngineError> {
        let file = std::fs::File::open(path).map_err(|source| EngineError::ReadSource {
            path: path.display().to_string(),
            source,
        })?;
        let metadata = file.metadata().map_err(|source| EngineError::ReadSource {
            path: path.display().to_string(),
            source,
        })?;
        if !metadata.file_type().is_file() {
            return Err(EngineError::ReadSource {
                path: path.display().to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "source path is not a regular file",
                ),
            });
        }
        ensure_source_limit("source", metadata.len(), max_bytes)?;
        read_capped_source(file, max_bytes)
    }

    pub(super) fn read_capped_source<R: Read>(
        reader: R,
        max_bytes: u64,
    ) -> Result<Vec<u8>, EngineError> {
        let mut bytes = Vec::new();
        let mut limited = reader.take(max_bytes.saturating_add(1));
        limited
            .read_to_end(&mut bytes)
            .map_err(|source| EngineError::ReadSource {
                path: "<stream>".to_string(),
                source,
            })?;
        ensure_source_limit("source", bytes_len_u64(bytes.len()), max_bytes)?;
        Ok(bytes)
    }

    fn parse_wat_source<F>(
        field: &'static str,
        source_bytes: &[u8],
        parse: F,
        max_bytes: u64,
    ) -> Result<Vec<u8>, EngineError>
    where
        F: FnOnce() -> Result<Vec<u8>, EngineError>,
    {
        ensure_source_limit(field, bytes_len_u64(source_bytes.len()), max_bytes)?;
        let module = parse()?;
        ensure_source_limit("module", bytes_len_u64(module.len()), max_bytes)?;
        Ok(module)
    }

    fn ensure_decoded_source_estimate(
        field: &'static str,
        estimate: u64,
        limit: u64,
    ) -> Result<(), EngineError> {
        if estimate > limit {
            Err(EngineError::SourceEstimateTooLarge {
                field,
                estimate,
                limit,
            })
        } else {
            Ok(())
        }
    }

    fn wasm_module_byte_limit(policy_memory_bytes: u64) -> u64 {
        policy_memory_bytes.min(MAX_WASM_MODULE_BYTES)
    }

    fn digest_wasm_inputs(
        request: &ExecuteRequest,
        module_bytes: &[u8],
    ) -> Result<String, EngineError> {
        use sha2::{Digest, Sha256};

        let source_digest = Sha256::digest(module_bytes);
        let canonical = serde_json::json!({
            "lane": &request.lane,
            "source_sha256": format!("sha256:{}", hex::encode(source_digest)),
            "entrypoint": &request.entrypoint,
            "input": &request.input,
            "stdin_sha256": format!("sha256:{}", hex::encode(Sha256::digest(request.stdin.as_bytes()))),
            "policy": &request.policy,
        });
        digest_json(&canonical).map_err(EngineError::from)
    }

    fn wasm_isolation() -> EffectiveIsolation {
        EffectiveIsolation::for_current_os(
            vec![
                "wasmtime".to_string(),
                "empty-linker".to_string(),
                "host-import-deny".to_string(),
                "precompile-import-scan".to_string(),
                "fuel".to_string(),
                "epoch-interruption".to_string(),
                "store-limits".to_string(),
            ],
            Vec::new(),
        )
    }

    fn module_imports_from_bytes(module_bytes: &[u8]) -> Result<Vec<String>, String> {
        for payload in wasmparser::Parser::new(0).parse_all(module_bytes) {
            match payload.map_err(|error| error.to_string())? {
                wasmparser::Payload::ImportSection(imports) => {
                    return imports
                        .into_imports()
                        .map(|import| {
                            import
                                .map(|import| format!("{}::{}", import.module, import.name))
                                .map_err(|error| error.to_string())
                        })
                        .collect();
                }
                wasmparser::Payload::ComponentImportSection(imports) => {
                    return imports
                        .into_iter()
                        .map(|import| {
                            import
                                .map(|import| format!("component::{}", import.name.0))
                                .map_err(|error| error.to_string())
                        })
                        .collect();
                }
                _ => {}
            }
        }
        Ok(Vec::new())
    }

    fn module_imports(module: &Module) -> Vec<String> {
        module
            .imports()
            .map(|import| format!("{}::{}", import.module(), import.name()))
            .collect()
    }

    fn run_entrypoint(
        store: &mut Store<WasmState>,
        linker: &Linker<WasmState>,
        module: &Module,
        request: &ExecuteRequest,
    ) -> Result<serde_json::Value, WasmFailure> {
        let instance = linker
            .instantiate(&mut *store, module)
            .map_err(WasmFailure::runtime)?;
        let entrypoint = request.entrypoint.as_deref().unwrap_or("run");

        if let Ok(func) = instance.get_typed_func::<i64, i64>(&mut *store, entrypoint) {
            let input = input_i64(&request.input).map_err(WasmFailure::guest)?;
            let value = func
                .call(&mut *store, input)
                .map_err(WasmFailure::runtime)?;
            return Ok(serde_json::json!(value));
        }

        if let Ok(func) = instance.get_typed_func::<(), i64>(&mut *store, entrypoint) {
            let value = func.call(&mut *store, ()).map_err(WasmFailure::runtime)?;
            return Ok(serde_json::json!(value));
        }

        if let Ok(func) = instance.get_typed_func::<(), ()>(&mut *store, entrypoint) {
            func.call(&mut *store, ()).map_err(WasmFailure::runtime)?;
            return Ok(serde_json::Value::Null);
        }

        Err(WasmFailure::guest(format!(
            "missing supported entrypoint `{entrypoint}`; expected ()->(), ()->i64, or i64->i64"
        )))
    }

    fn input_i64(input: &serde_json::Value) -> Result<i64, String> {
        if input.is_null() {
            return Ok(0);
        }
        if let Some(value) = input.as_i64() {
            return Ok(value);
        }
        if let Some(value) = input.get("n").and_then(serde_json::Value::as_i64) {
            return Ok(value);
        }
        Err("wasm i64 entrypoints require input as an integer or {\"n\": integer}".to_string())
    }

    enum WasmFailureSource {
        Guest,
        Runtime,
    }

    struct WasmFailure {
        message: String,
        source: WasmFailureSource,
    }

    impl WasmFailure {
        fn guest(message: impl Into<String>) -> Self {
            Self {
                message: message.into(),
                source: WasmFailureSource::Guest,
            }
        }

        fn runtime(error: wasmtime::Error) -> Self {
            Self {
                message: error_chain_message(error),
                source: WasmFailureSource::Runtime,
            }
        }
    }

    fn error_chain_message(error: wasmtime::Error) -> String {
        let mut message = error.to_string();
        for cause in error.chain().skip(1) {
            message.push_str("\ncaused by: ");
            message.push_str(&cause.to_string());
        }
        message
    }

    fn classify_wasm_error(error: &WasmFailure) -> (ExecutionStatus, &'static str) {
        let lower = error.message.to_ascii_lowercase();
        if lower.contains("fuel") {
            (ExecutionStatus::Timeout, "fuel_exhausted")
        } else if lower.contains("epoch") || lower.contains("interrupt") {
            (ExecutionStatus::Timeout, "wall_timeout")
        } else if matches!(error.source, WasmFailureSource::Runtime)
            && is_memory_limit_error(&lower)
        {
            (ExecutionStatus::Oom, "memory_limit")
        } else {
            (ExecutionStatus::Error, "wasm_trap")
        }
    }

    fn is_memory_limit_error(lower: &str) -> bool {
        lower.contains("forcing trap when growing memory")
            || lower.contains("forcing a memory growth failure to be a trap")
            || lower.contains("failed to grow memory")
            || lower.contains("memory limit exceeded")
            || lower.contains("exceeds memory limits")
            || lower.contains("exceeds memory limit")
            || lower.contains("out of memory")
            || lower.contains("memory allocation")
    }

    fn epoch_deadline_ticks(wall_ms: u64) -> u64 {
        wall_ms.max(1).div_ceil(EPOCH_TICK_MS).max(1)
    }

    fn configure_epoch_deadline(
        store: &mut Store<WasmState>,
        wall_ms: u64,
        cancellation: Option<CancellationToken>,
    ) {
        let started = Instant::now();
        let wall_limit = Duration::from_millis(wall_ms.max(1));
        store.epoch_deadline_callback(move |_| {
            if cancellation
                .as_ref()
                .is_some_and(CancellationToken::is_canceled)
                || started.elapsed() >= wall_limit
            {
                Ok(UpdateDeadline::Interrupt)
            } else {
                Ok(UpdateDeadline::Continue(1))
            }
        });
        store.set_epoch_deadline(epoch_deadline_ticks(EPOCH_TICK_MS));
    }

    #[allow(dead_code)]
    fn source_path(source: &Source) -> Option<&Path> {
        match source {
            Source::WasmFile { path } => Some(path.as_path()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use beatbox_core::{Determinism, FsPolicy, Mount, MountMode, Policy, Secret, SecretExpose};

    fn request_for(wat: &str, input: serde_json::Value) -> ExecuteRequest {
        ExecuteRequest {
            lane: Lane::Wasm,
            source: Source::WasmWat {
                text: wat.to_string(),
            },
            entrypoint: None,
            input,
            stdin: String::new(),
            policy: Policy::default(),
            idempotency_key: None,
        }
    }

    #[test]
    fn wasm_run_i64_returns_value() -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        let result = engine.execute(request_for(
            r#"
            (module
              (func (export "run") (param i64) (result i64)
                local.get 0
                i64.const 2
                i64.mul))
            "#,
            serde_json::json!({"n": 21}),
        ))?;
        assert_eq!(result.status, ExecutionStatus::Ok);
        assert_eq!(result.value, serde_json::json!(42));
        assert!(result.deterministic);
        assert!(result
            .effective_isolation
            .mechanisms
            .contains(&"empty-linker".to_string()));
        Ok(())
    }

    #[test]
    fn inputs_digest_ignores_idempotency_key() -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        let wat = r#"
            (module
              (func (export "run") (param i64) (result i64)
                local.get 0))
            "#;
        let mut first = request_for(wat, serde_json::json!({"n": 7}));
        first.idempotency_key = Some("journal-step-a".to_string());
        let mut second = first.clone();
        second.idempotency_key = Some("journal-step-b".to_string());

        let first = engine.execute(first)?;
        let second = engine.execute(second)?;

        assert_eq!(first.status, ExecutionStatus::Ok);
        assert_eq!(second.status, ExecutionStatus::Ok);
        assert_eq!(first.inputs_digest, second.inputs_digest);
        Ok(())
    }

    #[test]
    fn wasm_imports_are_denied() -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        let result = engine.execute(request_for(
            r#"
            (module
              (import "wasi:filesystem" "read" (func))
              (func (export "run")))
            "#,
            serde_json::Value::Null,
        ))?;
        assert_eq!(result.status, ExecutionStatus::Denied);
        let code = result.error.map(|error| error.code);
        assert_eq!(code.as_deref(), Some("host_import_denied"));
        Ok(())
    }

    #[test]
    fn wasm_imports_are_denied_before_full_compile() -> Result<(), Box<dyn std::error::Error>> {
        use base64::Engine as _;

        let engine = BeatboxEngine::new()?;
        let mut module = wat::parse_str(
            r#"
            (module
              (import "wasi:filesystem" "read" (func))
              (func (export "run")))
            "#,
        )?;
        module.push(0xff);

        let mut request = request_for("(module)", serde_json::Value::Null);
        request.source = Source::WasmBytesBase64 {
            bytes: base64::engine::general_purpose::STANDARD.encode(module),
        };
        let result = engine.execute(request)?;

        assert_eq!(result.status, ExecutionStatus::Denied);
        let code = result.error.map(|error| error.code);
        assert_eq!(code.as_deref(), Some("host_import_denied"));
        Ok(())
    }

    #[test]
    fn wasi_capability_imports_are_denied_under_seeded_policy(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        let mut request = request_for(
            r#"
            (module
              (import "wasi:clocks/wall-clock" "now" (func))
              (import "wasi:random/random" "get-random-bytes" (func))
              (import "wasi:sockets/tcp" "start-connect" (func))
              (func (export "run")))
            "#,
            serde_json::Value::Null,
        );
        request.policy.determinism = Determinism::Seeded {
            seed: 7,
            epoch_ms: 0,
        };

        let result = engine.execute(request)?;

        assert_eq!(result.status, ExecutionStatus::Denied);
        assert!(result.deterministic);
        let error = result
            .error
            .as_ref()
            .ok_or_else(|| std::io::Error::other("denied imports should include an error"))?;
        assert_eq!(error.code, "host_import_denied");
        assert!(result.stderr.contains("wasi:clocks/wall-clock::now"));
        assert!(result
            .stderr
            .contains("wasi:random/random::get-random-bytes"));
        assert!(result.stderr.contains("wasi:sockets/tcp::start-connect"));
        Ok(())
    }

    #[test]
    fn wasm_policy_expansion_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
        let workspace = Policy {
            fs: FsPolicy {
                workspace: Some(std::path::PathBuf::from("/tmp/beatbox-workspace")),
                mounts: Vec::new(),
            },
            ..Policy::default()
        };

        let mounts = Policy {
            fs: FsPolicy {
                workspace: None,
                mounts: vec![Mount {
                    host: std::path::PathBuf::from("/tmp/host"),
                    guest: std::path::PathBuf::from("/workspace"),
                    mode: MountMode::Ro,
                }],
            },
            ..Policy::default()
        };

        let net = Policy {
            net: NetPolicy::Proxy {
                allow_domains: vec!["example.com".to_string()],
                allow_ports: vec![443],
            },
            ..Policy::default()
        };

        let env = Policy {
            env: std::collections::BTreeMap::from([(
                "AWS_ACCESS_KEY_ID".to_string(),
                "must-not-leak".to_string(),
            )]),
            ..Policy::default()
        };

        let secrets = Policy {
            secrets: vec![Secret {
                name: "TOKEN".to_string(),
                value_ref: "host-token".to_string(),
                expose: SecretExpose::Env,
            }],
            ..Policy::default()
        };

        let double_jail = Policy {
            double_jail: true,
            ..Policy::default()
        };

        for (expected_field, policy) in [
            ("fs.workspace", workspace),
            ("fs.mounts", mounts),
            ("net", net),
            ("env", env),
            ("secrets", secrets),
            ("double_jail", double_jail),
        ] {
            let mut request = request_for(
                r#"
                (module
                  (func (export "run")))
                "#,
                serde_json::Value::Null,
            );
            request.policy = policy;
            match BeatboxEngine::new()?.execute(request) {
                Err(EngineError::PolicyUnenforceable { field, lane, .. }) => {
                    assert_eq!(field, expected_field);
                    assert_eq!(lane, Lane::Wasm);
                }
                other => panic!("expected {expected_field} policy rejection, got {other:?}"),
            }
        }
        Ok(())
    }

    #[test]
    fn wasm_unsupported_stdin_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
        let mut request = request_for(
            r#"
            (module
              (func (export "run")))
            "#,
            serde_json::Value::Null,
        );
        request.stdin = "ignored input must fail".to_string();

        match BeatboxEngine::new()?.execute(request) {
            Err(EngineError::UnsupportedRequestField { field, lane, .. }) => {
                assert_eq!(field, "stdin");
                assert_eq!(lane, Lane::Wasm);
            }
            other => panic!("expected stdin request field rejection, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn wasm_wall_deadlines_are_isolated_between_concurrent_runs(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        let mut long_request = spin_request();
        long_request.policy.limits.wall_ms = 250;
        long_request.policy.limits.fuel = Some(1_000_000_000);
        let mut short_request = spin_request();
        short_request.policy.limits.wall_ms = 40;
        short_request.policy.limits.fuel = Some(1_000_000_000);

        let long_engine = engine.clone();
        let long_handle = std::thread::spawn(move || {
            let started = Instant::now();
            let result = long_engine.execute(long_request);
            (started.elapsed(), result)
        });
        std::thread::sleep(Duration::from_millis(20));

        let short_started = Instant::now();
        let short_result = engine.execute(short_request)?;
        let short_elapsed = short_started.elapsed();
        let (long_elapsed, long_result) = long_handle
            .join()
            .map_err(|_| std::io::Error::other("long wasm thread panicked"))?;
        let long_result = long_result?;

        assert_eq!(short_result.status, ExecutionStatus::Timeout);
        assert_eq!(
            short_result.error.as_ref().map(|error| error.code.as_str()),
            Some("wall_timeout"),
            "{}",
            short_result.stderr
        );
        assert_eq!(long_result.status, ExecutionStatus::Timeout);
        assert_eq!(
            long_result.error.as_ref().map(|error| error.code.as_str()),
            Some("wall_timeout"),
            "{}",
            long_result.stderr
        );
        assert!(
            long_elapsed >= Duration::from_millis(150),
            "long run was interrupted after {long_elapsed:?}; short run took {short_elapsed:?}"
        );
        Ok(())
    }

    #[test]
    fn wasm_running_execution_can_be_canceled() -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        let mut request = spin_request();
        request.policy.limits.wall_ms = 5_000;
        request.policy.limits.fuel = Some(10_000_000_000);
        let cancellation = CancellationToken::new();
        let run_cancellation = cancellation.clone();

        let handle = std::thread::spawn(move || {
            let started = Instant::now();
            let result = engine.execute_with_cancellation(request, run_cancellation);
            (started.elapsed(), result)
        });
        std::thread::sleep(Duration::from_millis(30));
        cancellation.cancel();

        let (elapsed, result) = handle
            .join()
            .map_err(|_| std::io::Error::other("canceled wasm thread panicked"))?;
        let result = result?;

        assert_eq!(result.status, ExecutionStatus::Killed);
        assert_eq!(
            result.error.as_ref().map(|error| error.code.as_str()),
            Some("canceled")
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "canceled execution took {elapsed:?}"
        );
        Ok(())
    }

    fn spin_request() -> ExecuteRequest {
        request_for(
            r#"
            (module
              (func (export "run") (param i64) (result i64)
                (loop
                  br 0)
                i64.const 0))
            "#,
            serde_json::json!({"n": 0}),
        )
    }

    #[test]
    fn error_body_respects_output_byte_limit() -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        let mut request = request_for(
            r#"
            (module
              (import "very-long-import-module-name" "very-long-import-name" (func))
              (func (export "run")))
            "#,
            serde_json::Value::Null,
        );
        request.policy.limits.output_bytes = 8;
        let result = engine.execute(request)?;

        assert_eq!(result.status, ExecutionStatus::Denied);
        assert!(result.stderr_truncated);
        assert!(result.stderr.len() <= 8);
        let error = result.error.as_ref().ok_or_else(|| {
            std::io::Error::other("denied import should include a structured error")
        })?;
        assert_eq!(error.code, "host_import_denied");
        assert!(error.message.len() <= 8);
        Ok(())
    }

    #[test]
    fn wasm_spin_consumes_fuel() -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        let mut request = request_for(
            r#"
            (module
              (func (export "run") (param i64) (result i64)
                (loop
                  br 0)
                i64.const 0))
            "#,
            serde_json::json!({"n": 0}),
        );
        request.policy.limits.fuel = Some(1_000);
        request.policy.limits.wall_ms = 1_000;
        let result = engine.execute(request)?;
        assert_eq!(result.status, ExecutionStatus::Timeout);
        let code = result.error.map(|error| error.code);
        assert_eq!(code.as_deref(), Some("fuel_exhausted"));
        Ok(())
    }

    #[test]
    fn wasm_memory_minimum_hits_store_limit() -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        let mut request = request_for(
            r#"
            (module
              (memory 2)
              (func (export "run") (result i64)
                i64.const 1))
            "#,
            serde_json::Value::Null,
        );
        request.policy.limits.memory_bytes = 65_536;

        let result = engine.execute(request)?;

        assert_eq!(result.status, ExecutionStatus::Oom, "{}", result.stderr);
        let error = result
            .error
            .as_ref()
            .ok_or_else(|| std::io::Error::other("oom should include an error"))?;
        assert_eq!(error.code, "memory_limit");
        Ok(())
    }

    #[test]
    fn wasm_memory_grow_traps_at_store_limit() -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        let mut request = request_for(
            r#"
            (module
              (memory 1)
              (func (export "run") (result i64)
                i32.const 1
                memory.grow
                drop
                i64.const 1))
            "#,
            serde_json::Value::Null,
        );
        request.policy.limits.memory_bytes = 65_536;

        let result = engine.execute(request)?;

        assert_eq!(result.status, ExecutionStatus::Oom, "{}", result.stderr);
        let error = result
            .error
            .as_ref()
            .ok_or_else(|| std::io::Error::other("oom should include an error"))?;
        assert_eq!(error.code, "memory_limit");
        assert!(result.stderr.contains("beatbox memory limit exceeded"));
        Ok(())
    }

    #[test]
    fn wasm_table_minimum_hits_store_limit() -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        let mut request = request_for(
            r#"
            (module
              (table 64 funcref)
              (func (export "run") (result i64)
                i64.const 1))
            "#,
            serde_json::Value::Null,
        );
        request.policy.limits.memory_bytes = 256;

        let result = engine.execute(request)?;

        assert_eq!(result.status, ExecutionStatus::Oom, "{}", result.stderr);
        let error = result
            .error
            .as_ref()
            .ok_or_else(|| std::io::Error::other("oom should include an error"))?;
        assert_eq!(error.code, "memory_limit");
        assert!(result.stderr.contains("table desired"));
        Ok(())
    }

    #[test]
    fn wasm_table_grow_traps_at_store_limit() -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        let mut request = request_for(
            r#"
            (module
              (table 1 funcref)
              (func (export "run") (result i64)
                ref.null func
                i32.const 64
                table.grow
                drop
                i64.const 1))
            "#,
            serde_json::Value::Null,
        );
        request.policy.limits.memory_bytes = 512;

        let result = engine.execute(request)?;

        assert_eq!(result.status, ExecutionStatus::Oom, "{}", result.stderr);
        let error = result
            .error
            .as_ref()
            .ok_or_else(|| std::io::Error::other("oom should include an error"))?;
        assert_eq!(error.code, "memory_limit");
        assert!(result.stderr.contains("table desired"));
        Ok(())
    }

    #[test]
    fn wasm_memory_and_table_share_one_store_budget() -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        let mut request = request_for(
            r#"
            (module
              (memory 1)
              (table 1 funcref)
              (func (export "run") (result i64)
                i64.const 1))
            "#,
            serde_json::Value::Null,
        );
        request.policy.limits.memory_bytes = 65_536;

        let result = engine.execute(request)?;

        assert_eq!(result.status, ExecutionStatus::Oom, "{}", result.stderr);
        let error = result
            .error
            .as_ref()
            .ok_or_else(|| std::io::Error::other("oom should include an error"))?;
        assert_eq!(error.code, "memory_limit");
        assert!(result.stderr.contains("across linear memory and tables"));
        Ok(())
    }

    #[test]
    fn wasm_memory_grow_preserves_module_max_failure_semantics(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        let request = request_for(
            r#"
            (module
              (memory 1 1)
              (func (export "run") (result i64)
                i32.const 1
                memory.grow
                i64.extend_i32_s))
            "#,
            serde_json::Value::Null,
        );

        let result = engine.execute(request)?;

        assert_eq!(result.status, ExecutionStatus::Ok);
        assert_eq!(result.value, serde_json::json!(-1));
        Ok(())
    }

    #[test]
    fn wasm_inline_source_is_limited_before_wat_parse() -> Result<(), Box<dyn std::error::Error>> {
        let mut request = request_for("(module)", serde_json::Value::Null);
        request.policy.limits.memory_bytes = 1;

        match BeatboxEngine::new()?.execute(request) {
            Err(EngineError::SourceTooLarge {
                field,
                actual,
                limit,
            }) => {
                assert_eq!(field, "source");
                assert_eq!(actual, 8);
                assert_eq!(limit, 1);
            }
            other => panic!("expected inline source limit rejection, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn wasm_base64_module_is_limited_before_compile() -> Result<(), Box<dyn std::error::Error>> {
        use base64::Engine as _;

        let module = wat::parse_str("(module)")?;
        let mut request = request_for("(module)", serde_json::Value::Null);
        request.source = Source::WasmBytesBase64 {
            bytes: base64::engine::general_purpose::STANDARD.encode(&module),
        };
        let module_len = u64::try_from(module.len()).unwrap_or(u64::MAX);
        request.policy.limits.memory_bytes = module_len.saturating_sub(1);

        match BeatboxEngine::new()?.execute(request) {
            Err(EngineError::SourceEstimateTooLarge {
                field,
                estimate,
                limit,
            }) => {
                assert_eq!(field, "module");
                assert!(estimate >= module_len);
                assert_eq!(limit, module_len.saturating_sub(1));
            }
            other => panic!("expected base64 module limit rejection, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn wasm_base64_decode_estimate_is_limited_before_decode(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut request = request_for("(module)", serde_json::Value::Null);
        request.source = Source::WasmBytesBase64 {
            bytes: "!!!!".to_string(),
        };
        request.policy.limits.memory_bytes = 1;

        match BeatboxEngine::new()?.execute(request) {
            Err(EngineError::SourceEstimateTooLarge {
                field,
                estimate,
                limit,
            }) => {
                assert_eq!(field, "module");
                assert_eq!(estimate, 3);
                assert_eq!(limit, 1);
            }
            other => panic!("expected base64 estimate limit rejection, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn wasm_file_source_is_limited_before_reading() -> Result<(), Box<dyn std::error::Error>> {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "beatbox-engine-source-limit-{}-{}.wat",
            std::process::id(),
            unique
        ));
        std::fs::write(&path, b"(module)")?;

        let mut request = request_for("(module)", serde_json::Value::Null);
        request.source = Source::WasmFile { path: path.clone() };
        request.policy.limits.memory_bytes = 1;

        let result = BeatboxEngine::new()?.execute(request);
        std::fs::remove_file(&path).ok();

        match result {
            Err(EngineError::SourceTooLarge {
                field,
                actual,
                limit,
            }) => {
                assert_eq!(field, "source");
                assert_eq!(actual, 8);
                assert_eq!(limit, 1);
            }
            other => panic!("expected file source limit rejection, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn wasm_source_reader_enforces_cap_while_reading() -> Result<(), Box<dyn std::error::Error>> {
        match wasm::read_capped_source(std::io::repeat(0), 4) {
            Err(EngineError::SourceTooLarge {
                field,
                actual,
                limit,
            }) => {
                assert_eq!(field, "source");
                assert_eq!(actual, 5);
                assert_eq!(limit, 4);
            }
            other => panic!("expected streaming source cap rejection, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn wasm_source_cap_is_independent_of_large_memory_budget(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let over_cap = usize::try_from(MAX_WASM_MODULE_BYTES)? + 1;
        let mut request = request_for("(module)", serde_json::Value::Null);
        request.source = Source::WasmWat {
            text: " ".repeat(over_cap),
        };
        request.policy.limits.memory_bytes = MAX_WASM_MODULE_BYTES + 1024;

        match BeatboxEngine::new()?.execute(request) {
            Err(EngineError::SourceTooLarge {
                field,
                actual,
                limit,
            }) => {
                assert_eq!(field, "source");
                assert_eq!(actual, MAX_WASM_MODULE_BYTES + 1);
                assert_eq!(limit, MAX_WASM_MODULE_BYTES);
            }
            other => panic!("expected fixed module cap rejection, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn guest_entrypoint_names_do_not_drive_memory_classification(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        let mut request = request_for(
            r#"
            (module
              (func (export "run") (result i64)
                i64.const 1))
            "#,
            serde_json::Value::Null,
        );
        request.entrypoint = Some("grow".to_string());

        let result = engine.execute(request)?;

        assert_eq!(result.status, ExecutionStatus::Error);
        assert_eq!(
            result.error.as_ref().map(|error| error.code.as_str()),
            Some("wasm_trap")
        );
        assert!(result
            .stderr
            .contains("missing supported entrypoint `grow`"));
        Ok(())
    }

    #[test]
    #[cfg(feature = "lane-python")]
    fn python_native_workspace_is_exclusive_random_and_private(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let first = python_native::make_workspace()?;
        let second = python_native::make_workspace()?;

        assert_ne!(first.path, second.path);
        let first_meta = std::fs::symlink_metadata(&first.path)?;
        let second_meta = std::fs::symlink_metadata(&second.path)?;
        assert!(first_meta.file_type().is_dir());
        assert!(second_meta.file_type().is_dir());
        assert!(!first_meta.file_type().is_symlink());
        assert!(!second_meta.file_type().is_symlink());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(first_meta.permissions().mode() & 0o777, 0o700);
            assert_eq!(second_meta.permissions().mode() & 0o777, 0o700);
        }
        Ok(())
    }

    #[test]
    #[cfg(feature = "lane-python")]
    fn python_native_sandbox_profile_denies_broad_host_metadata_channels(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let workspace = python_native::make_workspace()?;
        let profile = python_native::sandbox_profile(
            &workspace.path,
            std::path::Path::new("/usr/bin/python3"),
        )?;

        assert!(profile.contains("(deny default)"));
        assert!(profile.contains("(deny process-fork)"));
        assert!(profile.contains("(deny network*)"));
        assert!(!profile.contains("(allow mach-lookup)"));
        assert!(!profile.contains("(allow sysctl-read)"));
        Ok(())
    }

    #[test]
    #[cfg(feature = "lane-python")]
    fn python_native_sandbox_profile_rejects_unrepresentable_paths(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let newline_workspace = std::path::Path::new("/tmp/beatbox\n(profile-inject)");
        match python_native::sandbox_profile(
            newline_workspace,
            std::path::Path::new("/Library/Developer/CommandLineTools/usr/bin/python3"),
        ) {
            Err(EngineError::SandboxProfile { reason }) => {
                assert!(reason.contains("control character"));
            }
            other => panic!("expected control-character sandbox profile rejection, got {other:?}"),
        }

        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;

            let mut raw = b"/tmp/beatbox-".to_vec();
            raw.push(0xff);
            let non_utf8 = std::path::PathBuf::from(std::ffi::OsString::from_vec(raw));
            match python_native::sandbox_profile(
                &non_utf8,
                std::path::Path::new("/Library/Developer/CommandLineTools/usr/bin/python3"),
            ) {
                Err(EngineError::SandboxProfile { reason }) => {
                    assert!(reason.contains("not valid UTF-8"));
                }
                other => panic!("expected non-UTF-8 sandbox profile rejection, got {other:?}"),
            }
        }

        Ok(())
    }

    #[test]
    #[cfg(feature = "lane-python")]
    fn python_native_interpreter_requires_trusted_root() -> Result<(), Box<dyn std::error::Error>> {
        assert!(python_native::python_binary_path_allowed(
            std::path::Path::new("/Library/Developer/CommandLineTools/usr/bin/python3")
        ));
        assert!(python_native::python_binary_path_allowed(
            std::path::Path::new("/opt/homebrew/Cellar/python@3.12/3.12.1/bin/python3.12")
        ));
        assert!(python_native::python_binary_path_allowed(
            std::path::Path::new("/usr/local/Cellar/python@3.11/3.11.9/Frameworks/Python.framework/Versions/3.11/Resources/Python.app/Contents/MacOS/Python")
        ));
        assert!(python_native::python_binary_path_allowed(
            std::path::Path::new("/Library/Developer/CommandLineTools/Library/Frameworks/Python3.framework/Versions/3.9/bin/python3")
        ));
        assert!(!python_native::python_binary_path_allowed(
            std::path::Path::new("/opt/homebrew/bin/python3")
        ));
        assert!(!python_native::python_binary_path_allowed(
            std::path::Path::new("/usr/local/bin/python3")
        ));
        assert!(!python_native::python_binary_path_allowed(
            std::path::Path::new("/opt/homebrew-malicious/bin/python3")
        ));
        assert!(!python_native::python_binary_path_allowed(
            std::path::Path::new("/tmp/python3")
        ));
        assert!(!python_native::python_binary_path_allowed(
            std::path::Path::new("/opt/homebrew/etc/python3")
        ));
        assert!(!python_native::python_binary_path_allowed(
            std::path::Path::new("/usr/local/share/python3")
        ));
        assert!(!python_native::python_binary_path_allowed(
            std::path::Path::new("/usr/local/malicious/bin/python3")
        ));
        assert!(!python_native::python_binary_path_allowed(
            std::path::Path::new("/Library/Developer/CommandLineTools/usr/local/bin/python3")
        ));
        assert!(!python_native::python_binary_path_allowed(
            std::path::Path::new("/opt/homebrew/Cellar/ruby/3.3.0/bin/python3")
        ));
        assert!(!python_native::python_binary_path_allowed(
            std::path::Path::new("/opt/homebrew/Cellar/python@dev/3.12.1/bin/python3")
        ));
        assert!(!python_native::python_binary_path_allowed(
            std::path::Path::new("/opt/homebrew/Cellar/python@3.12/3.12.1/libexec/python3")
        ));
        assert!(!python_native::python_binary_path_allowed(
            std::path::Path::new("/opt/homebrew/Cellar/python@3.12/dev-build/bin/python3")
        ));
        assert!(!python_native::python_binary_path_allowed(
            std::path::Path::new("/Library/Developer/CommandLineTools/Library/Frameworks/Python3.framework/Versions/dev/bin/python3")
        ));
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;

            let mut path = b"/opt/homebrew/Cellar/python@3.12/3.12.1/".to_vec();
            path.push(0xff);
            path.extend_from_slice(b"/bin/python3");
            let path = std::path::PathBuf::from(std::ffi::OsString::from_vec(path));
            assert!(!python_native::python_binary_path_allowed(&path));
        }

        let workspace = python_native::make_workspace()?;
        let untrusted = workspace.path.join("python3");
        std::fs::write(&untrusted, b"#!/bin/sh\n")?;

        assert!(python_native::trusted_python_binary(untrusted).is_none());
        Ok(())
    }

    #[test]
    #[cfg(feature = "lane-python")]
    fn python_native_framework_companion_must_match_trusted_runtime(
    ) -> Result<(), Box<dyn std::error::Error>> {
        assert!(python_native::framework_python_binary(std::path::Path::new(
            "/Library/Developer/CommandLineTools/usr/bin/python3"
        ))
        .is_none());

        let clt_framework = python_native::framework_python_binary(std::path::Path::new(
            "/Library/Developer/CommandLineTools/Library/Frameworks/Python3.framework/Versions/3.9/bin/python3",
        ))
        .ok_or_else(|| std::io::Error::other("CLT framework Python.app path must be trusted"))?;
        assert_eq!(
            clt_framework,
            std::path::Path::new(
                "/Library/Developer/CommandLineTools/Library/Frameworks/Python3.framework/Versions/3.9/Resources/Python.app/Contents/MacOS/Python"
            )
        );

        let homebrew_framework = python_native::framework_python_binary(std::path::Path::new(
            "/opt/homebrew/Cellar/python@3.12/3.12.1/bin/python3.12",
        ))
        .ok_or_else(|| std::io::Error::other("Homebrew Python.app path must be trusted"))?;
        assert_eq!(
            homebrew_framework,
            std::path::Path::new(
                "/opt/homebrew/Cellar/python@3.12/3.12.1/Resources/Python.app/Contents/MacOS/Python"
            )
        );

        Ok(())
    }

    #[test]
    #[cfg(feature = "lane-python")]
    fn python_native_runtime_read_roots_stay_narrow() -> Result<(), Box<dyn std::error::Error>> {
        let homebrew_shim =
            python_native::python_install_roots(std::path::Path::new("/opt/homebrew/bin/python3"));
        assert!(!homebrew_shim
            .iter()
            .any(|root| root == std::path::Path::new("/opt/homebrew")));

        let local_shim =
            python_native::python_install_roots(std::path::Path::new("/usr/local/bin/python3"));
        assert!(!local_shim
            .iter()
            .any(|root| root == std::path::Path::new("/usr/local")));

        let clt_shim = python_native::python_install_roots(std::path::Path::new(
            "/Library/Developer/CommandLineTools/usr/bin/python3",
        ));
        assert!(!clt_shim.iter().any(|root| {
            root == std::path::Path::new("/Library/Developer/CommandLineTools/usr")
        }));

        let cellar = python_native::python_install_roots(std::path::Path::new(
            "/opt/homebrew/Cellar/python@3.12/3.12.1/bin/python3.12",
        ));
        assert!(cellar.iter().any(|root| {
            root == std::path::Path::new("/opt/homebrew/Cellar/python@3.12/3.12.1")
        }));

        let framework = python_native::python_install_roots(std::path::Path::new(
            "/opt/homebrew/Cellar/python@3.12/3.12.1/Frameworks/Python.framework/Versions/3.12/bin/python3.12",
        ));
        assert!(framework.iter().any(|root| {
            root == std::path::Path::new(
                "/opt/homebrew/Cellar/python@3.12/3.12.1/Frameworks/Python.framework",
            )
        }));
        assert!(framework.iter().any(|root| {
            root == std::path::Path::new(
                "/opt/homebrew/Cellar/python@3.12/3.12.1/Frameworks/Python.framework/Versions/3.12",
            )
        }));
        Ok(())
    }

    #[test]
    #[cfg(feature = "lane-python")]
    fn python_native_clt_executable_scan_filters_version_names(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let root = python_native::make_workspace()?;
        let versions = root.path.join("Versions");

        for path in [
            versions.join("3.9").join("bin").join("python3"),
            versions.join("3.9").join("bin").join("python3.9"),
            versions
                .join("3.9")
                .join("Resources")
                .join("Python.app")
                .join("Contents")
                .join("MacOS")
                .join("Python"),
            versions.join("dev").join("bin").join("python3"),
            versions.join("dev").join("bin").join("pythondev"),
            versions
                .join("dev")
                .join("Resources")
                .join("Python.app")
                .join("Contents")
                .join("MacOS")
                .join("Python"),
        ] {
            let parent = path
                .parent()
                .ok_or_else(|| std::io::Error::other("test path must have a parent"))?;
            std::fs::create_dir_all(parent)?;
            std::fs::write(path, b"")?;
        }

        let paths = python_native::command_line_tools_framework_python_binaries(&versions);

        assert!(paths.iter().any(|path| path.ends_with("3.9/bin/python3")));
        assert!(paths.iter().any(|path| path.ends_with("3.9/bin/python3.9")));
        assert!(paths.iter().any(|path| path
            .to_string_lossy()
            .ends_with("3.9/Resources/Python.app/Contents/MacOS/Python")));
        assert!(!paths
            .iter()
            .any(|path| path.to_string_lossy().contains("/dev/")));
        Ok(())
    }

    #[test]
    #[cfg(all(feature = "lane-python", unix))]
    fn python_native_workspace_disk_usage_stays_inside_workspace(
    ) -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let workspace = python_native::make_workspace()?;
        let outside = workspace.path.with_extension("outside");
        std::fs::write(&outside, vec![0_u8; 64 * 1024])?;
        std::fs::write(workspace.path.join("inside.bin"), vec![0_u8; 512])?;
        symlink(&outside, workspace.path.join("outside-link"))?;

        let used_bytes = python_native::workspace_disk_usage(&workspace.path, u64::MAX)?;

        assert!(used_bytes >= 512);
        assert!(
            used_bytes < 64 * 1024,
            "workspace usage followed a symlink outside the sandbox: {used_bytes}"
        );

        std::fs::remove_file(outside).ok();
        Ok(())
    }

    #[test]
    #[cfg(all(feature = "lane-python", target_os = "macos"))]
    fn python_native_unsupported_request_fields_fail_closed(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut with_entrypoint = python_native_request("print('hello')");
        with_entrypoint.entrypoint = Some("main".to_string());

        let mut with_input = python_native_request("print('hello')");
        with_input.input = serde_json::json!({"n": 1});

        let mut with_stdin = python_native_request("print('hello')");
        with_stdin.stdin = "data".to_string();

        for (expected_field, request) in [
            ("entrypoint", with_entrypoint),
            ("input", with_input),
            ("stdin", with_stdin),
        ] {
            match BeatboxEngine::new()?.execute(request) {
                Err(EngineError::UnsupportedRequestField { field, lane, .. }) => {
                    assert_eq!(field, expected_field);
                    assert_eq!(lane, Lane::PythonNative);
                }
                other => panic!("expected {expected_field} request field rejection, got {other:?}"),
            }
        }
        Ok(())
    }

    #[test]
    #[cfg(all(feature = "lane-python", target_os = "macos"))]
    fn python_native_rejects_host_file_source_before_reading(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let request = ExecuteRequest {
            lane: Lane::PythonNative,
            source: Source::WasmFile {
                path: std::path::PathBuf::from("/tmp/beatbox-python-source-must-not-be-read.py"),
            },
            entrypoint: None,
            input: serde_json::Value::Null,
            stdin: String::new(),
            policy: Policy::default(),
            idempotency_key: None,
        };

        match BeatboxEngine::new()?.execute(request) {
            Err(EngineError::InvalidSource { lane, reason }) => {
                assert_eq!(lane, Lane::PythonNative);
                assert!(reason.contains("inline source"));
                assert!(reason.contains("wasm_file"));
            }
            other => panic!("expected python_native source rejection, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    #[cfg(all(feature = "lane-python", target_os = "macos"))]
    fn python_native_inline_source_is_limited_before_spawn(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let request = python_native_request(&"x".repeat((MAX_PYTHON_SOURCE_BYTES as usize) + 1));

        match BeatboxEngine::new()?.execute(request) {
            Err(EngineError::SourceTooLarge {
                field,
                actual,
                limit,
            }) => {
                assert_eq!(field, "source");
                assert!(actual > limit);
                assert_eq!(limit, MAX_PYTHON_SOURCE_BYTES);
            }
            other => panic!("expected python_native source limit rejection, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    #[cfg(all(feature = "lane-python", unix))]
    fn python_native_stdin_delivery_failure_kills_child() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut child = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("exec 0<&-; sleep 60")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        let stdin = python_native::write_stdin_async(child.stdin.take(), vec![b'x'; 1024 * 1024]);

        match python_native::wait_with_wall_limit(&mut child, 5_000, None, stdin, None) {
            python_native::ChildOutcome::StdinError(error) => {
                assert!(!error.is_empty());
            }
            _ => panic!("expected stdin delivery failure"),
        }
        assert!(child.try_wait()?.is_some());
        Ok(())
    }

    #[test]
    #[cfg(all(feature = "lane-python", target_os = "macos"))]
    fn python_native_kills_workspace_disk_limit() -> Result<(), Box<dyn std::error::Error>> {
        if !python_native_available() {
            return Ok(());
        }
        let mut request = python_native_request(
            r#"
with open("big.bin", "wb") as handle:
    handle.write(b"x" * 131072)
print("done")
"#,
        );
        request.policy.limits.disk_bytes = 32 * 1024;

        let result = BeatboxEngine::new()?.execute(request)?;

        assert_eq!(result.status, ExecutionStatus::Killed);
        assert_eq!(
            result.error.as_ref().map(|error| error.code.as_str()),
            Some("disk_limit")
        );
        Ok(())
    }

    #[test]
    #[cfg(all(feature = "lane-python", target_os = "macos"))]
    fn python_native_clears_env_and_denies_sensitive_host_reads(
    ) -> Result<(), Box<dyn std::error::Error>> {
        if !python_native_available() {
            return Ok(());
        }
        let engine = BeatboxEngine::new()?;
        let request = ExecuteRequest {
            lane: Lane::PythonNative,
            source: Source::Inline {
                code: r#"import os
print(os.environ.get("HOME"))
for path in ("/etc/passwd", "/bin/ls"):
    try:
        open(path, "rb").read(1)
        print("read-ambient-path:" + path)
    except Exception as exc:
        print(path + ":" + type(exc).__name__)
"#
                .to_string(),
            },
            entrypoint: None,
            input: serde_json::Value::Null,
            stdin: String::new(),
            policy: Policy::default(),
            idempotency_key: None,
        };

        let result = engine.execute(request)?;

        assert_eq!(result.status, ExecutionStatus::Ok, "{}", result.stderr);
        assert_eq!(result.lane, Lane::PythonNative);
        assert!(result.stdout.lines().any(|line| line == "None"));
        assert!(!result.stdout.contains("read-ambient-path"));
        assert!(result.stdout.contains("/etc/passwd:"));
        assert!(result.stdout.contains("/bin/ls:"));
        assert!(result
            .effective_isolation
            .mechanisms
            .contains(&"sandbox-exec".to_string()));
        Ok(())
    }

    #[test]
    #[cfg(feature = "lane-python")]
    fn python_native_policy_expansion_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        let os = std::env::consts::OS.to_string();
        if !cfg!(target_os = "macos") {
            match engine.execute(python_native_request("print('hello')")) {
                Err(EngineError::PolicyUnenforceable { field, lane, .. }) => {
                    assert_eq!(field, "lane");
                    assert_eq!(lane, Lane::PythonNative);
                }
                other => panic!("expected python_native OS rejection on {os}, got {other:?}"),
            }
            return Ok(());
        }

        let workspace = Policy {
            fs: FsPolicy {
                workspace: Some(std::path::PathBuf::from("/tmp/beatbox-workspace")),
                mounts: Vec::new(),
            },
            ..Policy::default()
        };

        let mounts = Policy {
            fs: FsPolicy {
                workspace: None,
                mounts: vec![Mount {
                    host: std::path::PathBuf::from("/tmp/host"),
                    guest: std::path::PathBuf::from("/workspace"),
                    mode: MountMode::Ro,
                }],
            },
            ..Policy::default()
        };

        let net = Policy {
            net: NetPolicy::Proxy {
                allow_domains: vec!["example.com".to_string()],
                allow_ports: vec![443],
            },
            ..Policy::default()
        };

        let env = Policy {
            env: std::collections::BTreeMap::from([(
                "AWS_ACCESS_KEY_ID".to_string(),
                "must-not-leak".to_string(),
            )]),
            ..Policy::default()
        };

        let secrets = Policy {
            secrets: vec![Secret {
                name: "TOKEN".to_string(),
                value_ref: "host-token".to_string(),
                expose: SecretExpose::Env,
            }],
            ..Policy::default()
        };

        let determinism = Policy {
            determinism: Determinism::Seeded {
                seed: 7,
                epoch_ms: 0,
            },
            ..Policy::default()
        };

        let double_jail = Policy {
            double_jail: true,
            ..Policy::default()
        };

        let mut cpu_ms = Policy::default();
        cpu_ms.limits.cpu_ms += 1;
        let mut memory_bytes = Policy::default();
        memory_bytes.limits.memory_bytes += 1;
        let mut pids = Policy::default();
        pids.limits.pids += 1;
        let mut fuel = Policy::default();
        fuel.limits.fuel = Some(fuel.limits.fuel.unwrap_or_default() + 1);

        for (expected_field, policy) in [
            ("fs.workspace", workspace),
            ("fs.mounts", mounts),
            ("net", net),
            ("env", env),
            ("secrets", secrets),
            ("determinism", determinism),
            ("double_jail", double_jail),
            ("limits.cpu_ms", cpu_ms),
            ("limits.memory_bytes", memory_bytes),
            ("limits.pids", pids),
            ("limits.fuel", fuel),
        ] {
            let mut request = python_native_request("print('hello')");
            request.policy = policy;
            match engine.execute(request) {
                Err(EngineError::PolicyUnenforceable { field, lane, .. }) => {
                    assert_eq!(field, expected_field);
                    assert_eq!(lane, Lane::PythonNative);
                }
                other => panic!("expected {expected_field} policy rejection, got {other:?}"),
            }
        }

        Ok(())
    }

    #[test]
    fn unimplemented_lanes_are_denied_without_isolation_claims(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        let lanes = [Lane::PythonWasi, Lane::JsWasm, Lane::JsNative, Lane::Exec];
        #[cfg(not(feature = "lane-python"))]
        let lanes = [lanes.as_slice(), &[Lane::PythonNative]].concat();
        #[cfg(feature = "lane-python")]
        let lanes = lanes.to_vec();

        for lane in lanes {
            let request = ExecuteRequest {
                lane: lane.clone(),
                source: Source::Inline {
                    code: "print('hello')".to_string(),
                },
                entrypoint: None,
                input: serde_json::Value::Null,
                stdin: String::new(),
                policy: Policy::default(),
                idempotency_key: None,
            };
            let result = engine.execute(request)?;

            assert_eq!(result.status, ExecutionStatus::Denied);
            assert!(!result.deterministic);
            assert!(result.effective_isolation.mechanisms.is_empty());
            let error = result.error.as_ref().ok_or_else(|| {
                std::io::Error::other("unimplemented lane should include an error")
            })?;
            assert_eq!(error.code, "lane_unavailable");
        }
        Ok(())
    }

    #[cfg(feature = "lane-python")]
    fn python_native_request(code: &str) -> ExecuteRequest {
        ExecuteRequest {
            lane: Lane::PythonNative,
            source: Source::Inline {
                code: code.to_string(),
            },
            entrypoint: None,
            input: serde_json::Value::Null,
            stdin: String::new(),
            policy: Policy::default(),
            idempotency_key: None,
        }
    }
}
