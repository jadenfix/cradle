use std::time::{Duration, Instant};

use beatbox_core::{
    EffectiveIsolation, ErrorBody, ExecuteRequest, ExecutionResult, ExecutionStatus, Lane, Metrics,
    NetPolicy, Source,
};
use thiserror::Error;

#[derive(Clone)]
pub struct BeatboxEngine {
    #[cfg(feature = "lane-wasi")]
    wasm: wasm::WasmLane,
}

impl BeatboxEngine {
    pub fn new() -> Result<Self, EngineError> {
        Ok(Self {
            #[cfg(feature = "lane-wasi")]
            wasm: wasm::WasmLane::new()?,
        })
    }

    pub fn execute(&self, request: ExecuteRequest) -> Result<ExecutionResult, EngineError> {
        match request.lane.clone() {
            Lane::Wasm => {
                #[cfg(feature = "lane-wasi")]
                {
                    self.wasm.execute(request)
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
    #[error("failed to read source {path}: {source}")]
    ReadSource {
        path: String,
        source: std::io::Error,
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
            Self::ReadSource { path, source } => {
                ErrorBody::new("read_source", format!("failed to read {path}: {source}"))
            }
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
    let deterministic = matches!(request.lane, Lane::Wasm);
    let stderr = error
        .as_ref()
        .map_or_else(String::new, |body| body.message.clone());
    let (stderr, stderr_truncated) = truncate(stderr, request.policy.limits.output_bytes);
    ExecutionResult {
        status,
        value,
        exit_code: None,
        stdout: String::new(),
        stdout_truncated: false,
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

#[cfg(feature = "lane-wasi")]
mod wasm {
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    use base64::Engine as _;
    use beatbox_core::{MountMode, Policy};
    use wasmtime::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};

    use super::*;

    #[derive(Clone)]
    pub struct WasmLane {
        engine: Engine,
    }

    struct WasmState {
        limits: StoreLimits,
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
            Ok(Self { engine })
        }

        pub fn execute(&self, request: ExecuteRequest) -> Result<ExecutionResult, EngineError> {
            admit_wasm_policy(&request.policy)?;
            let started = Instant::now();
            let module_bytes = load_wasm_source(&request.source)?;
            let inputs_digest = digest_wasm_inputs(&request, &module_bytes)?;
            let isolation = wasm_isolation(&request.policy);

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
            let store_limits = StoreLimitsBuilder::new()
                .memory_size(memory_limit)
                .instances(1)
                .memories(1)
                .tables(4)
                .build();
            let mut store = Store::new(
                &self.engine,
                WasmState {
                    limits: store_limits,
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

            store.set_epoch_deadline(1);
            let stop_ticker = epoch_ticker(self.engine.clone(), request.policy.limits.wall_ms);
            let linker = Linker::new(&self.engine);
            let value = run_entrypoint(&mut store, &linker, &module, &request);
            stop_ticker.stop();

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
                    let (status, code) = if fuel_exhausted {
                        (ExecutionStatus::Timeout, "fuel_exhausted")
                    } else {
                        classify_wasm_error(&error)
                    };
                    Ok(result(
                        &request,
                        status,
                        serde_json::Value::Null,
                        Some(ErrorBody::new(code, error)),
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
        Ok(())
    }

    fn load_wasm_source(source: &Source) -> Result<Vec<u8>, EngineError> {
        match source {
            Source::Inline { code } | Source::WasmWat { text: code } => {
                wat::parse_str(code).map_err(|error| EngineError::ParseWat(error.to_string()))
            }
            Source::WasmFile { path } => {
                let bytes = std::fs::read(path).map_err(|source| EngineError::ReadSource {
                    path: path.display().to_string(),
                    source,
                })?;
                if path.extension().and_then(|ext| ext.to_str()) == Some("wat") {
                    wat::parse_bytes(&bytes)
                        .map(|cow| cow.into_owned())
                        .map_err(|error| EngineError::ParseWat(error.to_string()))
                } else {
                    Ok(bytes)
                }
            }
            Source::WasmBytesBase64 { bytes } => base64::engine::general_purpose::STANDARD
                .decode(bytes)
                .map_err(|error| EngineError::DecodeBase64(error.to_string())),
            Source::ModuleRef { .. } => Err(EngineError::InvalidSource {
                lane: Lane::Wasm,
                reason: "module_ref storage is planned for M2.5 and is not implemented yet"
                    .to_string(),
            }),
        }
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

    fn wasm_isolation(policy: &Policy) -> EffectiveIsolation {
        let mut downgrades = Vec::new();
        if policy.double_jail {
            downgrades.push("double_jail_unavailable_in_initial_wasm_lane".to_string());
        }
        EffectiveIsolation::for_current_os(
            vec![
                "wasmtime".to_string(),
                "empty-linker".to_string(),
                "host-import-deny".to_string(),
                "fuel".to_string(),
                "epoch-interruption".to_string(),
                "store-limits".to_string(),
            ],
            downgrades,
        )
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
    ) -> Result<serde_json::Value, String> {
        let instance = linker
            .instantiate(&mut *store, module)
            .map_err(|error| error.to_string())?;
        let entrypoint = request.entrypoint.as_deref().unwrap_or("run");

        if let Ok(func) = instance.get_typed_func::<i64, i64>(&mut *store, entrypoint) {
            let input = input_i64(&request.input)?;
            let value = func
                .call(&mut *store, input)
                .map_err(|error| error.to_string())?;
            return Ok(serde_json::json!(value));
        }

        if let Ok(func) = instance.get_typed_func::<(), i64>(&mut *store, entrypoint) {
            let value = func
                .call(&mut *store, ())
                .map_err(|error| error.to_string())?;
            return Ok(serde_json::json!(value));
        }

        if let Ok(func) = instance.get_typed_func::<(), ()>(&mut *store, entrypoint) {
            func.call(&mut *store, ())
                .map_err(|error| error.to_string())?;
            return Ok(serde_json::Value::Null);
        }

        Err(format!(
            "missing supported entrypoint `{entrypoint}`; expected ()->(), ()->i64, or i64->i64"
        ))
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

    fn classify_wasm_error(message: &str) -> (ExecutionStatus, &'static str) {
        let lower = message.to_ascii_lowercase();
        if lower.contains("fuel") {
            (ExecutionStatus::Timeout, "fuel_exhausted")
        } else if lower.contains("epoch") || lower.contains("interrupt") {
            (ExecutionStatus::Timeout, "wall_timeout")
        } else if lower.contains("memory")
            || lower.contains("allocation")
            || lower.contains("failed to grow")
        {
            (ExecutionStatus::Oom, "memory_limit")
        } else {
            (ExecutionStatus::Error, "wasm_trap")
        }
    }

    struct EpochTicker {
        stop: Arc<AtomicBool>,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl EpochTicker {
        fn stop(mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn epoch_ticker(engine: Engine, wall_ms: u64) -> EpochTicker {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let sleep_for = Duration::from_millis(wall_ms.max(1));
        let handle = thread::spawn(move || {
            let tick = Duration::from_millis(10);
            let started = Instant::now();
            while started.elapsed() < sleep_for {
                if thread_stop.load(Ordering::SeqCst) {
                    return;
                }
                thread::sleep(tick);
            }
            if !thread_stop.load(Ordering::SeqCst) {
                engine.increment_epoch();
            }
        });
        EpochTicker {
            stop,
            handle: Some(handle),
        }
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
    use beatbox_core::Policy;

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
        assert!(
            result
                .effective_isolation
                .mechanisms
                .contains(&"empty-linker".to_string())
        );
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
}
