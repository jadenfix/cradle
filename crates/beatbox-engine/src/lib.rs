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

/// Cooperative cancellation handle for an in-flight execution. Cloning shares the
/// same flag; setting it (`cancel`) trips the wasm epoch interrupt at the next
/// interruptible point so a running guest stops promptly instead of running to
/// its full wall/fuel budget.
#[derive(Clone, Default)]
pub struct CancelFlag(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl CancelFlag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn is_canceled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl BeatboxEngine {
    pub fn new() -> Result<Self, EngineError> {
        Ok(Self {
            #[cfg(feature = "lane-wasi")]
            wasm: wasm::WasmLane::new()?,
        })
    }

    pub fn execute(&self, request: ExecuteRequest) -> Result<ExecutionResult, EngineError> {
        self.execute_cancellable(request, &CancelFlag::new())
    }

    /// Like [`execute`](Self::execute) but interruptible: setting `cancel` trips
    /// the wasm epoch deadline so a running execution unwinds promptly.
    pub fn execute_cancellable(
        &self,
        request: ExecuteRequest,
        cancel: &CancelFlag,
    ) -> Result<ExecutionResult, EngineError> {
        let _ = cancel;
        match request.lane.clone() {
            Lane::Wasm => {
                #[cfg(feature = "lane-wasi")]
                {
                    self.wasm.execute(request, cancel)
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
    // The initial wasm lane is W0: an empty linker denies every host import, so
    // core modules are deterministic by construction. Revisit this when W1
    // capability-scoped WASI is added under Lane::Wasm.
    let deterministic = matches!(request.lane, Lane::Wasm);
    let (error, stderr, stderr_truncated) =
        truncate_error(error, request.policy.limits.output_bytes);
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
    use wasmtime::{Config, Engine, Linker, Module, ResourceLimiter, Store};

    use super::*;

    /// Upper bound on both the raw source (WAT text / decoded bytes) and the
    /// resulting module bytes accepted by the engine. Attacker-controlled bytes
    /// are parsed and compiled by Cranelift before any store/fuel/memory/epoch
    /// limit applies, so a crafted large module can burn host CPU/memory outside
    /// all policy limits. The HTTP surface caps request bodies separately; this
    /// guard protects the engine (and non-HTTP embedders) independently.
    const MAX_WASM_MODULE_BYTES: usize = 16 * 1024 * 1024;

    #[derive(Clone)]
    pub struct WasmLane {
        // Intentionally holds no shared Engine. `wasmtime::Engine` clones share a
        // single Arc-backed epoch counter, so a per-execution deadline ticker
        // incrementing that global counter trips *every* concurrent store whose
        // relative deadline matches — a short job spuriously kills a long one.
        // Each execution builds its own engine below so the epoch counter and its
        // ticker are isolated. Engine construction is cheap next to compilation.
    }

    struct WasmState {
        limits: WasmStoreLimits,
    }

    struct WasmStoreLimits {
        memory_size: usize,
        // Current linear-memory and table byte usage, tracked so the policy
        // memory ceiling bounds *total* guest host memory. Tables and linear
        // memory share one budget: capping each in isolation would let a guest
        // reach `tables * memory_size` (the store permits several tables) plus
        // the linear-memory budget on top, several times the intended ceiling.
        linear_bytes: usize,
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
            // `desired` is the absolute new size of this linear memory (one
            // memory per store). Bound it together with table storage.
            let total = desired.saturating_add(self.table_bytes);
            if total > self.memory_size {
                Err(wasmtime::format_err!(
                    "beatbox memory limit exceeded: desired {desired} bytes plus {} table bytes exceeds policy limit {} bytes",
                    self.table_bytes,
                    self.memory_size
                ))
            } else {
                self.linear_bytes = desired;
                Ok(true)
            }
        }

        fn table_growing(
            &mut self,
            current: usize,
            desired: usize,
            _maximum: Option<usize>,
        ) -> wasmtime::Result<bool> {
            // Table element storage is host memory too, but wasmtime does not
            // count it against `memory_growing`. Without a bound a guest can
            // allocate many GB of pointers via a large table (declared minimum or
            // `table.grow`) and bypass the policy memory ceiling. Charge each
            // element at least a pointer and accumulate across every table so the
            // aggregate (plus linear memory) stays within the single budget.
            let added = desired
                .saturating_sub(current)
                .saturating_mul(std::mem::size_of::<usize>());
            let new_table_bytes = self.table_bytes.saturating_add(added);
            let total = self.linear_bytes.saturating_add(new_table_bytes);
            if total > self.memory_size {
                Err(wasmtime::format_err!(
                    "beatbox table limit exceeded: {new_table_bytes} table bytes plus {} linear bytes exceeds policy limit {} bytes",
                    self.linear_bytes,
                    self.memory_size
                ))
            } else {
                self.table_bytes = new_table_bytes;
                Ok(true)
            }
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

    fn build_engine() -> Result<Engine, EngineError> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.consume_fuel(true);
        config.epoch_interruption(true);
        config.cranelift_nan_canonicalization(true);
        config.relaxed_simd_deterministic(true);
        Engine::new(&config).map_err(|error| EngineError::WasmtimeEngine(error.to_string()))
    }

    impl WasmLane {
        pub fn new() -> Result<Self, EngineError> {
            // Validate the engine configuration once so misconfiguration fails at
            // construction rather than on the first execute.
            build_engine()?;
            Ok(Self {})
        }

        pub fn execute(
            &self,
            request: ExecuteRequest,
            cancel: &CancelFlag,
        ) -> Result<ExecutionResult, EngineError> {
            admit_wasm_policy(&request.policy)?;
            let started = Instant::now();
            let engine = build_engine()?;
            let module_bytes = load_wasm_source(&request.source)?;
            let inputs_digest = digest_wasm_inputs(&request, &module_bytes)?;
            let isolation = wasm_isolation(&request.policy);

            if module_bytes.len() > MAX_WASM_MODULE_BYTES {
                return Ok(result(
                    &request,
                    ExecutionStatus::Denied,
                    serde_json::Value::Null,
                    Some(ErrorBody::new(
                        "module_too_large",
                        format!(
                            "wasm module is {} bytes, exceeding the engine limit of {MAX_WASM_MODULE_BYTES} bytes",
                            module_bytes.len()
                        ),
                    )),
                    Metrics {
                        wall_time_ms: elapsed_ms(started),
                        ..Metrics::default()
                    },
                    isolation,
                    inputs_digest,
                ));
            }

            let module = match Module::new(&engine, &module_bytes) {
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
                &engine,
                WasmState {
                    limits: WasmStoreLimits {
                        memory_size: memory_limit,
                        linear_bytes: 0,
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

            store.set_epoch_deadline(1);
            let stop_ticker = epoch_ticker(
                engine.clone(),
                request.policy.limits.wall_ms,
                cancel.clone(),
            );
            let linker = Linker::new(&engine);
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
                        Some(ErrorBody::new(code, error.message)),
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
        if !matches!(policy.net, NetPolicy::Deny {}) {
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
        // Fail closed on resource limits the wasm lane cannot honor, rather than
        // silently ignoring them (which gives false assurance — SECURITY.md
        // advertises a CPU budget). The lane bounds compute via wall_ms + fuel and
        // memory via memory_bytes; cpu_ms/pids/disk_bytes have no W0 enforcement
        // point. Only a value that differs from the default (i.e. the caller
        // actually asked for that ceiling) is rejected.
        let defaults = Policy::default().limits;
        if policy.limits.cpu_ms != defaults.cpu_ms {
            return Err(EngineError::PolicyUnenforceable {
                field: "limits.cpu_ms",
                lane: Lane::Wasm,
                os,
                reason: "the wasm lane bounds compute via wall_ms and fuel; an independent cpu_ms ceiling cannot be enforced".to_string(),
            });
        }
        if policy.limits.pids != defaults.pids {
            return Err(EngineError::PolicyUnenforceable {
                field: "limits.pids",
                lane: Lane::Wasm,
                os,
                reason:
                    "the W0 wasm lane runs no host processes; a pids ceiling cannot be enforced"
                        .to_string(),
            });
        }
        if policy.limits.disk_bytes != defaults.disk_bytes {
            return Err(EngineError::PolicyUnenforceable {
                field: "limits.disk_bytes",
                lane: Lane::Wasm,
                os,
                reason: "the W0 wasm lane exposes no filesystem; a disk_bytes ceiling cannot be enforced".to_string(),
            });
        }
        Ok(())
    }

    fn load_wasm_source(source: &Source) -> Result<Vec<u8>, EngineError> {
        match source {
            Source::Inline { code } | Source::WasmWat { text: code } => {
                guard_source_len(code.len())?;
                wat::parse_str(code).map_err(|error| EngineError::ParseWat(error.to_string()))
            }
            Source::WasmFile { path } => {
                // Read at most the cap (+1 to detect overflow) so a huge file is
                // rejected without slurping it all into memory first.
                let bytes = read_file_capped(path)?;
                if path.extension().and_then(|ext| ext.to_str()) == Some("wat") {
                    wat::parse_bytes(&bytes)
                        .map(|cow| cow.into_owned())
                        .map_err(|error| EngineError::ParseWat(error.to_string()))
                } else {
                    Ok(bytes)
                }
            }
            Source::WasmBytesBase64 { bytes } => {
                guard_source_len(bytes.len())?;
                base64::engine::general_purpose::STANDARD
                    .decode(bytes)
                    .map_err(|error| EngineError::DecodeBase64(error.to_string()))
            }
            Source::ModuleRef { .. } => Err(EngineError::InvalidSource {
                lane: Lane::Wasm,
                reason: "module_ref storage is planned for M2.5 and is not implemented yet"
                    .to_string(),
            }),
        }
    }

    fn read_file_capped(path: &Path) -> Result<Vec<u8>, EngineError> {
        use std::io::Read;

        let read_error = |source| EngineError::ReadSource {
            path: path.display().to_string(),
            source,
        };
        let file = std::fs::File::open(path).map_err(read_error)?;
        // take(cap + 1): if the file is larger than the cap, we read cap+1 bytes
        // and guard_source_len rejects it; memory use stays bounded either way.
        let mut bytes = Vec::new();
        file.take(MAX_WASM_MODULE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(read_error)?;
        guard_source_len(bytes.len())?;
        Ok(bytes)
    }

    fn guard_source_len(len: usize) -> Result<(), EngineError> {
        if len > MAX_WASM_MODULE_BYTES {
            Err(EngineError::InvalidSource {
                lane: Lane::Wasm,
                reason: format!(
                    "wasm source is {len} bytes, exceeding the engine limit of {MAX_WASM_MODULE_BYTES} bytes"
                ),
            })
        } else {
            Ok(())
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
            || lower.contains("table limit exceeded")
            || lower.contains("exceeds memory limits")
            || lower.contains("exceeds memory limit")
            || lower.contains("out of memory")
            || lower.contains("memory allocation")
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

    fn epoch_ticker(engine: Engine, wall_ms: u64, cancel: CancelFlag) -> EpochTicker {
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
                // A cancel trips the epoch early (fall through to increment_epoch),
                // so a running guest is interrupted rather than pinning its worker
                // for the full wall/fuel budget.
                if cancel.is_canceled() {
                    break;
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
    fn wasi_capability_imports_are_denied_under_seeded_policy()
    -> Result<(), Box<dyn std::error::Error>> {
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
        assert!(
            result
                .stderr
                .contains("wasi:random/random::get-random-bytes")
        );
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

        let cpu = Policy {
            limits: beatbox_core::Limits {
                cpu_ms: 1,
                ..beatbox_core::Limits::default()
            },
            ..Policy::default()
        };

        let pids = Policy {
            limits: beatbox_core::Limits {
                pids: 8,
                ..beatbox_core::Limits::default()
            },
            ..Policy::default()
        };

        let disk = Policy {
            limits: beatbox_core::Limits {
                disk_bytes: 1,
                ..beatbox_core::Limits::default()
            },
            ..Policy::default()
        };

        for (expected_field, policy) in [
            ("fs.workspace", workspace),
            ("fs.mounts", mounts),
            ("net", net),
            ("env", env),
            ("secrets", secrets),
            ("limits.cpu_ms", cpu),
            ("limits.pids", pids),
            ("limits.disk_bytes", disk),
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
    fn wasm_table_grow_is_bounded_by_memory_budget() -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        // A single cheap `table.grow` with a huge delta would otherwise allocate
        // gigabytes of host pointers, uncounted against memory_bytes.
        let mut request = request_for(
            r#"
            (module
              (table 1 funcref)
              (func (export "run") (result i64)
                (table.grow (ref.null func) (i32.const 1000000000))
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
            .ok_or_else(|| std::io::Error::other("table growth denial should include an error"))?;
        assert_eq!(error.code, "memory_limit");
        assert!(result.stderr.contains("beatbox table limit exceeded"));
        Ok(())
    }

    #[test]
    fn wasm_large_initial_table_is_denied() -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        // A large declared minimum table must be caught at instantiation, not
        // allocated wholesale before any policy limit applies.
        let mut request = request_for(
            r#"
            (module
              (table 1000000000 funcref)
              (func (export "run") (result i64)
                i64.const 1))
            "#,
            serde_json::Value::Null,
        );
        request.policy.limits.memory_bytes = 65_536;

        let result = engine.execute(request)?;

        assert_eq!(result.status, ExecutionStatus::Oom, "{}", result.stderr);
        assert_eq!(
            result.error.as_ref().map(|error| error.code.as_str()),
            Some("memory_limit")
        );
        Ok(())
    }

    #[test]
    fn wasm_multiple_tables_cannot_exceed_aggregate_budget()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        // Each table individually stays under the budget, but together they would
        // exceed it. The limiter accumulates across tables, so this must be
        // denied rather than allowing tables * memory_bytes of host storage.
        let elems = 65_536 / std::mem::size_of::<usize>(); // exactly one budget's worth
        let wat = format!(
            r#"
            (module
              (table {elems} funcref)
              (table {elems} funcref)
              (func (export "run") (result i64)
                i64.const 1))
            "#
        );
        let mut request = request_for(&wat, serde_json::Value::Null);
        request.policy.limits.memory_bytes = 65_536;

        let result = engine.execute(request)?;

        assert_eq!(result.status, ExecutionStatus::Oom, "{}", result.stderr);
        assert_eq!(
            result.error.as_ref().map(|error| error.code.as_str()),
            Some("memory_limit")
        );
        Ok(())
    }

    #[test]
    fn oversized_wasm_source_is_denied_before_compilation() -> Result<(), Box<dyn std::error::Error>>
    {
        let engine = BeatboxEngine::new()?;
        // A WAT text larger than the engine cap is rejected without parsing or
        // compiling it (the compile-time DoS guard).
        let filler = "(func)".repeat(3_000_000);
        let wat = format!("(module {filler})");
        assert!(wat.len() > 16 * 1024 * 1024);
        let result = engine.execute(request_for(&wat, serde_json::Value::Null));

        match result {
            Err(EngineError::InvalidSource { reason, .. }) => {
                assert!(reason.contains("exceeding the engine limit"), "{reason}");
            }
            other => panic!("expected oversized source rejection, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn oversized_wasm_file_is_denied_by_capped_read() -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        // A file-source larger than the cap must be rejected via the bounded read
        // rather than slurped whole into memory first.
        let path = std::env::temp_dir().join("beatbox-oversized-file-source.bin");
        std::fs::write(&path, vec![0_u8; 16 * 1024 * 1024 + 1])?;

        let request = ExecuteRequest {
            lane: Lane::Wasm,
            source: Source::WasmFile { path: path.clone() },
            entrypoint: None,
            input: serde_json::Value::Null,
            stdin: String::new(),
            policy: Policy::default(),
            idempotency_key: None,
        };
        let result = engine.execute(request);
        std::fs::remove_file(&path).ok();

        match result {
            Err(EngineError::InvalidSource { reason, .. }) => {
                assert!(reason.contains("exceeding the engine limit"), "{reason}");
            }
            other => panic!("expected oversized file rejection, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn max_size_wasm_file_passes_the_size_guard() -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        // A file of exactly the cap must clear the size guard (strict `>`), failing
        // later at compilation rather than being rejected as oversized.
        let path = std::env::temp_dir().join("beatbox-max-size-file-source.bin");
        std::fs::write(&path, vec![0_u8; 16 * 1024 * 1024])?;

        let request = ExecuteRequest {
            lane: Lane::Wasm,
            source: Source::WasmFile { path: path.clone() },
            entrypoint: None,
            input: serde_json::Value::Null,
            stdin: String::new(),
            policy: Policy::default(),
            idempotency_key: None,
        };
        let result = engine.execute(request);
        std::fs::remove_file(&path).ok();

        // Not rejected for size; garbage bytes fail at Module::new (wasm_compile).
        match result {
            Ok(execution) => {
                assert_eq!(execution.status, ExecutionStatus::Error);
                assert_eq!(
                    execution.error.as_ref().map(|error| error.code.as_str()),
                    Some("wasm_compile")
                );
            }
            Err(EngineError::InvalidSource { reason, .. }) => {
                panic!("exactly-max file should pass the size guard, got: {reason}")
            }
            other => panic!("unexpected result: {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn wasm_memory_grow_preserves_module_max_failure_semantics()
    -> Result<(), Box<dyn std::error::Error>> {
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
    fn guest_entrypoint_names_do_not_drive_memory_classification()
    -> Result<(), Box<dyn std::error::Error>> {
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
        assert!(
            result
                .stderr
                .contains("missing supported entrypoint `grow`")
        );
        Ok(())
    }

    #[test]
    fn cancel_flag_interrupts_running_execution() -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        let cancel = CancelFlag::new();
        let setter = cancel.clone();

        // An infinite loop with a huge wall/fuel budget would otherwise run for
        // ~30s; a cancel must interrupt it in well under that.
        let mut request = request_for(
            r#"
            (module
              (func (export "run") (param i64) (result i64)
                (loop br 0)
                (i64.const 0)))
            "#,
            serde_json::json!({ "n": 0 }),
        );
        request.policy.limits.wall_ms = 30_000;
        request.policy.limits.fuel = Some(2_000_000_000);

        let canceller = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            setter.cancel();
        });

        let started = std::time::Instant::now();
        let result = engine.execute_cancellable(request, &cancel)?;
        let elapsed = started.elapsed();
        canceller
            .join()
            .map_err(|_| std::io::Error::other("canceller thread panicked"))?;

        assert_eq!(result.status, ExecutionStatus::Timeout);
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "cancel should interrupt promptly, took {elapsed:?}"
        );
        Ok(())
    }

    #[test]
    fn concurrent_short_execution_does_not_trip_long_execution()
    -> Result<(), Box<dyn std::error::Error>> {
        // Regression for the shared-epoch-counter bug: a short job's deadline
        // ticker must not spuriously time out a concurrent long job. The long
        // execution does bounded work well under its own wall/fuel budget while a
        // short execution with a tiny wall budget times out beside it.
        let long_wat = r#"
            (module
              (func (export "run") (param i64) (result i64)
                (local $i i64)
                (local.set $i (i64.const 50000000))
                (block $done
                  (loop $loop
                    (br_if $done (i64.eqz (local.get $i)))
                    (local.set $i (i64.sub (local.get $i) (i64.const 1)))
                    (br $loop)))
                (i64.const 7)))
        "#;
        let short_wat = r#"
            (module
              (func (export "run") (param i64) (result i64)
                (loop br 0)
                (i64.const 0)))
        "#;

        let long_engine = BeatboxEngine::new()?;
        let short_engine = long_engine.clone();

        let long = std::thread::spawn(move || {
            let mut request = request_for(long_wat, serde_json::json!({ "n": 0 }));
            request.policy.limits.wall_ms = 30_000;
            request.policy.limits.fuel = Some(1_000_000_000);
            long_engine.execute(request)
        });
        let short = std::thread::spawn(move || {
            let mut request = request_for(short_wat, serde_json::json!({ "n": 0 }));
            request.policy.limits.wall_ms = 25;
            request.policy.limits.fuel = Some(1_000_000_000);
            short_engine.execute(request)
        });

        let long = long
            .join()
            .map_err(|_| std::io::Error::other("long execution thread panicked"))??;
        let short = short
            .join()
            .map_err(|_| std::io::Error::other("short execution thread panicked"))??;

        assert_eq!(
            long.status,
            ExecutionStatus::Ok,
            "long job was spuriously killed: {}",
            long.stderr
        );
        assert_eq!(long.value, serde_json::json!(7));
        assert_eq!(short.status, ExecutionStatus::Timeout);
        Ok(())
    }

    #[test]
    fn unimplemented_lanes_are_denied_without_isolation_claims()
    -> Result<(), Box<dyn std::error::Error>> {
        let engine = BeatboxEngine::new()?;
        for lane in [
            Lane::PythonWasi,
            Lane::PythonNative,
            Lane::JsWasm,
            Lane::JsNative,
            Lane::Exec,
        ] {
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
}
