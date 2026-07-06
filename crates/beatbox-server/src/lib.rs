mod jobs;

use std::collections::{BTreeMap, HashMap};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::body::{Body, to_bytes};
use axum::extract::{Path as AxumPath, State};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use beatbox_core::{
    BrowserAdmissionDecision, BrowserAdmissionRequest, BrowserAdmissionResponse,
    BrowserArtifactMode, BrowserCredentialMode, BrowserIntegrationContract,
    BrowserProfilesResponse, BrowserSandboxAvailability, BrowserSandboxControl,
    BrowserSandboxLevel, BrowserSandboxProfile, BrowserSensitivity, BrowserSessionActor,
    CapabilitiesResponse, CapabilityLane, CapabilityLimits, CreateJobResponse, ErrorBody,
    ErrorResponse, ExecuteRequest, ExecutionResult, ExecutionStatus, JobRecord, Lane, Policy,
    Source,
};
use beatbox_engine::{BeatboxEngine, CancelFlag, EngineError};
use bytes::Bytes;
pub use jobs::JobStore;
use jobs::{CancelOutcome, JobStoreError};
use serde_json::{Value, json};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use url::{Host, Url};
use utoipa::OpenApi;

pub const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
pub const DEFAULT_SYNC_WALL_MS: u64 = 60_000;
pub const DEFAULT_JOB_WALL_MS: u64 = 5 * 60_000;
pub const DEFAULT_MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;
pub const DEFAULT_MAX_MEMORY_BYTES: u64 = 256 * 1024 * 1024;
pub const DEFAULT_MAX_OUTPUT_BYTES: u64 = 1024 * 1024;
pub const DEFAULT_MAX_FUEL: u64 = 100_000_000;
pub const DEFAULT_MAX_CONCURRENT_SYNC: usize = 8;
pub const DEFAULT_MAX_CONCURRENT_JOBS: usize = 4;

#[derive(Clone)]
pub struct ServerConfig {
    pub auth: AuthMode,
    pub engine: BeatboxEngine,
    pub jobs: JobStore,
    pub sync_wall_ms: u64,
    pub job_wall_ms: u64,
    pub max_memory_bytes: u64,
    pub max_output_bytes: u64,
    pub max_fuel: u64,
    pub max_request_bytes: usize,
    pub max_concurrent_sync: usize,
    pub max_concurrent_jobs: usize,
}

impl ServerConfig {
    /// Build a config with a default in-memory job store, returning an error if
    /// the store cannot be constructed. Prefer this over [`new`](Self::new) in
    /// library code that must not panic.
    pub fn try_new(engine: BeatboxEngine) -> Result<Self, JobStoreError> {
        Ok(Self {
            auth: AuthMode::None,
            engine,
            jobs: JobStore::in_memory()?,
            sync_wall_ms: DEFAULT_SYNC_WALL_MS,
            job_wall_ms: DEFAULT_JOB_WALL_MS,
            max_memory_bytes: DEFAULT_MAX_MEMORY_BYTES,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            max_fuel: DEFAULT_MAX_FUEL,
            max_request_bytes: DEFAULT_MAX_REQUEST_BYTES,
            max_concurrent_sync: DEFAULT_MAX_CONCURRENT_SYNC,
            max_concurrent_jobs: DEFAULT_MAX_CONCURRENT_JOBS,
        })
    }

    /// Build a config with a default in-memory job store.
    ///
    /// Panics only if the in-memory SQLite store cannot be constructed, which
    /// does not happen in practice. Use [`try_new`](Self::try_new) for a
    /// non-panicking constructor.
    pub fn new(engine: BeatboxEngine) -> Self {
        match Self::try_new(engine) {
            Ok(config) => config,
            Err(error) => panic!("default in-memory JobStore must construct: {error}"),
        }
    }

    pub fn with_job_store(mut self, jobs: JobStore) -> Self {
        self.jobs = jobs;
        self
    }

    pub fn with_sqlite_job_store(mut self, path: impl AsRef<Path>) -> Result<Self, JobStoreError> {
        self.jobs = JobStore::open(path)?;
        Ok(self)
    }

    pub fn with_max_request_bytes(mut self, max_request_bytes: usize) -> Self {
        self.max_request_bytes = max_request_bytes;
        self
    }

    pub fn with_max_concurrent_jobs(mut self, max_concurrent_jobs: usize) -> Self {
        self.max_concurrent_jobs = max_concurrent_jobs;
        self
    }

    pub fn with_max_concurrent_sync(mut self, max_concurrent_sync: usize) -> Self {
        self.max_concurrent_sync = max_concurrent_sync;
        self
    }
}

#[derive(Clone, Default)]
pub enum AuthMode {
    #[default]
    None,
    Required {
        token: AuthToken,
    },
}

/// A validated, non-empty authentication token. The inner value is private so an
/// `AuthMode::Required` carrying an empty token cannot be constructed from any
/// entry point — the only way in is [`AuthToken::new`], which rejects empties.
/// This closes the hole where `constant_time_eq(b"", b"")` let a request with an
/// empty `x-beatbox-api-key`/`Authorization: Bearer` header authorize.
#[derive(Clone)]
pub struct AuthToken(String);

impl AuthToken {
    pub fn new(token: impl Into<String>) -> Result<Self, AuthError> {
        let token = token.into();
        if token.trim().is_empty() {
            return Err(AuthError::EmptyToken);
        }
        Ok(Self(token))
    }

    fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("authentication token must not be empty")]
    EmptyToken,
}

impl AuthMode {
    /// Build a token-required auth mode, rejecting empty/whitespace tokens.
    pub fn required(token: impl Into<String>) -> Result<Self, AuthError> {
        Ok(Self::Required {
            token: AuthToken::new(token)?,
        })
    }

    fn is_required(&self) -> bool {
        matches!(self, Self::Required { .. })
    }
}

#[derive(Clone)]
struct AppState {
    started: Instant,
    config: ServerConfig,
    sync_permits: Arc<Semaphore>,
    job_permits: Arc<Semaphore>,
    // Cancel handles for jobs whose worker is in flight, so DELETE can interrupt
    // a running execution instead of only flipping the DB row. Entries are added
    // in spawn_job and removed when the worker finishes.
    job_cancels: Arc<Mutex<HashMap<String, CancelFlag>>>,
}

pub fn router(config: ServerConfig) -> Router {
    let sync_permits = Arc::new(Semaphore::new(config.max_concurrent_sync));
    let job_permits = Arc::new(Semaphore::new(config.max_concurrent_jobs));
    let state = AppState {
        started: Instant::now(),
        config,
        sync_permits,
        job_permits,
        job_cancels: Arc::new(Mutex::new(HashMap::new())),
    };
    Router::new()
        .route("/v1/health", get(health))
        .route("/openapi.json", get(openapi))
        .route("/v1/capabilities", get(capabilities))
        .route("/v1/browser/profiles", get(browser_profiles))
        .route("/v1/browser/admit", post(browser_admit))
        .route("/v1/execute", post(execute))
        .route("/v1/jobs", post(create_job))
        .route("/v1/jobs/{id}", get(get_job).delete(cancel_job))
        .route("/mcp", get(mcp_get).post(mcp_post))
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_s": state.started.elapsed().as_secs(),
    }))
}

async fn capabilities(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<CapabilitiesResponse>, ApiError> {
    state.authorize(&headers)?;
    Ok(Json(capabilities_json(&state.config)))
}

async fn browser_profiles(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<BrowserProfilesResponse>, ApiError> {
    state.authorize(&headers)?;
    Ok(Json(browser_profiles_response()))
}

async fn browser_admit(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request<Body>,
) -> Result<Json<BrowserAdmissionResponse>, ApiError> {
    state.authorize(&headers)?;
    let request = parse_json_body(&state, request).await?;
    validate_browser_admission_request(&request)
        .map_err(|message| ApiError::bad_request("invalid_browser_intent", message))?;
    Ok(Json(browser_admission_response(request)))
}

async fn execute(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request<Body>,
) -> Result<Json<ExecutionResult>, ApiError> {
    state.authorize(&headers)?;
    let request = parse_json_body(&state, request).await?;
    execute_sync(&state, request).await.map(Json)
}

async fn create_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request<Body>,
) -> Result<(StatusCode, Json<CreateJobResponse>), ApiError> {
    state.authorize(&headers)?;
    let request = parse_json_body(&state, request).await?;
    admit_execution_request(&state.config, &request, ExecutionMode::Job)?;
    let request = Arc::new(request);

    // Fast-path dedupe for an already-known idempotency key.
    let store = state.config.jobs.clone();
    let lookup = Arc::clone(&request);
    if let Some(job_id) = blocking_store(move || store.find_idempotent(&lookup))
        .await
        .map_err(ApiError::job_store)?
    {
        return Ok((StatusCode::ACCEPTED, Json(CreateJobResponse { job_id })));
    }

    // Insert-or-dedupe atomically. A duplicate (inserted == false) returns here
    // *without* consuming a concurrency permit, so a concurrent same-key
    // submission never spuriously 429s just because it lost a race for a permit.
    let store = state.config.jobs.clone();
    let create = Arc::clone(&request);
    let created = blocking_store(move || store.create_or_get(&create))
        .await
        .map_err(ApiError::job_store)?;
    let job_id = created.job_id;
    if !created.inserted {
        return Ok((StatusCode::ACCEPTED, Json(CreateJobResponse { job_id })));
    }

    // Genuinely new job: reserve a worker slot. If the cap is hit, roll back the
    // row we just inserted, then report 429.
    let permit = match state.job_permits.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            let store = state.config.jobs.clone();
            let cleanup_id = job_id.clone();
            // A keyless row can't be referenced by anyone else, so just delete it.
            // A keyed row may already have been handed to a concurrent same-key
            // request via dedupe; deleting it would 404 that client, so instead
            // fail it (releasing the key so a retry re-runs) — a terminal,
            // retrievable state rather than a vanished id.
            let has_key = request
                .idempotency_key
                .as_deref()
                .map(str::trim)
                .is_some_and(|key| !key.is_empty());
            blocking_store(move || {
                if has_key {
                    store.fail_queued_and_release_key(
                        &cleanup_id,
                        &ErrorBody::new(
                            "job_capacity",
                            "no worker slot was available; retry later",
                        ),
                    )
                } else {
                    store.delete_queued(&cleanup_id)
                }
            })
            .await
            .map_err(ApiError::job_store)?;
            return Err(ApiError::too_many(
                "job_concurrency_exceeded",
                format!(
                    "maximum concurrent jobs ({}) are already running",
                    state.config.max_concurrent_jobs
                ),
            ));
        }
    };
    let request = Arc::try_unwrap(request).unwrap_or_else(|arc| (*arc).clone());
    spawn_job(state, job_id.clone(), request, permit);
    Ok((StatusCode::ACCEPTED, Json(CreateJobResponse { job_id })))
}

async fn get_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<JobRecord>, ApiError> {
    state.authorize(&headers)?;
    let store = state.config.jobs.clone();
    let lookup_id = id.clone();
    blocking_store(move || store.get(&lookup_id))
        .await
        .map_err(ApiError::job_store)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("unknown job: {id}")))
}

async fn cancel_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<StatusCode, ApiError> {
    state.authorize(&headers)?;
    let store = state.config.jobs.clone();
    let cancel_id = id.clone();
    match blocking_store(move || store.cancel(&cancel_id))
        .await
        .map_err(ApiError::job_store)?
    {
        CancelOutcome::Canceled => {
            // Interrupt the in-flight worker (if any) so it stops promptly and
            // releases its concurrency permit instead of running to its full
            // wall/fuel budget. No-op if the job was still queued or already done.
            state.trip_cancel(&id);
            Ok(StatusCode::NO_CONTENT)
        }
        CancelOutcome::AlreadyTerminal => Err(ApiError::conflict(
            "job_already_terminal",
            format!("job {id} already finished and cannot be canceled"),
        )),
        CancelOutcome::NotFound => Err(ApiError::not_found(format!("unknown job: {id}"))),
    }
}

fn spawn_job(
    state: AppState,
    job_id: String,
    request: ExecuteRequest,
    permit: OwnedSemaphorePermit,
) {
    // Register the cancel handle synchronously (before the task is scheduled) so a
    // DELETE arriving right after the 202 can always find it.
    let cancel = state.register_cancel(&job_id);
    tokio::spawn(async move {
        let _permit = permit;
        // Ensures the cancel entry is removed on every exit path.
        let _cancel_guard = CancelGuard {
            state: state.clone(),
            job_id: job_id.clone(),
        };
        let store = state.config.jobs.clone();
        let mark_id = job_id.clone();
        match blocking_store(move || store.mark_running(&mark_id)).await {
            Ok(true) => {}
            Ok(false) => {
                tracing::info!(%job_id, "job was canceled before worker start");
                return;
            }
            Err(error) => {
                // Do not leave the job queued with no worker: fail it so callers
                // (and idempotent retries) see a terminal state.
                tracing::warn!(%job_id, %error, "failed to mark job running; failing job");
                let body = ErrorBody::new("job_worker", format!("failed to start worker: {error}"));
                fail_job(&state, &job_id, body).await;
                return;
            }
        }
        let engine = state.config.engine.clone();
        let result =
            tokio::task::spawn_blocking(move || engine.execute_cancellable(request, &cancel)).await;
        match result {
            Ok(Ok(result)) => {
                let store = state.config.jobs.clone();
                let complete_id = job_id.clone();
                if let Err(error) =
                    blocking_store(move || store.complete(&complete_id, &result)).await
                {
                    tracing::warn!(%job_id, %error, "failed to persist job result");
                }
            }
            Ok(Err(error)) => {
                fail_job(&state, &job_id, error.error_body()).await;
            }
            Err(error) => {
                fail_job(
                    &state,
                    &job_id,
                    ErrorBody::new("job_worker", error.to_string()),
                )
                .await;
            }
        }
    });
}

/// Removes a job's cancel handle from the registry when its worker finishes.
struct CancelGuard {
    state: AppState,
    job_id: String,
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        self.state.unregister_cancel(&self.job_id);
    }
}

async fn fail_job(state: &AppState, job_id: &str, body: ErrorBody) {
    let store = state.config.jobs.clone();
    let fail_id = job_id.to_string();
    if let Err(store_error) = blocking_store(move || store.fail(&fail_id, &body)).await {
        tracing::warn!(%job_id, %store_error, "failed to persist job failure");
    }
}

impl AppState {
    fn register_cancel(&self, job_id: &str) -> CancelFlag {
        let flag = CancelFlag::new();
        if let Ok(mut map) = self.job_cancels.lock() {
            map.insert(job_id.to_string(), flag.clone());
        }
        flag
    }

    fn unregister_cancel(&self, job_id: &str) {
        if let Ok(mut map) = self.job_cancels.lock() {
            map.remove(job_id);
        }
    }

    fn trip_cancel(&self, job_id: &str) {
        if let Ok(map) = self.job_cancels.lock()
            && let Some(flag) = map.get(job_id)
        {
            flag.cancel();
        }
    }

    fn authorize(&self, headers: &HeaderMap) -> Result<(), ApiError> {
        match &self.config.auth {
            AuthMode::None => Ok(()),
            AuthMode::Required { token } => {
                if api_key_authorized(headers, token) || bearer_authorized(headers, token) {
                    Ok(())
                } else {
                    Err(ApiError::unauthorized("missing or invalid API key"))
                }
            }
        }
    }
}

fn api_key_authorized(headers: &HeaderMap, token: &AuthToken) -> bool {
    headers
        .get("x-beatbox-api-key")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|actual| constant_time_eq(actual.as_bytes(), token.as_bytes()))
}

fn bearer_authorized(headers: &HeaderMap, token: &AuthToken) -> bool {
    let mut expected = b"Bearer ".to_vec();
    expected.extend_from_slice(token.as_bytes());
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|actual| constant_time_eq(actual.as_bytes(), &expected))
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    body: ErrorBody,
}

impl ApiError {
    fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            body: ErrorBody::new("unauthorized", message),
        }
    }

    fn unprocessable(error: EngineError) -> Self {
        Self::unprocessable_body(error.error_body())
    }

    fn unprocessable_body(body: ErrorBody) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            body,
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            body: ErrorBody::new("not_found", message),
        }
    }

    fn conflict(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            body: ErrorBody::new(code, message),
        }
    }

    fn job_store(error: JobStoreError) -> Self {
        match error {
            JobStoreError::IdempotencyConflict => Self {
                status: StatusCode::CONFLICT,
                body: ErrorBody::new("idempotency_conflict", error.to_string()),
            },
            error => Self {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                body: ErrorBody::new("job_store", error.to_string()),
            },
        }
    }

    fn bad_request(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            body: ErrorBody::new(code, message),
        }
    }

    fn too_many(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            body: ErrorBody::new(code, message),
        }
    }

    fn internal(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            body: ErrorBody::new(code, message),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(ErrorResponse { error: self.body })).into_response()
    }
}

async fn parse_json_body<T: serde::de::DeserializeOwned>(
    state: &AppState,
    request: Request<Body>,
) -> Result<T, ApiError> {
    require_json_content_type(request.headers())?;
    let bytes = read_limited_body(state, request).await?;
    serde_json::from_slice(&bytes)
        .map_err(|error| ApiError::bad_request("invalid_json", error.to_string()))
}

fn require_json_content_type(headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(content_type) = headers.get(CONTENT_TYPE) else {
        return Err(ApiError::bad_request(
            "unsupported_media_type",
            "content-type must be application/json",
        ));
    };
    if json_content_type(content_type) {
        Ok(())
    } else {
        Err(ApiError::bad_request(
            "unsupported_media_type",
            "content-type must be application/json",
        ))
    }
}

fn json_content_type(value: &HeaderValue) -> bool {
    value
        .to_str()
        .ok()
        .and_then(|value| value.split(';').next())
        .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case("application/json"))
}

async fn read_limited_body(state: &AppState, request: Request<Body>) -> Result<Bytes, ApiError> {
    let limit = state.config.max_request_bytes;
    to_bytes(request.into_body(), limit)
        .await
        .map_err(|error| ApiError::bad_request("body_limit", error.to_string()))
}

/// Run a synchronous job-store operation on a blocking thread so its
/// `std::sync::Mutex` + SQLite I/O never stalls a tokio worker under contention.
async fn blocking_store<T, F>(f: F) -> Result<T, JobStoreError>
where
    F: FnOnce() -> Result<T, JobStoreError> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        Err(join_error) => Err(JobStoreError::Worker(join_error.to_string())),
    }
}

async fn execute_sync(
    state: &AppState,
    request: ExecuteRequest,
) -> Result<ExecutionResult, ApiError> {
    admit_execution_request(&state.config, &request, ExecutionMode::Sync)?;
    let _permit = state
        .sync_permits
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::too_many(
                "sync_concurrency_exceeded",
                format!(
                    "maximum concurrent synchronous executions ({}) are already running",
                    state.config.max_concurrent_sync
                ),
            )
        })?;
    let engine = state.config.engine.clone();
    let result = tokio::task::spawn_blocking(move || engine.execute(request))
        .await
        .map_err(|error| ApiError::internal("execute_worker", error.to_string()))?;
    result.map_err(ApiError::unprocessable)
}

#[derive(Clone, Copy)]
enum ExecutionMode {
    Sync,
    Job,
}

fn admit_execution_request(
    config: &ServerConfig,
    request: &ExecuteRequest,
    mode: ExecutionMode,
) -> Result<(), ApiError> {
    admit_remote_source(request)?;
    let max_wall_ms = match mode {
        ExecutionMode::Sync => config.sync_wall_ms,
        ExecutionMode::Job => config.job_wall_ms,
    };
    if request.policy.limits.wall_ms > max_wall_ms {
        let code = match mode {
            ExecutionMode::Sync => "sync_limit_exceeded",
            ExecutionMode::Job => "job_limit_exceeded",
        };
        let target = match mode {
            ExecutionMode::Sync => "synchronous ceiling",
            ExecutionMode::Job => "asynchronous job ceiling",
        };
        return Err(ApiError::unprocessable_body(ErrorBody::new(
            code,
            format!(
                "policy.limits.wall_ms={} exceeds {target} {max_wall_ms}",
                request.policy.limits.wall_ms
            ),
        )));
    }
    if request.policy.limits.memory_bytes > config.max_memory_bytes {
        return Err(limit_exceeded(
            "memory_bytes",
            request.policy.limits.memory_bytes,
            config.max_memory_bytes,
        ));
    }
    if request.policy.limits.output_bytes > config.max_output_bytes {
        return Err(limit_exceeded(
            "output_bytes",
            request.policy.limits.output_bytes,
            config.max_output_bytes,
        ));
    }
    if let Some(fuel) = request.policy.limits.fuel
        && fuel > config.max_fuel
    {
        return Err(limit_exceeded("fuel", fuel, config.max_fuel));
    }
    Ok(())
}

fn limit_exceeded(field: &'static str, actual: u64, max: u64) -> ApiError {
    ApiError::unprocessable_body(ErrorBody::new(
        "limit_exceeded",
        format!("policy.limits.{field}={actual} exceeds daemon maximum {max}"),
    ))
}

fn admit_remote_source(request: &ExecuteRequest) -> Result<(), ApiError> {
    match (&request.lane, &request.source) {
        (_, Source::WasmFile { .. }) => Err(ApiError::unprocessable_body(ErrorBody::new(
            "host_file_source_denied",
            "remote API requests cannot reference daemon-local source paths; upload WAT or base64 Wasm bytes",
        ))),
        (_, Source::ModuleRef { .. }) => Err(ApiError::unprocessable_body(ErrorBody::new(
            "module_ref_unavailable",
            "module_ref storage is planned for M2.5 and is not implemented yet",
        ))),
        (Lane::Wasm, Source::WasmWat { .. } | Source::WasmBytesBase64 { .. }) => Ok(()),
        (Lane::Wasm, Source::Inline { .. }) => Err(source_lane_mismatch(
            &request.lane,
            &request.source,
            "wasm_wat or wasm_bytes_base64",
        )),
        (
            Lane::PythonWasi | Lane::PythonNative | Lane::JsWasm | Lane::JsNative,
            Source::Inline { .. },
        ) => Ok(()),
        (Lane::PythonWasi | Lane::PythonNative | Lane::JsWasm | Lane::JsNative, _) => Err(
            source_lane_mismatch(&request.lane, &request.source, "inline source code"),
        ),
        (Lane::Exec, Source::Inline { .. }) => Ok(()),
        (Lane::Exec, _) => Err(source_lane_mismatch(
            &request.lane,
            &request.source,
            "inline exec source",
        )),
    }
}

fn source_lane_mismatch(lane: &Lane, source: &Source, expected: &'static str) -> ApiError {
    ApiError::unprocessable_body(ErrorBody::new(
        "source_lane_mismatch",
        format!(
            "lane {lane:?} does not accept {} source; expected {expected}",
            source_kind(source)
        ),
    ))
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

fn capabilities_json(config: &ServerConfig) -> CapabilitiesResponse {
    let planned_wasmtime = BTreeMap::from([
        ("linux".to_string(), "planned".to_string()),
        ("macos".to_string(), "planned".to_string()),
    ]);
    let planned_os_jail = BTreeMap::from([
        ("linux".to_string(), "planned".to_string()),
        ("macos".to_string(), "planned_dev".to_string()),
    ]);
    CapabilitiesResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        lanes: vec![
            CapabilityLane {
                lane: Lane::Wasm,
                available: true,
                substrate: "wasmtime".to_string(),
                grade: BTreeMap::from([
                    ("linux".to_string(), "prod".to_string()),
                    ("macos".to_string(), "prod".to_string()),
                ]),
                mechanisms: vec![
                    "empty-linker".to_string(),
                    "fuel".to_string(),
                    "epoch-interruption".to_string(),
                    "store-limits".to_string(),
                ],
            },
            CapabilityLane {
                lane: Lane::PythonWasi,
                available: false,
                substrate: "wasmtime".to_string(),
                grade: planned_wasmtime.clone(),
                mechanisms: Vec::new(),
            },
            CapabilityLane {
                lane: Lane::JsWasm,
                available: false,
                substrate: "wasmtime".to_string(),
                grade: planned_wasmtime,
                mechanisms: Vec::new(),
            },
            CapabilityLane {
                lane: Lane::PythonNative,
                available: false,
                substrate: "os_jail".to_string(),
                grade: planned_os_jail.clone(),
                mechanisms: Vec::new(),
            },
            CapabilityLane {
                lane: Lane::JsNative,
                available: false,
                substrate: "os_jail".to_string(),
                grade: planned_os_jail.clone(),
                mechanisms: Vec::new(),
            },
            CapabilityLane {
                lane: Lane::Exec,
                available: false,
                substrate: "os_jail".to_string(),
                grade: planned_os_jail,
                mechanisms: Vec::new(),
            },
        ],
        limits: CapabilityLimits {
            sync_wall_ms: config.sync_wall_ms,
            job_wall_ms: config.job_wall_ms,
            default_wall_ms: Policy::default().limits.wall_ms,
            default_memory_bytes: Policy::default().limits.memory_bytes,
            default_output_bytes: Policy::default().limits.output_bytes,
            max_request_bytes: config.max_request_bytes,
            max_memory_bytes: config.max_memory_bytes,
            max_output_bytes: config.max_output_bytes,
            max_fuel: config.max_fuel,
            max_concurrent_sync: config.max_concurrent_sync,
            max_concurrent_jobs: config.max_concurrent_jobs,
        },
        engines: BTreeMap::from([("wasmtime".to_string(), "45".to_string())]),
        browser_sandbox: browser_profiles_response(),
    }
}

fn browser_profiles_response() -> BrowserProfilesResponse {
    BrowserProfilesResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        runnable_browser_sessions: false,
        default_level: None,
        integration: BrowserIntegrationContract {
            status: BrowserSandboxAvailability::Planned,
            consumer: "tempo".to_string(),
            endpoint: "/v1/browser/profiles".to_string(),
            admission_endpoint: "/v1/browser/admit".to_string(),
            selection_field: "browser_sandbox_level".to_string(),
            required_consumer_behavior: vec![
                "read this endpoint before offering browser work".to_string(),
                "call /v1/browser/admit before starting browser work".to_string(),
                "treat planned or unavailable profiles as non-runnable".to_string(),
                "do not downgrade to a weaker profile without an explicit user or policy decision".to_string(),
                "bind sensitive browsing to a fresh user-approved profile selection".to_string(),
            ],
        },
        profiles: vec![
            BrowserSandboxProfile {
                level: BrowserSandboxLevel::InstrumentedExternal,
                availability: BrowserSandboxAvailability::Unavailable,
                summary: "Drive a caller-supplied browser for local debugging only.".to_string(),
                controls: browser_profile_controls(&BrowserSandboxLevel::InstrumentedExternal),
                isolation_boundary: "none; not a sandbox".to_string(),
                privacy_controls: vec!["no ambient credential claim".to_string()],
                egress_controls: vec!["caller browser network stack".to_string()],
                credential_controls: vec![
                    "must never be selected for sensitive work".to_string(),
                    "caller remains responsible for browser profile contents".to_string(),
                ],
                storage_controls: vec!["caller browser profile storage".to_string()],
                encryption_claims: vec!["no beatbox encryption claim".to_string()],
                non_goals: vec!["site isolation".to_string(), "secret handling".to_string()],
                downgrade_reasons: vec![
                    "no browser adapter is implemented".to_string(),
                    "external browser state cannot be isolated by beatbox".to_string(),
                ],
            },
            BrowserSandboxProfile {
                level: BrowserSandboxLevel::EphemeralProfile,
                availability: BrowserSandboxAvailability::Planned,
                summary: "Launch a fresh browser profile with no host cookies, passwords, or extensions.".to_string(),
                controls: browser_profile_controls(&BrowserSandboxLevel::EphemeralProfile),
                isolation_boundary: "browser process plus temporary profile directory".to_string(),
                privacy_controls: vec![
                    "fresh profile per task".to_string(),
                    "no host browser profile reuse".to_string(),
                    "automatic profile deletion after task completion".to_string(),
                ],
                egress_controls: vec!["default-deny proxy hook required before sensitive use".to_string()],
                credential_controls: vec![
                    "no ambient password manager".to_string(),
                    "explicit user-provided secrets only through a future scoped secret channel".to_string(),
                ],
                storage_controls: vec!["temporary profile state only".to_string()],
                encryption_claims: vec![
                    "no at-rest encryption claim because state is not persisted".to_string(),
                ],
                non_goals: vec!["kernel isolation".to_string(), "malicious browser exploit containment".to_string()],
                downgrade_reasons: vec!["browser launcher and teardown are not implemented".to_string()],
            },
            BrowserSandboxProfile {
                level: BrowserSandboxLevel::NetworkSuppressed,
                availability: BrowserSandboxAvailability::Planned,
                summary: "Ephemeral browser with egress routed through an allowlist-capable proxy.".to_string(),
                controls: browser_profile_controls(&BrowserSandboxLevel::NetworkSuppressed),
                isolation_boundary: "ephemeral profile plus enforced proxy egress boundary".to_string(),
                privacy_controls: vec![
                    "fresh profile per task".to_string(),
                    "request log available to the control plane".to_string(),
                ],
                egress_controls: vec![
                    "raw network disabled".to_string(),
                    "domain and port allowlists".to_string(),
                    "localhost, LAN, and metadata IP ranges denied by default".to_string(),
                ],
                credential_controls: vec!["no ambient credentials".to_string()],
                storage_controls: vec!["temporary profile state only".to_string()],
                encryption_claims: vec!["no persisted-state encryption claim".to_string()],
                non_goals: vec!["full OS process containment".to_string()],
                downgrade_reasons: vec!["egress proxy is not implemented".to_string()],
            },
            BrowserSandboxProfile {
                level: BrowserSandboxLevel::SealedState,
                availability: BrowserSandboxAvailability::Planned,
                summary: "Persist selected artifacts or profiles only under an explicit encryption key policy.".to_string(),
                controls: browser_profile_controls(&BrowserSandboxLevel::SealedState),
                isolation_boundary: "ephemeral or OS-isolated browser plus encrypted artifact store".to_string(),
                privacy_controls: vec![
                    "artifact allowlist required".to_string(),
                    "no automatic cookie or password persistence".to_string(),
                ],
                egress_controls: vec!["inherits selected runtime profile egress controls".to_string()],
                credential_controls: vec![
                    "caller-supplied key material required".to_string(),
                    "beatbox must not silently fall back to plaintext persistence".to_string(),
                ],
                storage_controls: vec![
                    "encrypted persisted artifacts only".to_string(),
                    "plaintext temporary workspace deleted after seal".to_string(),
                ],
                encryption_claims: vec![
                    "planned only; no encryption is currently applied".to_string(),
                    "future claim must name algorithm, key source, and plaintext lifetime".to_string(),
                ],
                non_goals: vec!["protecting data from the live browser process".to_string()],
                downgrade_reasons: vec!["encrypted artifact store is not implemented".to_string()],
            },
            BrowserSandboxProfile {
                level: BrowserSandboxLevel::OsIsolated,
                availability: BrowserSandboxAvailability::Planned,
                summary: "Run the browser inside an OS jail or microVM with explicit filesystem and network policy.".to_string(),
                controls: browser_profile_controls(&BrowserSandboxLevel::OsIsolated),
                isolation_boundary: "host OS jail or microVM boundary".to_string(),
                privacy_controls: vec![
                    "fresh guest profile".to_string(),
                    "no host home-directory mounts by default".to_string(),
                ],
                egress_controls: vec![
                    "guest egress routed through policy proxy".to_string(),
                    "control-plane loopback denied unless explicitly granted".to_string(),
                ],
                credential_controls: vec!["scoped secret injection only".to_string()],
                storage_controls: vec!["discardable guest disk by default".to_string()],
                encryption_claims: vec!["guest disk encryption depends on selected substrate and is not yet claimed".to_string()],
                non_goals: vec!["defense against a malicious host kernel".to_string()],
                downgrade_reasons: vec!["OS jail or microVM browser substrate is not implemented".to_string()],
            },
            BrowserSandboxProfile {
                level: BrowserSandboxLevel::RemoteIsolated,
                availability: BrowserSandboxAvailability::Planned,
                summary: "Run browsing on a remote disposable worker with no local credential or filesystem access.".to_string(),
                controls: browser_profile_controls(&BrowserSandboxLevel::RemoteIsolated),
                isolation_boundary: "remote worker boundary plus authenticated control channel".to_string(),
                privacy_controls: vec![
                    "no local browser profile access".to_string(),
                    "remote artifact return allowlist".to_string(),
                ],
                egress_controls: vec!["remote policy proxy required".to_string()],
                credential_controls: vec!["explicit scoped secret transfer only".to_string()],
                storage_controls: vec!["remote workspace destroyed after task".to_string()],
                encryption_claims: vec![
                    "transport encryption required".to_string(),
                    "remote at-rest encryption must be reported by worker capability metadata".to_string(),
                ],
                non_goals: vec!["trusting an unknown remote operator".to_string()],
                downgrade_reasons: vec!["remote worker protocol is not implemented".to_string()],
            },
        ],
    }
}

fn browser_admission_response(request: BrowserAdmissionRequest) -> BrowserAdmissionResponse {
    let requested_profile_controls = browser_profile_controls(&request.requested_level);
    let missing_controls: Vec<_> = request
        .required_controls
        .iter()
        .filter(|control| !requested_profile_controls.contains(control))
        .cloned()
        .collect();
    let level_satisfies_requested_controls = missing_controls.is_empty();
    let mut reasons = vec![
        "beatbox does not currently expose a runnable browser sandbox".to_string(),
        "no browser launcher, teardown path, egress boundary, storage policy, or encryption behavior is implemented"
            .to_string(),
    ];
    match &request.requested_level {
        BrowserSandboxLevel::InstrumentedExternal => {
            reasons.push(
                "instrumented_external is explicitly not a sandbox and is unavailable for sensitive work"
                    .to_string(),
            );
        }
        BrowserSandboxLevel::EphemeralProfile => {
            reasons.push("ephemeral profile launcher and cleanup are not implemented".to_string());
        }
        BrowserSandboxLevel::NetworkSuppressed => {
            reasons.push("network suppression proxy is not implemented".to_string());
        }
        BrowserSandboxLevel::SealedState => {
            reasons.push("encrypted artifact sealing is not implemented".to_string());
        }
        BrowserSandboxLevel::OsIsolated => {
            reasons.push("OS jail or microVM browser substrate is not implemented".to_string());
        }
        BrowserSandboxLevel::RemoteIsolated => {
            reasons
                .push("remote disposable browser worker protocol is not implemented".to_string());
        }
    }
    if request.allow_downgrade {
        reasons.push(
            "downgrade was allowed, but no weaker browser profile is currently runnable"
                .to_string(),
        );
    }
    if matches!(&request.sensitivity, BrowserSensitivity::Sensitive) {
        reasons.push(
            "sensitive browser work requires an explicitly available isolated profile".to_string(),
        );
    }
    let mut intent_warnings = Vec::new();
    if request.target_origins.is_empty() {
        intent_warnings.push(
            "no target origins were declared; future runnable sensitive sessions will require an origin allowlist"
                .to_string(),
        );
    }
    if matches!(
        &request.credential_mode,
        BrowserCredentialMode::UserMediated | BrowserCredentialMode::ScopedSecrets
    ) {
        reasons.push(format!(
            "credential mode `{}` is not implemented by any runnable browser profile",
            browser_credential_mode_wire_name(&request.credential_mode)
        ));
    }
    if matches!(
        &request.artifact_mode,
        BrowserArtifactMode::ExplicitDownloads | BrowserArtifactMode::SealedArtifacts
    ) {
        reasons.push(format!(
            "artifact mode `{}` is not implemented by any runnable browser profile",
            browser_artifact_mode_wire_name(&request.artifact_mode)
        ));
    }
    if !level_satisfies_requested_controls {
        reasons.push(format!(
            "requested profile does not satisfy required controls: {}",
            missing_controls
                .iter()
                .map(browser_control_wire_name)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    BrowserAdmissionResponse {
        decision: BrowserAdmissionDecision::Rejected,
        runnable_browser_sessions: false,
        requested_level: request.requested_level.clone(),
        selected_level: None,
        actor: request.actor,
        sensitivity: request.sensitivity,
        target_origins: request.target_origins,
        credential_mode: request.credential_mode,
        artifact_mode: request.artifact_mode,
        requested_controls: request.required_controls,
        requested_profile_controls,
        missing_controls,
        level_satisfies_requested_controls,
        intent_warnings,
        downgrade_allowed: request.allow_downgrade,
        reasons,
        required_next_steps: vec![
            "implement a browser launcher with a fresh profile per task".to_string(),
            "enforce network egress through a deny-by-default policy boundary".to_string(),
            "prove teardown removes plaintext browser state".to_string(),
            "add production-path tests before marking any browser profile available".to_string(),
        ],
        profiles_endpoint: "/v1/browser/profiles".to_string(),
    }
}

fn browser_profile_controls(level: &BrowserSandboxLevel) -> Vec<BrowserSandboxControl> {
    match level {
        BrowserSandboxLevel::InstrumentedExternal => Vec::new(),
        BrowserSandboxLevel::EphemeralProfile => vec![
            BrowserSandboxControl::FreshProfile,
            BrowserSandboxControl::NoAmbientCredentials,
            BrowserSandboxControl::TeardownProof,
        ],
        BrowserSandboxLevel::NetworkSuppressed => vec![
            BrowserSandboxControl::FreshProfile,
            BrowserSandboxControl::NoAmbientCredentials,
            BrowserSandboxControl::EgressPolicy,
            BrowserSandboxControl::LocalNetworkBlock,
            BrowserSandboxControl::TeardownProof,
        ],
        BrowserSandboxLevel::SealedState => vec![
            BrowserSandboxControl::FreshProfile,
            BrowserSandboxControl::NoAmbientCredentials,
            BrowserSandboxControl::SealedArtifacts,
            BrowserSandboxControl::TeardownProof,
        ],
        BrowserSandboxLevel::OsIsolated => vec![
            BrowserSandboxControl::FreshProfile,
            BrowserSandboxControl::NoAmbientCredentials,
            BrowserSandboxControl::EgressPolicy,
            BrowserSandboxControl::LocalNetworkBlock,
            BrowserSandboxControl::OsProcessIsolation,
            BrowserSandboxControl::TeardownProof,
        ],
        BrowserSandboxLevel::RemoteIsolated => vec![
            BrowserSandboxControl::FreshProfile,
            BrowserSandboxControl::NoAmbientCredentials,
            BrowserSandboxControl::EgressPolicy,
            BrowserSandboxControl::RemoteWorkerIsolation,
            BrowserSandboxControl::TeardownProof,
        ],
    }
}

fn validate_browser_admission_request(request: &BrowserAdmissionRequest) -> Result<(), String> {
    const MAX_TARGET_ORIGINS: usize = 16;
    if request.target_origins.len() > MAX_TARGET_ORIGINS {
        return Err(format!(
            "browser admission target_origins must contain at most {MAX_TARGET_ORIGINS} entries"
        ));
    }
    for origin in &request.target_origins {
        validate_browser_target_origin(origin)?;
    }
    Ok(())
}

fn validate_browser_target_origin(origin: &str) -> Result<(), String> {
    if origin.is_empty() || origin.trim() != origin {
        return Err("browser admission target_origins entries must be non-empty origins without surrounding whitespace".to_string());
    }
    let url = Url::parse(origin).map_err(|error| {
        format!("browser admission target origin `{origin}` is invalid: {error}")
    })?;
    match url.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(format!(
                "browser admission target origin `{origin}` uses unsupported scheme `{scheme}`"
            ));
        }
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(format!(
            "browser admission target origin `{origin}` must not contain credentials"
        ));
    }
    if url.host().is_none() {
        return Err(format!(
            "browser admission target origin `{origin}` must include a host"
        ));
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err(format!(
            "browser admission target origin `{origin}` must be only scheme, host, and optional port"
        ));
    }
    if let Some(host) = url.host() {
        validate_browser_target_host(origin, host)?;
    }
    Ok(())
}

fn validate_browser_target_host(origin: &str, host: Host<&str>) -> Result<(), String> {
    match host {
        Host::Domain(domain) => {
            let domain = domain.trim_end_matches('.').to_ascii_lowercase();
            if domain == "localhost" || domain.ends_with(".localhost") {
                return Err(format!(
                    "browser admission target origin `{origin}` must not target localhost"
                ));
            }
        }
        Host::Ipv4(addr) => {
            if ipv4_is_restricted_browser_target(addr) {
                return Err(format!(
                    "browser admission target origin `{origin}` must not target local or private IPv4 space"
                ));
            }
        }
        Host::Ipv6(addr) => {
            if let Some(mapped) = addr.to_ipv4_mapped()
                && ipv4_is_restricted_browser_target(mapped)
            {
                return Err(format!(
                    "browser admission target origin `{origin}` must not target local or private IPv4-mapped IPv6 space"
                ));
            }
            if addr.is_loopback()
                || addr.is_unspecified()
                || ipv6_is_unique_local(addr)
                || ipv6_is_unicast_link_local(addr)
            {
                return Err(format!(
                    "browser admission target origin `{origin}` must not target local or private IPv6 space"
                ));
            }
        }
    }
    Ok(())
}

fn ipv4_is_restricted_browser_target(addr: Ipv4Addr) -> bool {
    let octets = addr.octets();
    addr.is_loopback()
        || addr.is_private()
        || addr.is_link_local()
        || addr.is_unspecified()
        || addr.is_broadcast()
        || addr.is_multicast()
        || addr.is_documentation()
        || (octets[0] == 100 && (octets[1] & 0b1100_0000) == 0b0100_0000)
}

fn ipv6_is_unique_local(addr: Ipv6Addr) -> bool {
    (addr.segments()[0] & 0xfe00) == 0xfc00
}

fn ipv6_is_unicast_link_local(addr: Ipv6Addr) -> bool {
    (addr.segments()[0] & 0xffc0) == 0xfe80
}

fn browser_control_wire_name(control: &BrowserSandboxControl) -> &'static str {
    match control {
        BrowserSandboxControl::FreshProfile => "fresh_profile",
        BrowserSandboxControl::NoAmbientCredentials => "no_ambient_credentials",
        BrowserSandboxControl::EgressPolicy => "egress_policy",
        BrowserSandboxControl::LocalNetworkBlock => "local_network_block",
        BrowserSandboxControl::SealedArtifacts => "sealed_artifacts",
        BrowserSandboxControl::OsProcessIsolation => "os_process_isolation",
        BrowserSandboxControl::RemoteWorkerIsolation => "remote_worker_isolation",
        BrowserSandboxControl::TeardownProof => "teardown_proof",
    }
}

fn browser_credential_mode_wire_name(mode: &BrowserCredentialMode) -> &'static str {
    match mode {
        BrowserCredentialMode::NoCredentials => "no_credentials",
        BrowserCredentialMode::UserMediated => "user_mediated",
        BrowserCredentialMode::ScopedSecrets => "scoped_secrets",
    }
}

fn browser_artifact_mode_wire_name(mode: &BrowserArtifactMode) -> &'static str {
    match mode {
        BrowserArtifactMode::Discard => "discard",
        BrowserArtifactMode::ExplicitDownloads => "explicit_downloads",
        BrowserArtifactMode::SealedArtifacts => "sealed_artifacts",
    }
}

async fn openapi() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

/// The canonical OpenAPI document, pretty-printed exactly as it is served at
/// `GET /openapi.json` and checked into `sdks/openapi.json`.
///
/// This is the single source of truth the SDK fleet is generated/synced
/// against. A drift test (`tests/openapi_drift.rs`) asserts the committed
/// `sdks/openapi.json` matches this byte-for-byte, so the spec can never
/// silently diverge from the server that implements it.
pub fn openapi_spec_json() -> String {
    match ApiDoc::openapi().to_pretty_json() {
        // Match the on-disk convention: pretty JSON with a trailing newline.
        Ok(json) => format!("{json}\n"),
        // The document is a compile-time-fixed tree of plain data, so JSON
        // serialization of it cannot fail; a failure here is a build bug.
        Err(e) => panic!("beatbox OpenAPI document failed to serialize: {e}"),
    }
}

#[derive(OpenApi)]
#[openapi(
    info(title = "beatbox API", version = "0.1.0"),
    paths(
        openapi_paths::health,
        openapi_paths::capabilities,
        openapi_paths::browser_profiles,
        openapi_paths::browser_admit,
        openapi_paths::execute,
        openapi_paths::create_job,
        openapi_paths::get_job,
        openapi_paths::cancel_job,
        openapi_paths::mcp_post
    ),
    components(schemas(
        ExecuteRequest,
        beatbox_core::Lane,
        Source,
        beatbox_core::Policy,
        beatbox_core::FsPolicy,
        beatbox_core::Mount,
        beatbox_core::MountMode,
        beatbox_core::NetPolicy,
        beatbox_core::Secret,
        beatbox_core::SecretExpose,
        beatbox_core::Limits,
        beatbox_core::Determinism,
        ExecutionResult,
        beatbox_core::ExecutionStatus,
        beatbox_core::Metrics,
        beatbox_core::EffectiveIsolation,
        beatbox_core::EgressRecord,
        ErrorBody,
        ErrorResponse,
        CreateJobResponse,
        JobRecord,
        beatbox_core::JobStatus,
        beatbox_core::BrowserProfilesResponse,
        beatbox_core::BrowserIntegrationContract,
        beatbox_core::BrowserSandboxProfile,
        beatbox_core::BrowserSandboxLevel,
        beatbox_core::BrowserSandboxAvailability,
        beatbox_core::BrowserSandboxControl,
        beatbox_core::BrowserCredentialMode,
        beatbox_core::BrowserArtifactMode,
        beatbox_core::BrowserAdmissionRequest,
        beatbox_core::BrowserAdmissionResponse,
        beatbox_core::BrowserAdmissionDecision,
        beatbox_core::BrowserSessionActor,
        beatbox_core::BrowserSensitivity,
        beatbox_core::CapabilitiesResponse,
        beatbox_core::CapabilityLane,
        beatbox_core::CapabilityLimits,
    )),
    tags(
        (name = "v1", description = "beatbox REST API"),
        (name = "mcp", description = "stateless MCP JSON-RPC endpoint")
    )
)]
struct ApiDoc;

#[allow(dead_code)]
mod openapi_paths {
    use beatbox_core::{
        BrowserAdmissionRequest, BrowserAdmissionResponse, BrowserProfilesResponse,
        CapabilitiesResponse, CreateJobResponse, ErrorResponse, ExecuteRequest, ExecutionResult,
        JobRecord,
    };

    #[utoipa::path(
        get,
        path = "/v1/health",
        tag = "v1",
        responses((status = 200, description = "Daemon health"))
    )]
    pub fn health() {}

    #[utoipa::path(
        get,
        path = "/v1/capabilities",
        tag = "v1",
        responses(
            (status = 200, description = "Lane availability and host limits", body = CapabilitiesResponse),
            (status = 401, description = "Missing or invalid bearer token", body = ErrorResponse)
        )
    )]
    pub fn capabilities() {}

    #[utoipa::path(
        get,
        path = "/v1/browser/profiles",
        tag = "v1",
        responses(
            (status = 200, description = "Browser sandbox profile discovery contract", body = BrowserProfilesResponse),
            (status = 401, description = "Missing or invalid bearer token", body = ErrorResponse)
        )
    )]
    pub fn browser_profiles() {}

    #[utoipa::path(
        post,
        path = "/v1/browser/admit",
        tag = "v1",
        request_body = BrowserAdmissionRequest,
        responses(
            (status = 200, description = "Browser sandbox admission decision", body = BrowserAdmissionResponse),
            (status = 400, description = "Malformed, oversized, or non-JSON request body", body = ErrorResponse),
            (status = 401, description = "Missing or invalid bearer token", body = ErrorResponse)
        )
    )]
    pub fn browser_admit() {}

    #[utoipa::path(
        post,
        path = "/v1/execute",
        tag = "v1",
        request_body = ExecuteRequest,
        responses(
            (status = 200, description = "ExecutionResult", body = ExecutionResult),
            (status = 401, description = "Missing or invalid bearer token", body = ErrorResponse),
            (status = 422, description = "Protocol, source, policy, or sync-limit rejection", body = ErrorResponse)
        )
    )]
    pub fn execute() {}

    #[utoipa::path(
        post,
        path = "/v1/jobs",
        tag = "v1",
        request_body = ExecuteRequest,
        responses(
            (status = 202, description = "Created asynchronous job", body = CreateJobResponse),
            (status = 401, description = "Missing or invalid bearer token", body = ErrorResponse),
            (status = 409, description = "Idempotency key reused with a different payload", body = ErrorResponse),
            (status = 422, description = "Protocol, source, policy, or job-limit rejection", body = ErrorResponse),
            (status = 429, description = "Concurrency cap exhausted", body = ErrorResponse)
        )
    )]
    pub fn create_job() {}

    #[utoipa::path(
        get,
        path = "/v1/jobs/{id}",
        tag = "v1",
        params(("id" = String, Path, description = "Job id")),
        responses(
            (status = 200, description = "JobRecord", body = JobRecord),
            (status = 401, description = "Missing or invalid bearer token", body = ErrorResponse),
            (status = 404, description = "Unknown job", body = ErrorResponse)
        )
    )]
    pub fn get_job() {}

    #[utoipa::path(
        delete,
        path = "/v1/jobs/{id}",
        tag = "v1",
        params(("id" = String, Path, description = "Job id")),
        responses(
            (status = 204, description = "Canceled job (or already canceled)"),
            (status = 401, description = "Missing or invalid bearer token"),
            (status = 404, description = "Unknown job"),
            (status = 409, description = "Job already finished and cannot be canceled")
        )
    )]
    pub fn cancel_job() {}

    #[utoipa::path(
        post,
        path = "/mcp",
        tag = "mcp",
        responses(
            (status = 200, description = "JSON-RPC response"),
            (status = 202, description = "JSON-RPC notification accepted"),
            (status = 403, description = "Origin not allowed")
        )
    )]
    pub fn mcp_post() {}
}

pub fn origin_allowed(headers: &HeaderMap) -> bool {
    match headers.get("origin").and_then(|value| value.to_str().ok()) {
        None => true,
        Some(origin) => local_origin_allowed(origin),
    }
}

fn local_origin_allowed(origin: &str) -> bool {
    let Some(url) = parse_origin(origin) else {
        return false;
    };
    match url.host() {
        Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(addr)) => addr == Ipv4Addr::LOCALHOST,
        Some(Host::Ipv6(addr)) => addr == Ipv6Addr::LOCALHOST,
        None => false,
    }
}

fn parse_origin(origin: &str) -> Option<Url> {
    let url = Url::parse(origin.trim().trim_end_matches('/')).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    url.host()?;
    Some(url)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let max_len = left.len().max(right.len());
    let mut diff = left.len() ^ right.len();
    for index in 0..max_len {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        diff |= usize::from(left_byte ^ right_byte);
    }
    diff == 0
}

async fn mcp_get() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        [("allow", "POST")],
        "this MCP endpoint does not offer a server-initiated stream",
    )
        .into_response()
}

async fn mcp_post(State(state): State<AppState>, request: Request<Body>) -> Response {
    let (parts, body) = request.into_parts();
    let headers = parts.headers;
    if !origin_allowed(&headers) {
        return json_response(
            StatusCode::FORBIDDEN,
            json!({"jsonrpc": "2.0", "error": {"code": -32600, "message": "origin not allowed"}}),
        );
    }
    if state.config.auth.is_required()
        && let Err(error) = state.authorize(&headers)
    {
        return json_response(
            StatusCode::UNAUTHORIZED,
            json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32001, "message": error.body.message}}),
        );
    }
    if let Err(error) = require_json_content_type(&headers) {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32600, "message": error.body.message}}),
        );
    }

    let body = match to_bytes(body, state.config.max_request_bytes).await {
        Ok(body) => body,
        Err(error) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32600, "message": format!("body limit exceeded: {error}")}}),
            );
        }
    };

    let message: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(error) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {error}")}}),
            );
        }
    };

    let Some(id) = message.get("id").filter(|id| !id.is_null()).cloned() else {
        return StatusCode::ACCEPTED.into_response();
    };

    let method = message["method"].as_str().unwrap_or_default();
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    if matches!(method, "tools/list" | "tools/call")
        && let Err(error) = state.authorize(&headers)
    {
        return json_response(
            StatusCode::UNAUTHORIZED,
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32001, "message": error.body.message}}),
        );
    }
    let reply = match method {
        "initialize" => Ok(json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "beatbox", "version": env!("CARGO_PKG_VERSION")},
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": mcp_tools()})),
        "tools/call" => mcp_tools_call(&state, &headers, &params).await,
        other => Err((-32601, format!("method not found: {other}"))),
    };

    let body = match reply {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, message)) => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
        }
    };
    json_response(StatusCode::OK, body)
}

fn mcp_tools() -> Value {
    json!([
        {
            "name": "run_wasm",
            "description": "Run a WebAssembly text or base64 module in the hermetic Wasmtime lane.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "oneOf": [{"required": ["wat"]}, {"required": ["wasm_base64"]}],
                "properties": {
                    "wat": {"type": "string"},
                    "wasm_base64": {"type": "string"},
                    "input": {},
                    "entrypoint": {"type": "string"},
                    "timeout_ms": {"type": "integer"},
                    "memory_bytes": {"type": "integer"},
                    "fuel": {"type": "integer"}
                }
            }
        },
        {
            "name": "run_python",
            "description": "Run Python source in the planned python-wasi lane.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "required": ["code"],
                "properties": {
                    "code": {"type": "string"},
                    "input": {},
                    "timeout_ms": {"type": "integer"},
                    "memory_bytes": {"type": "integer"}
                }
            }
        },
        {
            "name": "run_javascript",
            "description": "Run JavaScript source in the planned js-wasm lane.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "required": ["code"],
                "properties": {
                    "code": {"type": "string"},
                    "input": {},
                    "timeout_ms": {"type": "integer"},
                    "memory_bytes": {"type": "integer"}
                }
            }
        },
        {
            "name": "get_capabilities",
            "description": "Return beatbox lane availability.",
            "inputSchema": {"type": "object", "additionalProperties": false}
        },
        {
            "name": "get_browser_profiles",
            "description": "Return beatbox browser sandbox profile discovery metadata for Tempo-style integrations.",
            "inputSchema": {"type": "object", "additionalProperties": false}
        },
        {
            "name": "admit_browser_session",
            "description": "Return a fail-closed browser sandbox admission decision for a requested actor, sensitivity, and sandbox level.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "required": ["requested_level", "actor", "sensitivity"],
                "properties": {
                    "requested_level": {
                        "type": "string",
                        "enum": [
                            "instrumented_external",
                            "ephemeral_profile",
                            "network_suppressed",
                            "sealed_state",
                            "os_isolated",
                            "remote_isolated"
                        ]
                    },
                    "actor": {"type": "string", "enum": ["agent", "human"]},
                    "sensitivity": {"type": "string", "enum": ["public", "sensitive"]},
                    "target_origins": {
                        "type": "array",
                        "description": "Bare public HTTP(S) origins allowed for the requested browser session. Entries must contain only scheme, host, and optional port; credentials, paths, queries, fragments, localhost, private/LAN IP ranges, and link-local metadata targets are rejected.",
                        "maxItems": 16,
                        "items": {"type": "string"}
                    },
                    "credential_mode": {
                        "type": "string",
                        "description": "Credential posture requested for the session. Non-default modes remain fail-closed until a real browser substrate implements them.",
                        "enum": ["no_credentials", "user_mediated", "scoped_secrets"]
                    },
                    "artifact_mode": {
                        "type": "string",
                        "description": "Artifact persistence posture requested for the session. Non-default modes remain fail-closed until storage and sealing are implemented.",
                        "enum": ["discard", "explicit_downloads", "sealed_artifacts"]
                    },
                    "required_controls": {
                        "type": "array",
                        "items": {
                            "type": "string",
                            "enum": [
                                "fresh_profile",
                                "no_ambient_credentials",
                                "egress_policy",
                                "local_network_block",
                                "sealed_artifacts",
                                "os_process_isolation",
                                "remote_worker_isolation",
                                "teardown_proof"
                            ]
                        }
                    },
                    "allow_downgrade": {"type": "boolean"},
                    "task_label": {"type": "string"}
                }
            }
        }
    ])
}

async fn mcp_tools_call(
    state: &AppState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, (i64, String)> {
    state
        .authorize(headers)
        .map_err(|error| (-32001, error.body.message))?;
    let name = params["name"]
        .as_str()
        .ok_or((-32602, "tools/call requires params.name".to_string()))?;
    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

    match name {
        "get_capabilities" => {
            mcp_tool_arguments(&arguments, "get_capabilities", &[])?;
            let capabilities = capabilities_json(&state.config);
            Ok(json!({
                "content": [{"type": "text", "text": "beatbox capabilities"}],
                "structuredContent": capabilities,
                "isError": false,
            }))
        }
        "get_browser_profiles" => {
            mcp_tool_arguments(&arguments, "get_browser_profiles", &[])?;
            let profiles = serde_json::to_value(browser_profiles_response()).map_err(|error| {
                (
                    -32603,
                    format!("failed to serialize browser profiles: {error}"),
                )
            })?;
            Ok(json!({
                "content": [{"type": "text", "text": "beatbox browser sandbox profiles"}],
                "structuredContent": profiles,
                "isError": false,
            }))
        }
        "admit_browser_session" => {
            let request = mcp_browser_admission_request(&arguments)?;
            let decision = browser_admission_response(request);
            let is_error = decision.decision != BrowserAdmissionDecision::Accepted;
            let decision = serde_json::to_value(decision).map_err(|error| {
                (
                    -32603,
                    format!("failed to serialize browser admission decision: {error}"),
                )
            })?;
            Ok(json!({
                "content": [{"type": "text", "text": "beatbox browser admission decision"}],
                "structuredContent": decision,
                "isError": is_error,
            }))
        }
        "run_wasm" => {
            let request = mcp_run_wasm_request(&arguments)?;
            let result = execute_sync(state, request)
                .await
                .map_err(api_error_to_rpc)?;
            tool_result(result)
        }
        "run_python" => {
            let request = mcp_run_code_request(&arguments, Lane::PythonWasi, "run_python")?;
            let result = execute_sync(state, request)
                .await
                .map_err(api_error_to_rpc)?;
            tool_result(result)
        }
        "run_javascript" => {
            let request = mcp_run_code_request(&arguments, Lane::JsWasm, "run_javascript")?;
            let result = execute_sync(state, request)
                .await
                .map_err(api_error_to_rpc)?;
            tool_result(result)
        }
        other => Err((-32602, format!("unknown tool: {other}"))),
    }
}

fn mcp_browser_admission_request(
    arguments: &Value,
) -> Result<BrowserAdmissionRequest, (i64, String)> {
    let arguments = mcp_tool_arguments(
        arguments,
        "admit_browser_session",
        &[
            "requested_level",
            "actor",
            "sensitivity",
            "target_origins",
            "credential_mode",
            "artifact_mode",
            "required_controls",
            "allow_downgrade",
            "task_label",
        ],
    )?;
    let request = BrowserAdmissionRequest {
        requested_level: mcp_browser_level_arg(
            arguments,
            "requested_level",
            "admit_browser_session",
        )?,
        actor: mcp_browser_actor_arg(arguments, "actor", "admit_browser_session")?,
        sensitivity: mcp_browser_sensitivity_arg(
            arguments,
            "sensitivity",
            "admit_browser_session",
        )?,
        target_origins: mcp_string_array_arg(arguments, "target_origins", "admit_browser_session")?
            .unwrap_or_default(),
        credential_mode: mcp_browser_credential_mode_arg(
            arguments,
            "credential_mode",
            "admit_browser_session",
        )?
        .unwrap_or_default(),
        artifact_mode: mcp_browser_artifact_mode_arg(
            arguments,
            "artifact_mode",
            "admit_browser_session",
        )?
        .unwrap_or_default(),
        required_controls: mcp_browser_controls_arg(
            arguments,
            "required_controls",
            "admit_browser_session",
        )?
        .unwrap_or_default(),
        allow_downgrade: mcp_bool_arg(arguments, "allow_downgrade", "admit_browser_session")?
            .unwrap_or(false),
        task_label: mcp_optional_string_arg(arguments, "task_label", "admit_browser_session")?,
    };
    validate_browser_admission_request(&request).map_err(|message| (-32602, message))?;
    Ok(request)
}

fn mcp_run_wasm_request(arguments: &Value) -> Result<ExecuteRequest, (i64, String)> {
    let arguments = mcp_tool_arguments(
        arguments,
        "run_wasm",
        &[
            "wat",
            "wasm_base64",
            "input",
            "entrypoint",
            "timeout_ms",
            "memory_bytes",
            "fuel",
        ],
    )?;
    let has_wat = arguments.contains_key("wat");
    let has_wasm_base64 = arguments.contains_key("wasm_base64");
    if has_wat == has_wasm_base64 {
        return Err((
            -32602,
            "run_wasm requires exactly one of arguments.wat or arguments.wasm_base64".to_string(),
        ));
    }

    let source = if has_wat {
        Source::WasmWat {
            text: mcp_string_arg(arguments, "wat", "run_wasm")?,
        }
    } else {
        Source::WasmBytesBase64 {
            bytes: mcp_string_arg(arguments, "wasm_base64", "run_wasm")?,
        }
    };

    let mut policy = Policy::default();
    if let Some(timeout_ms) = mcp_u64_arg(arguments, "timeout_ms", "run_wasm")? {
        policy.limits.wall_ms = timeout_ms;
    }
    if let Some(memory_bytes) = mcp_u64_arg(arguments, "memory_bytes", "run_wasm")? {
        policy.limits.memory_bytes = memory_bytes;
    }
    if let Some(fuel) = mcp_u64_arg(arguments, "fuel", "run_wasm")? {
        policy.limits.fuel = Some(fuel);
    }

    Ok(ExecuteRequest {
        lane: Lane::Wasm,
        source,
        entrypoint: mcp_optional_string_arg(arguments, "entrypoint", "run_wasm")?,
        input: arguments.get("input").cloned().unwrap_or(Value::Null),
        stdin: String::new(),
        policy,
        idempotency_key: None,
    })
}

fn mcp_run_code_request(
    arguments: &Value,
    lane: Lane,
    tool: &'static str,
) -> Result<ExecuteRequest, (i64, String)> {
    let arguments = mcp_tool_arguments(
        arguments,
        tool,
        &["code", "input", "timeout_ms", "memory_bytes"],
    )?;
    let mut policy = Policy::default();
    if let Some(timeout_ms) = mcp_u64_arg(arguments, "timeout_ms", tool)? {
        policy.limits.wall_ms = timeout_ms;
    }
    if let Some(memory_bytes) = mcp_u64_arg(arguments, "memory_bytes", tool)? {
        policy.limits.memory_bytes = memory_bytes;
    }

    Ok(ExecuteRequest {
        lane,
        source: Source::Inline {
            code: mcp_string_arg(arguments, "code", tool)?,
        },
        entrypoint: None,
        input: arguments.get("input").cloned().unwrap_or(Value::Null),
        stdin: String::new(),
        policy,
        idempotency_key: None,
    })
}

fn mcp_tool_arguments<'a>(
    arguments: &'a Value,
    tool: &'static str,
    allowed: &[&'static str],
) -> Result<&'a serde_json::Map<String, Value>, (i64, String)> {
    let object = arguments
        .as_object()
        .ok_or((-32602, format!("{tool} arguments must be an object")))?;
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err((-32602, format!("{tool} does not accept argument `{key}`")));
        }
    }
    Ok(object)
}

fn mcp_string_arg(
    arguments: &serde_json::Map<String, Value>,
    key: &'static str,
    tool: &'static str,
) -> Result<String, (i64, String)> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or((-32602, format!("{tool} argument `{key}` must be a string")))
}

fn mcp_optional_string_arg(
    arguments: &serde_json::Map<String, Value>,
    key: &'static str,
    tool: &'static str,
) -> Result<Option<String>, (i64, String)> {
    arguments
        .get(key)
        .map(|_| mcp_string_arg(arguments, key, tool))
        .transpose()
}

fn mcp_string_array_arg(
    arguments: &serde_json::Map<String, Value>,
    key: &'static str,
    tool: &'static str,
) -> Result<Option<Vec<String>>, (i64, String)> {
    arguments
        .get(key)
        .map(|value| {
            let values = value
                .as_array()
                .ok_or((-32602, format!("{tool} argument `{key}` must be an array")))?;
            values
                .iter()
                .map(|value| {
                    value.as_str().map(str::to_string).ok_or((
                        -32602,
                        format!("{tool} argument `{key}` entries must be strings"),
                    ))
                })
                .collect()
        })
        .transpose()
}

fn mcp_bool_arg(
    arguments: &serde_json::Map<String, Value>,
    key: &'static str,
    tool: &'static str,
) -> Result<Option<bool>, (i64, String)> {
    arguments
        .get(key)
        .map(|value| {
            value
                .as_bool()
                .ok_or((-32602, format!("{tool} argument `{key}` must be a boolean")))
        })
        .transpose()
}

fn mcp_browser_controls_arg(
    arguments: &serde_json::Map<String, Value>,
    key: &'static str,
    tool: &'static str,
) -> Result<Option<Vec<BrowserSandboxControl>>, (i64, String)> {
    arguments
        .get(key)
        .map(|value| {
            let values = value
                .as_array()
                .ok_or((-32602, format!("{tool} argument `{key}` must be an array")))?;
            values
                .iter()
                .map(|value| {
                    let control = value.as_str().ok_or((
                        -32602,
                        format!("{tool} argument `{key}` entries must be strings"),
                    ))?;
                    mcp_browser_control(control, key, tool)
                })
                .collect()
        })
        .transpose()
}

fn mcp_u64_arg(
    arguments: &serde_json::Map<String, Value>,
    key: &'static str,
    tool: &'static str,
) -> Result<Option<u64>, (i64, String)> {
    arguments
        .get(key)
        .map(|value| {
            value.as_u64().ok_or((
                -32602,
                format!("{tool} argument `{key}` must be an unsigned integer"),
            ))
        })
        .transpose()
}

fn mcp_browser_level_arg(
    arguments: &serde_json::Map<String, Value>,
    key: &'static str,
    tool: &'static str,
) -> Result<BrowserSandboxLevel, (i64, String)> {
    match mcp_string_arg(arguments, key, tool)?.as_str() {
        "instrumented_external" => Ok(BrowserSandboxLevel::InstrumentedExternal),
        "ephemeral_profile" => Ok(BrowserSandboxLevel::EphemeralProfile),
        "network_suppressed" => Ok(BrowserSandboxLevel::NetworkSuppressed),
        "sealed_state" => Ok(BrowserSandboxLevel::SealedState),
        "os_isolated" => Ok(BrowserSandboxLevel::OsIsolated),
        "remote_isolated" => Ok(BrowserSandboxLevel::RemoteIsolated),
        other => Err((
            -32602,
            format!("{tool} argument `{key}` has unsupported browser sandbox level `{other}`"),
        )),
    }
}

fn mcp_browser_control(
    value: &str,
    key: &'static str,
    tool: &'static str,
) -> Result<BrowserSandboxControl, (i64, String)> {
    match value {
        "fresh_profile" => Ok(BrowserSandboxControl::FreshProfile),
        "no_ambient_credentials" => Ok(BrowserSandboxControl::NoAmbientCredentials),
        "egress_policy" => Ok(BrowserSandboxControl::EgressPolicy),
        "local_network_block" => Ok(BrowserSandboxControl::LocalNetworkBlock),
        "sealed_artifacts" => Ok(BrowserSandboxControl::SealedArtifacts),
        "os_process_isolation" => Ok(BrowserSandboxControl::OsProcessIsolation),
        "remote_worker_isolation" => Ok(BrowserSandboxControl::RemoteWorkerIsolation),
        "teardown_proof" => Ok(BrowserSandboxControl::TeardownProof),
        other => Err((
            -32602,
            format!("{tool} argument `{key}` has unsupported browser control `{other}`"),
        )),
    }
}

fn mcp_browser_actor_arg(
    arguments: &serde_json::Map<String, Value>,
    key: &'static str,
    tool: &'static str,
) -> Result<BrowserSessionActor, (i64, String)> {
    match mcp_string_arg(arguments, key, tool)?.as_str() {
        "agent" => Ok(BrowserSessionActor::Agent),
        "human" => Ok(BrowserSessionActor::Human),
        other => Err((
            -32602,
            format!("{tool} argument `{key}` has unsupported actor `{other}`"),
        )),
    }
}

fn mcp_browser_credential_mode_arg(
    arguments: &serde_json::Map<String, Value>,
    key: &'static str,
    tool: &'static str,
) -> Result<Option<BrowserCredentialMode>, (i64, String)> {
    arguments
        .get(key)
        .map(|_| match mcp_string_arg(arguments, key, tool)?.as_str() {
            "no_credentials" => Ok(BrowserCredentialMode::NoCredentials),
            "user_mediated" => Ok(BrowserCredentialMode::UserMediated),
            "scoped_secrets" => Ok(BrowserCredentialMode::ScopedSecrets),
            other => Err((
                -32602,
                format!("{tool} argument `{key}` has unsupported credential mode `{other}`"),
            )),
        })
        .transpose()
}

fn mcp_browser_artifact_mode_arg(
    arguments: &serde_json::Map<String, Value>,
    key: &'static str,
    tool: &'static str,
) -> Result<Option<BrowserArtifactMode>, (i64, String)> {
    arguments
        .get(key)
        .map(|_| match mcp_string_arg(arguments, key, tool)?.as_str() {
            "discard" => Ok(BrowserArtifactMode::Discard),
            "explicit_downloads" => Ok(BrowserArtifactMode::ExplicitDownloads),
            "sealed_artifacts" => Ok(BrowserArtifactMode::SealedArtifacts),
            other => Err((
                -32602,
                format!("{tool} argument `{key}` has unsupported artifact mode `{other}`"),
            )),
        })
        .transpose()
}

fn mcp_browser_sensitivity_arg(
    arguments: &serde_json::Map<String, Value>,
    key: &'static str,
    tool: &'static str,
) -> Result<BrowserSensitivity, (i64, String)> {
    match mcp_string_arg(arguments, key, tool)?.as_str() {
        "public" => Ok(BrowserSensitivity::Public),
        "sensitive" => Ok(BrowserSensitivity::Sensitive),
        other => Err((
            -32602,
            format!("{tool} argument `{key}` has unsupported sensitivity `{other}`"),
        )),
    }
}

fn tool_result(result: ExecutionResult) -> Result<Value, (i64, String)> {
    // Per the MCP spec, a tool that fails must set isError so the calling agent
    // can branch on it. A trap, fuel/wall timeout, OOM, or denied (e.g. an
    // unavailable lane, or a host-import denial) is not a success.
    let is_error = !matches!(result.status, ExecutionStatus::Ok);
    let text = serde_json::to_string(&result)
        .map_err(|error| (-32000, format!("failed to encode result: {error}")))?;
    Ok(json!({
        "content": [{"type": "text", "text": text}],
        "isError": is_error,
    }))
}

fn api_error_to_rpc(error: ApiError) -> (i64, String) {
    let code = match error.status {
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => -32602,
        StatusCode::UNAUTHORIZED => -32001,
        StatusCode::TOO_MANY_REQUESTS => -32004,
        _ => -32000,
    };
    (code, format!("{}: {}", error.body.code, error.body.message))
}

fn json_response(status: StatusCode, body: Value) -> Response {
    (
        status,
        [(CONTENT_TYPE, "application/json")],
        Body::from(body.to_string()),
    )
        .into_response()
}
