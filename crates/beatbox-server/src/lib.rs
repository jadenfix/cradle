mod jobs;

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::body::{to_bytes, Body};
use axum::extract::{Path as AxumPath, State};
use axum::http::header::{AsHeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE, HOST, ORIGIN};
use axum::http::uri::Authority;
use axum::http::{HeaderMap, Request, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use beatbox_core::{
    CreateJobResponse, Determinism, ErrorBody, ErrorResponse, ExecuteRequest, ExecutionResult,
    ExecutionStatus, JobRecord, JobStatus, Lane, MountMode, NetPolicy, Policy, Source,
};
use beatbox_engine::{
    python_native_available, BeatboxEngine, CancellationToken, EngineError,
    MAX_PYTHON_SOURCE_BYTES, MAX_WASM_MODULE_BYTES,
};
use bytes::Bytes;
pub use jobs::JobStore;
use jobs::{CancelOutcome, JobStoreError};
use serde_json::{json, Value};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use url::{Host, Url};
use utoipa::OpenApi;

pub const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
pub const DEFAULT_SYNC_WALL_MS: u64 = 60_000;
pub const DEFAULT_JOB_WALL_MS: u64 = 5 * 60_000;
pub const DEFAULT_MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;
pub const DEFAULT_MAX_MEMORY_BYTES: u64 = 256 * 1024 * 1024;
pub const DEFAULT_MAX_OUTPUT_BYTES: u64 = 1024 * 1024;
pub const DEFAULT_MAX_DISK_BYTES: u64 = 64 * 1024 * 1024;
pub const DEFAULT_MAX_FUEL: u64 = 100_000_000;
pub const DEFAULT_MAX_CONCURRENT_SYNC: usize = 8;
pub const DEFAULT_MAX_CONCURRENT_JOBS: usize = 4;
pub const DEFAULT_MAX_STORED_JOBS: usize = 10_000;

#[derive(Clone)]
pub struct ServerConfig {
    pub auth: AuthMode,
    pub engine: BeatboxEngine,
    pub jobs: JobStore,
    pub sync_wall_ms: u64,
    pub job_wall_ms: u64,
    pub max_memory_bytes: u64,
    pub max_output_bytes: u64,
    pub max_disk_bytes: u64,
    pub max_fuel: u64,
    pub max_request_bytes: usize,
    pub max_concurrent_sync: usize,
    pub max_concurrent_jobs: usize,
    pub max_stored_jobs: usize,
}

impl ServerConfig {
    pub fn new(engine: BeatboxEngine) -> Self {
        let jobs = JobStore::in_memory()
            .unwrap_or_else(|error| panic!("default in-memory JobStore must construct: {error}"));
        Self {
            auth: AuthMode::None,
            engine,
            jobs,
            sync_wall_ms: DEFAULT_SYNC_WALL_MS,
            job_wall_ms: DEFAULT_JOB_WALL_MS,
            max_memory_bytes: DEFAULT_MAX_MEMORY_BYTES,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            max_disk_bytes: DEFAULT_MAX_DISK_BYTES,
            max_fuel: DEFAULT_MAX_FUEL,
            max_request_bytes: DEFAULT_MAX_REQUEST_BYTES,
            max_concurrent_sync: DEFAULT_MAX_CONCURRENT_SYNC,
            max_concurrent_jobs: DEFAULT_MAX_CONCURRENT_JOBS,
            max_stored_jobs: DEFAULT_MAX_STORED_JOBS,
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

    pub fn with_max_disk_bytes(mut self, max_disk_bytes: u64) -> Self {
        self.max_disk_bytes = max_disk_bytes;
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

    pub fn with_max_stored_jobs(mut self, max_stored_jobs: usize) -> Self {
        self.max_stored_jobs = max_stored_jobs;
        self
    }
}

#[derive(Clone, Default)]
pub enum AuthMode {
    #[default]
    None,
    Required {
        token: String,
    },
}

#[derive(Clone)]
struct AppState {
    config: ServerConfig,
    sync_permits: Arc<Semaphore>,
    job_permits: Arc<Semaphore>,
    running_jobs: Arc<Mutex<HashMap<String, CancellationToken>>>,
}

pub fn router(config: ServerConfig) -> Router {
    let sync_permits = Arc::new(Semaphore::new(config.max_concurrent_sync));
    let job_permits = Arc::new(Semaphore::new(config.max_concurrent_jobs));
    let state = AppState {
        config,
        sync_permits,
        job_permits,
        running_jobs: Arc::new(Mutex::new(HashMap::new())),
    };
    Router::new()
        .route("/v1/health", get(health))
        .route("/openapi.json", get(openapi))
        .route("/v1/capabilities", get(capabilities))
        .route("/v1/execute", post(execute))
        .route("/v1/jobs", post(create_job))
        .route("/v1/jobs/{id}", get(get_job).delete(cancel_job))
        .route("/mcp", get(mcp_get).post(mcp_post))
        .with_state(state)
}

async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok"
    }))
}

async fn capabilities(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Json<Value>, ApiError> {
    require_control_plane_boundary(&headers, &uri)?;
    state.authorize(&headers)?;
    Ok(Json(capabilities_json(&state.config)))
}

async fn execute(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request<Body>,
) -> Result<Json<ExecutionResult>, ApiError> {
    require_control_plane_boundary(&headers, request.uri())?;
    state.authorize(&headers)?;
    let request = parse_json_body(&state, request).await?;
    execute_sync(&state, request).await.map(Json)
}

async fn create_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request<Body>,
) -> Result<(StatusCode, Json<CreateJobResponse>), ApiError> {
    require_control_plane_boundary(&headers, request.uri())?;
    state.authorize(&headers)?;
    let request = parse_json_body(&state, request).await?;
    admit_execution_request(&state.config, &request, ExecutionMode::Job)?;
    if let Some(job_id) = state
        .config
        .jobs
        .find_idempotent(&request)
        .map_err(ApiError::job_store)?
    {
        return Ok((StatusCode::ACCEPTED, Json(CreateJobResponse { job_id })));
    }
    let permit = state.job_permits.clone().try_acquire_owned().map_err(|_| {
        ApiError::too_many(
            "job_concurrency_exceeded",
            format!(
                "maximum concurrent jobs ({}) are already running",
                state.config.max_concurrent_jobs
            ),
        )
    })?;
    let created = state
        .config
        .jobs
        .create_or_get_with_limit(&request, state.config.max_stored_jobs)
        .map_err(ApiError::job_store)?;
    let job_id = created.job_id;
    if created.inserted {
        spawn_job(state, job_id.clone(), request, permit);
    }
    Ok((StatusCode::ACCEPTED, Json(CreateJobResponse { job_id })))
}

async fn get_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<JobRecord>, ApiError> {
    require_control_plane_boundary(&headers, &uri)?;
    state.authorize(&headers)?;
    state
        .config
        .jobs
        .get(&id)
        .map_err(ApiError::job_store)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("unknown job: {id}")))
}

async fn cancel_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    AxumPath(id): AxumPath<String>,
) -> Result<StatusCode, ApiError> {
    require_control_plane_boundary(&headers, &uri)?;
    state.authorize(&headers)?;
    match state.config.jobs.cancel(&id).map_err(ApiError::job_store)? {
        CancelOutcome::Canceled => {
            state.cancel_running_job(&id)?;
            Ok(StatusCode::NO_CONTENT)
        }
        CancelOutcome::AlreadyCanceled => Ok(StatusCode::NO_CONTENT),
        CancelOutcome::NotCancelable(status) => Err(ApiError::conflict(
            "job_not_cancelable",
            format!(
                "job {id} is already {} and cannot be canceled",
                status.as_str()
            ),
        )),
        CancelOutcome::Missing => Err(ApiError::not_found(format!("unknown job: {id}"))),
    }
}

fn spawn_job(
    state: AppState,
    job_id: String,
    request: ExecuteRequest,
    permit: OwnedSemaphorePermit,
) {
    tokio::spawn(async move {
        let _permit = permit;
        match state.config.jobs.mark_running(&job_id) {
            Ok(true) => {}
            Ok(false) => {
                tracing::info!(%job_id, "job was canceled before worker start");
                return;
            }
            Err(error) => {
                tracing::warn!(%job_id, %error, "failed to mark job running");
                return;
            }
        }
        let cancellation = CancellationToken::new();
        if let Err(error) = state.register_running_job(job_id.clone(), cancellation.clone()) {
            cancellation.cancel();
            if let Err(store_error) = state.config.jobs.fail(&job_id, &error) {
                tracing::warn!(%job_id, %store_error, "failed to persist cancellation registry failure");
            }
            return;
        }
        match state.config.jobs.get(&job_id) {
            Ok(Some(record)) if record.status == JobStatus::Canceled => {
                cancellation.cancel();
                if let Err(error) = state.unregister_running_job(&job_id) {
                    tracing::warn!(%job_id, ?error, "failed to unregister canceled job");
                }
                return;
            }
            Ok(Some(_)) => {}
            Ok(None) => {
                cancellation.cancel();
                if let Err(error) = state.unregister_running_job(&job_id) {
                    tracing::warn!(%job_id, ?error, "failed to unregister missing job");
                }
                return;
            }
            Err(error) => {
                cancellation.cancel();
                if let Err(registry_error) = state.unregister_running_job(&job_id) {
                    tracing::warn!(%job_id, ?registry_error, "failed to unregister job after store read failure");
                }
                let body = ErrorBody::new("job_store", error.to_string());
                if let Err(store_error) = state.config.jobs.fail(&job_id, &body) {
                    tracing::warn!(%job_id, %store_error, "failed to persist job store failure");
                }
                return;
            }
        }
        let engine = state.config.engine.clone();
        let result = tokio::task::spawn_blocking(move || {
            engine.execute_with_cancellation(request, cancellation)
        })
        .await;
        if let Err(error) = state.unregister_running_job(&job_id) {
            tracing::warn!(%job_id, ?error, "failed to unregister completed job");
        }
        match result {
            Ok(Ok(result)) => {
                if let Err(error) = state.config.jobs.complete(&job_id, &result) {
                    tracing::warn!(%job_id, %error, "failed to persist job result");
                }
            }
            Ok(Err(error)) => {
                if let Err(store_error) = state.config.jobs.fail(&job_id, &error.error_body()) {
                    tracing::warn!(%job_id, %store_error, "failed to persist job failure");
                }
            }
            Err(error) => {
                let body = ErrorBody::new("job_worker", error.to_string());
                if let Err(store_error) = state.config.jobs.fail(&job_id, &body) {
                    tracing::warn!(%job_id, %store_error, "failed to persist worker failure");
                }
            }
        }
    });
}

impl AppState {
    fn authorize(&self, headers: &HeaderMap) -> Result<(), ApiError> {
        match &self.config.auth {
            AuthMode::None => Ok(()),
            AuthMode::Required { token } => {
                if token.trim().is_empty() {
                    return Err(ApiError::unauthorized("server API key is empty"));
                }
                match authorization(headers, token) {
                    AuthorizationDecision::Authorized => Ok(()),
                    AuthorizationDecision::MissingOrInvalid => {
                        Err(ApiError::unauthorized("missing or invalid API key"))
                    }
                    AuthorizationDecision::Ambiguous => Err(ApiError::unauthorized(
                        "ambiguous API key headers are not accepted",
                    )),
                }
            }
        }
    }

    fn register_running_job(
        &self,
        id: String,
        cancellation: CancellationToken,
    ) -> Result<(), ErrorBody> {
        let mut running = self.running_jobs.lock().map_err(|_| {
            ErrorBody::new(
                "job_cancellation_registry",
                "job cancellation registry mutex was poisoned",
            )
        })?;
        running.insert(id, cancellation);
        Ok(())
    }

    fn unregister_running_job(&self, id: &str) -> Result<(), ErrorBody> {
        let mut running = self.running_jobs.lock().map_err(|_| {
            ErrorBody::new(
                "job_cancellation_registry",
                "job cancellation registry mutex was poisoned",
            )
        })?;
        running.remove(id);
        Ok(())
    }

    fn cancel_running_job(&self, id: &str) -> Result<(), ApiError> {
        let token = {
            let mut running = self.running_jobs.lock().map_err(|_| {
                ApiError::internal(
                    "job_cancellation_registry",
                    "job cancellation registry mutex was poisoned",
                )
            })?;
            running.remove(id)
        };
        if let Some(token) = token {
            token.cancel();
        }
        Ok(())
    }
}

enum AuthorizationDecision {
    Authorized,
    MissingOrInvalid,
    Ambiguous,
}

fn authorization(headers: &HeaderMap, token: &str) -> AuthorizationDecision {
    let api_key = unique_header_value(headers, "x-beatbox-api-key");
    let bearer = unique_header_value(headers, AUTHORIZATION);

    match (api_key, bearer) {
        (UniqueHeader::Duplicate, _) | (_, UniqueHeader::Duplicate) => {
            AuthorizationDecision::Ambiguous
        }
        (UniqueHeader::Present(_), UniqueHeader::Present(_)) => AuthorizationDecision::Ambiguous,
        (UniqueHeader::Present(value), UniqueHeader::Absent) => {
            if value
                .to_str()
                .ok()
                .is_some_and(|actual| constant_time_eq(actual.as_bytes(), token.as_bytes()))
            {
                AuthorizationDecision::Authorized
            } else {
                AuthorizationDecision::MissingOrInvalid
            }
        }
        (UniqueHeader::Absent, UniqueHeader::Present(value)) => {
            if bearer_authorized(value, token) {
                AuthorizationDecision::Authorized
            } else {
                AuthorizationDecision::MissingOrInvalid
            }
        }
        (UniqueHeader::Absent, UniqueHeader::Absent) => AuthorizationDecision::MissingOrInvalid,
    }
}

fn bearer_authorized(value: &HeaderValue, token: &str) -> bool {
    let expected = format!("Bearer {token}");
    value
        .to_str()
        .ok()
        .is_some_and(|actual| constant_time_eq(actual.as_bytes(), expected.as_bytes()))
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

    fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            body: ErrorBody::new("forbidden", message),
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
            JobStoreError::CapacityExceeded(max_jobs) => Self::too_many(
                "job_store_full",
                format!("maximum stored jobs ({max_jobs}) already exist"),
            ),
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
    let content_type = match unique_header_value(headers, CONTENT_TYPE) {
        UniqueHeader::Absent => {
            return Err(ApiError::bad_request(
                "unsupported_media_type",
                "content-type must be application/json",
            ));
        }
        UniqueHeader::Present(content_type) => content_type,
        UniqueHeader::Duplicate => {
            return Err(ApiError::bad_request(
                "unsupported_media_type",
                "content-type must be a single application/json value",
            ));
        }
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

async fn execute_sync(
    state: &AppState,
    request: ExecuteRequest,
) -> Result<ExecutionResult, ApiError> {
    admit_execution_request(&state.config, &request, ExecutionMode::Sync)?;
    let permit = state
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
    let cancellation = CancellationToken::new();
    let engine = state.config.engine.clone();
    let result = spawn_blocking_with_owned_permit(permit, cancellation, move |cancellation| {
        engine.execute_with_cancellation(request, cancellation)
    })
    .await
    .map_err(|error| ApiError::internal("execute_worker", error.to_string()))?;
    result.map_err(ApiError::unprocessable)
}

async fn spawn_blocking_with_owned_permit<T, F>(
    permit: OwnedSemaphorePermit,
    cancellation: CancellationToken,
    work: F,
) -> Result<T, tokio::task::JoinError>
where
    T: Send + 'static,
    F: FnOnce(CancellationToken) -> T + Send + 'static,
{
    let mut cancel_on_drop = CancelOnDrop::new(cancellation.clone());
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work(cancellation)
    })
    .await;
    cancel_on_drop.disarm();
    result
}

struct CancelOnDrop {
    cancellation: Option<CancellationToken>,
}

impl CancelOnDrop {
    fn new(cancellation: CancellationToken) -> Self {
        Self {
            cancellation: Some(cancellation),
        }
    }

    fn disarm(&mut self) {
        self.cancellation = None;
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(cancellation) = self.cancellation.take() {
            cancellation.cancel();
        }
    }
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
    admit_remote_lane(request)?;
    admit_remote_policy(request)?;
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
    if request.policy.limits.disk_bytes > config.max_disk_bytes {
        return Err(limit_exceeded(
            "disk_bytes",
            request.policy.limits.disk_bytes,
            config.max_disk_bytes,
        ));
    }
    if let Some(fuel) = request.policy.limits.fuel
        && fuel > config.max_fuel
    {
        return Err(limit_exceeded("fuel", fuel, config.max_fuel));
    }
    Ok(())
}

fn admit_remote_lane(request: &ExecuteRequest) -> Result<(), ApiError> {
    match &request.lane {
        Lane::Wasm => Ok(()),
        Lane::PythonNative if python_native_available() => Ok(()),
        Lane::PythonWasi | Lane::PythonNative | Lane::JsWasm | Lane::JsNative | Lane::Exec => {
            Err(lane_unavailable(&request.lane))
        }
    }
}

fn admit_remote_policy(request: &ExecuteRequest) -> Result<(), ApiError> {
    match request.lane {
        Lane::Wasm => admit_remote_wasm_policy(&request.policy),
        Lane::PythonNative => admit_remote_python_native_policy(&request.policy),
        Lane::PythonWasi | Lane::JsWasm | Lane::JsNative | Lane::Exec => Ok(()),
    }
}

fn admit_remote_wasm_policy(policy: &Policy) -> Result<(), ApiError> {
    if policy.fs.workspace.is_some() {
        return Err(policy_unenforceable(
            "fs.workspace",
            "the initial wasm lane is W0 hermetic and exposes no filesystem",
        ));
    }
    if let Some(mount) = policy.fs.mounts.first() {
        let mode = mount_mode_label(&mount.mode);
        return Err(policy_unenforceable(
            "fs.mounts",
            format!(
                "the initial wasm lane exposes no mounts; requested {mode} mount at {}",
                mount.guest.display()
            ),
        ));
    }
    if !matches!(policy.net, NetPolicy::Deny) {
        return Err(policy_unenforceable(
            "net",
            "raw network and proxy egress are not exposed in W0",
        ));
    }
    if !policy.env.is_empty() {
        return Err(policy_unenforceable(
            "env",
            "the initial wasm lane exposes no environment",
        ));
    }
    if !policy.secrets.is_empty() {
        return Err(policy_unenforceable(
            "secrets",
            "the initial wasm lane exposes no secrets",
        ));
    }
    if policy.double_jail {
        return Err(policy_unenforceable(
            "double_jail",
            "the initial wasm lane cannot add a second OS or VM isolation boundary",
        ));
    }
    Ok(())
}

fn admit_remote_python_native_policy(policy: &Policy) -> Result<(), ApiError> {
    if policy.fs.workspace.is_some() {
        return Err(policy_unenforceable(
            "fs.workspace",
            "python_native currently creates a fresh private workspace per run",
        ));
    }
    if let Some(mount) = policy.fs.mounts.first() {
        let mode = mount_mode_label(&mount.mode);
        return Err(policy_unenforceable(
            "fs.mounts",
            format!(
                "python_native does not expose host mounts yet; requested {mode} mount at {}",
                mount.guest.display()
            ),
        ));
    }
    if !matches!(policy.net, NetPolicy::Deny) {
        return Err(policy_unenforceable(
            "net",
            "python_native exposes no network or proxy egress",
        ));
    }
    if !policy.env.is_empty() {
        return Err(policy_unenforceable(
            "env",
            "python_native starts with an empty environment",
        ));
    }
    if !policy.secrets.is_empty() {
        return Err(policy_unenforceable(
            "secrets",
            "python_native does not expose secrets",
        ));
    }
    if !matches!(policy.determinism, Determinism::Off) {
        return Err(policy_unenforceable(
            "determinism",
            "python_native is not deterministic",
        ));
    }
    if policy.double_jail {
        return Err(policy_unenforceable(
            "double_jail",
            "double_jail applies only to wasm lanes",
        ));
    }
    admit_remote_python_native_unenforced_limits(policy)?;
    Ok(())
}

fn admit_remote_python_native_unenforced_limits(policy: &Policy) -> Result<(), ApiError> {
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
            return Err(policy_unenforceable(field, reason));
        }
    }
    if policy.limits.fuel != default.fuel {
        return Err(policy_unenforceable(
            "limits.fuel",
            "fuel applies only to Wasmtime-backed lanes",
        ));
    }
    Ok(())
}

fn mount_mode_label(mode: &MountMode) -> &'static str {
    match mode {
        MountMode::Ro => "ro",
        MountMode::Rw => "rw",
    }
}

fn policy_unenforceable(field: &'static str, reason: impl Into<String>) -> ApiError {
    ApiError::unprocessable_body(ErrorBody::new(
        "policy_unenforceable",
        format!("policy field {field} cannot be enforced: {}", reason.into()),
    ))
}

fn lane_unavailable(lane: &Lane) -> ApiError {
    ApiError::unprocessable_body(ErrorBody::new(
        "lane_unavailable",
        format!(
            "lane {} is not available on this daemon; check /v1/capabilities before submitting work",
            lane_label(lane)
        ),
    ))
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

fn capabilities_json(config: &ServerConfig) -> Value {
    let python_native_is_available = python_native_available();
    let python_native_mechanisms = if python_native_is_available {
        json!([
            "sandbox-exec",
            "seatbelt-profile",
            "env-clear",
            "trusted-python-binary",
            "runtime-read-allowlist",
            "network-deny",
            "process-fork-deny",
            "mach-lookup-deny",
            "sysctl-read-deny",
            "wall-time-watchdog",
            "source-byte-limit",
            "stdin-delivery-watchdog",
            "workspace-disk-quota",
            "output-cap"
        ])
    } else {
        json!([])
    };
    let python_native_downgrades = if python_native_is_available {
        json!([
            "macos_native_lane_dev_grade",
            "memory_limit_not_enforced",
            "cpu_limit_not_enforced"
        ])
    } else {
        json!([])
    };
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "lanes": [
            {
                "lane": "wasm",
                "available": true,
                "substrate": "wasmtime",
                "grade": {"linux": "prod", "macos": "prod"},
                "mechanisms": ["empty-linker", "host-import-deny", "precompile-import-scan", "fuel", "epoch-interruption", "module-byte-limit", "store-limits"]
            },
            {"lane": "python_wasi", "available": false, "substrate": "wasmtime", "grade": {"linux": "planned", "macos": "planned"}},
            {"lane": "js_wasm", "available": false, "substrate": "wasmtime", "grade": {"linux": "planned", "macos": "planned"}},
            {
                "lane": "python_native",
                "available": python_native_is_available,
                "substrate": "os_jail",
                "grade": {"linux": "planned", "macos": "dev"},
                "mechanisms": python_native_mechanisms,
                "downgrades": python_native_downgrades
            },
            {"lane": "js_native", "available": false, "substrate": "os_jail", "grade": {"linux": "planned", "macos": "planned_dev"}},
            {"lane": "exec", "available": false, "substrate": "os_jail", "grade": {"linux": "planned", "macos": "planned_dev"}}
        ],
        "limits": {
            "sync_wall_ms": config.sync_wall_ms,
            "job_wall_ms": config.job_wall_ms,
            "default_wall_ms": Policy::default().limits.wall_ms,
            "default_memory_bytes": Policy::default().limits.memory_bytes,
            "default_disk_bytes": Policy::default().limits.disk_bytes,
            "default_output_bytes": Policy::default().limits.output_bytes,
            "max_request_bytes": config.max_request_bytes,
            "max_wasm_module_bytes": MAX_WASM_MODULE_BYTES,
            "max_python_source_bytes": MAX_PYTHON_SOURCE_BYTES,
            "max_memory_bytes": config.max_memory_bytes,
            "max_output_bytes": config.max_output_bytes,
            "max_disk_bytes": config.max_disk_bytes,
            "max_fuel": config.max_fuel,
            "max_concurrent_sync": config.max_concurrent_sync,
            "max_concurrent_jobs": config.max_concurrent_jobs,
            "max_stored_jobs": config.max_stored_jobs
        },
        "engines": {"wasmtime": "45"}
    })
}

async fn openapi(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Json<Value>, ApiError> {
    require_control_plane_boundary(&headers, &uri)?;
    state.authorize(&headers)?;
    let mut value = serde_json::to_value(ApiDoc::openapi())
        .map_err(|error| ApiError::internal("openapi", error.to_string()))?;
    close_source_schema_variants(&mut value);
    Ok(Json(value))
}

fn close_source_schema_variants(openapi: &mut Value) {
    let Some(variants) = openapi
        .pointer_mut("/components/schemas/Source/oneOf")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for variant in variants {
        if let Some(object) = variant.as_object_mut() {
            object.insert("additionalProperties".to_string(), Value::Bool(false));
        }
    }
}

#[derive(OpenApi)]
#[openapi(
    info(title = "beatbox API", version = "0.1.0"),
    paths(
        openapi_paths::health,
        openapi_paths::capabilities,
        openapi_paths::execute,
        openapi_paths::create_job,
        openapi_paths::get_job,
        openapi_paths::cancel_job,
        openapi_paths::mcp_get,
        openapi_paths::mcp_post
    ),
    components(schemas(
        CreateJobResponse,
        ErrorBody,
        ErrorResponse,
        ExecuteRequest,
        ExecutionResult,
        JobRecord,
        Policy,
        Source
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
        CreateJobResponse, ErrorResponse, ExecuteRequest, ExecutionResult, JobRecord,
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
            (status = 200, description = "Lane availability and host limits"),
            (status = 401, description = "Missing or invalid bearer token", body = ErrorResponse),
            (status = 403, description = "Origin or Host not allowed", body = ErrorResponse)
        )
    )]
    pub fn capabilities() {}

    #[utoipa::path(
        post,
        path = "/v1/execute",
        tag = "v1",
        request_body = ExecuteRequest,
        responses(
            (status = 200, description = "ExecutionResult", body = ExecutionResult),
            (status = 400, description = "Unsupported content type, invalid JSON, or request body limit rejection", body = ErrorResponse),
            (status = 401, description = "Missing or invalid bearer token", body = ErrorResponse),
            (status = 403, description = "Origin or Host not allowed", body = ErrorResponse),
            (status = 422, description = "Protocol, source, lane availability, policy, or sync-limit rejection", body = ErrorResponse),
            (status = 429, description = "Synchronous execution concurrency limit exceeded", body = ErrorResponse)
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
            (status = 400, description = "Unsupported content type, invalid JSON, or request body limit rejection", body = ErrorResponse),
            (status = 401, description = "Missing or invalid bearer token", body = ErrorResponse),
            (status = 403, description = "Origin or Host not allowed", body = ErrorResponse),
            (status = 409, description = "Idempotency key was reused with a conflicting payload", body = ErrorResponse),
            (status = 422, description = "Protocol, source, lane availability, policy, or job-limit rejection", body = ErrorResponse),
            (status = 429, description = "Asynchronous job concurrency or stored-job quota exceeded", body = ErrorResponse)
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
            (status = 403, description = "Origin or Host not allowed", body = ErrorResponse),
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
            (status = 204, description = "Canceled queued job or signaled running job interruption"),
            (status = 401, description = "Missing or invalid bearer token", body = ErrorResponse),
            (status = 403, description = "Origin or Host not allowed", body = ErrorResponse),
            (status = 409, description = "Job is already terminal and cannot be canceled", body = ErrorResponse),
            (status = 404, description = "Unknown job", body = ErrorResponse)
        )
    )]
    pub fn cancel_job() {}

    #[utoipa::path(
        get,
        path = "/mcp",
        tag = "mcp",
        responses(
            (status = 405, description = "Server-initiated MCP stream is not available"),
            (status = 401, description = "Missing or invalid bearer token", body = ErrorResponse),
            (status = 403, description = "Origin or Host not allowed", body = ErrorResponse)
        )
    )]
    pub fn mcp_get() {}

    #[utoipa::path(
        post,
        path = "/mcp",
        tag = "mcp",
        responses(
            (status = 200, description = "JSON-RPC response"),
            (status = 202, description = "JSON-RPC notification accepted"),
            (status = 400, description = "Unsupported content type, invalid JSON, or request body limit rejection"),
            (status = 401, description = "Missing or invalid bearer token"),
            (status = 403, description = "Origin or Host not allowed")
        )
    )]
    pub fn mcp_post() {}
}

pub fn origin_allowed(headers: &HeaderMap) -> bool {
    match unique_header_value(headers, ORIGIN) {
        UniqueHeader::Absent => true,
        UniqueHeader::Present(origin) => origin.to_str().ok().is_some_and(local_origin_allowed),
        UniqueHeader::Duplicate => false,
    }
}

pub fn host_allowed(headers: &HeaderMap) -> bool {
    match unique_header_value(headers, HOST) {
        UniqueHeader::Absent => true,
        UniqueHeader::Present(host) => host.to_str().ok().is_some_and(local_host_allowed),
        UniqueHeader::Duplicate => false,
    }
}

pub fn request_target_allowed(uri: &Uri) -> bool {
    match uri.authority() {
        None => true,
        Some(authority) => local_host_allowed(authority.as_str()),
    }
}

fn require_control_plane_boundary(headers: &HeaderMap, uri: &Uri) -> Result<(), ApiError> {
    if !origin_allowed(headers) {
        Err(ApiError::forbidden("origin not allowed"))
    } else if !host_allowed(headers) {
        Err(ApiError::forbidden("host not allowed"))
    } else if !request_target_allowed(uri) {
        Err(ApiError::forbidden("request target not allowed"))
    } else {
        Ok(())
    }
}

fn local_origin_allowed(origin: &str) -> bool {
    if origin != origin.trim() {
        return false;
    }
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
    let url = Url::parse(origin).ok()?;
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

fn local_host_allowed(host: &str) -> bool {
    if host != host.trim() {
        return false;
    }
    if host.is_empty()
        || host.contains('@')
        || host.contains('/')
        || host.contains('\\')
        || host.contains('?')
        || host.contains('#')
    {
        return false;
    }
    let Some(parsed_host) = parse_host_header_name(host) else {
        return false;
    };
    if parsed_host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match parsed_host.parse::<IpAddr>() {
        Ok(IpAddr::V4(addr)) => addr == Ipv4Addr::LOCALHOST,
        Ok(IpAddr::V6(addr)) => addr == Ipv6Addr::LOCALHOST,
        Err(_) => false,
    }
}

fn parse_host_header_name(host: &str) -> Option<String> {
    if let Some(rest) = host.strip_prefix('[') {
        let (addr, suffix) = rest.split_once(']')?;
        if suffix.is_empty() || valid_host_port_suffix(suffix) {
            return Some(addr.to_string());
        }
        return None;
    }
    if !valid_non_bracket_host_port(host) {
        return None;
    }
    let authority: Authority = host.parse().ok()?;
    if authority.port().is_some() && authority.port_u16().is_none() {
        return None;
    }
    Some(authority.host().to_string())
}

fn valid_non_bracket_host_port(host: &str) -> bool {
    match host.as_bytes().iter().filter(|byte| **byte == b':').count() {
        0 => true,
        1 => host
            .rsplit_once(':')
            .is_some_and(|(_, port)| port.parse::<u16>().is_ok()),
        _ => false,
    }
}

fn valid_host_port_suffix(suffix: &str) -> bool {
    suffix
        .strip_prefix(':')
        .is_some_and(|port| port.parse::<u16>().is_ok())
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

enum UniqueHeader<'a> {
    Absent,
    Present(&'a HeaderValue),
    Duplicate,
}

fn unique_header_value<'a, K>(headers: &'a HeaderMap, name: K) -> UniqueHeader<'a>
where
    K: AsHeaderName,
{
    let mut values = headers.get_all(name).iter();
    let Some(first) = values.next() else {
        return UniqueHeader::Absent;
    };
    if values.next().is_some() {
        UniqueHeader::Duplicate
    } else {
        UniqueHeader::Present(first)
    }
}

async fn mcp_get(State(state): State<AppState>, headers: HeaderMap, uri: Uri) -> Response {
    if let Err(error) = require_control_plane_boundary(&headers, &uri) {
        return error.into_response();
    }
    if let Err(error) = state.authorize(&headers) {
        return error.into_response();
    }
    (
        StatusCode::METHOD_NOT_ALLOWED,
        [("allow", "POST")],
        "this MCP endpoint does not offer a server-initiated stream",
    )
        .into_response()
}

async fn mcp_post(State(state): State<AppState>, request: Request<Body>) -> Response {
    let (parts, body) = request.into_parts();
    let uri = parts.uri;
    let headers = parts.headers;
    if let Err(error) = require_control_plane_boundary(&headers, &uri) {
        return json_response(
            StatusCode::FORBIDDEN,
            json!({"jsonrpc": "2.0", "error": {"code": -32600, "message": error.body.message}}),
        );
    }
    if let Err(error) = state.authorize(&headers) {
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
                "$defs": mcp_json_schema_defs(),
                "properties": {
                    "wat": {
                        "type": "string",
                        "description": "WebAssembly text module source. Mutually exclusive with wasm_base64."
                    },
                    "wasm_base64": {
                        "type": "string",
                        "description": "Base64-encoded WebAssembly module bytes. Mutually exclusive with wat."
                    },
                    "input": mcp_json_input_schema(),
                    "entrypoint": {
                        "type": "string",
                        "description": "Optional exported function name to invoke."
                    },
                    "timeout_ms": mcp_unsigned_integer_schema("Optional wall-clock timeout in milliseconds."),
                    "memory_bytes": mcp_unsigned_integer_schema("Optional memory limit in bytes."),
                    "fuel": mcp_unsigned_integer_schema("Optional Wasmtime fuel budget.")
                }
            }
        },
        {
            "name": "run_python",
            "description": "Run Python source in the explicitly enabled native macOS Python lane.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "required": ["code"],
                "$defs": mcp_json_schema_defs(),
                "properties": {
                    "code": {
                        "type": "string",
                        "description": "Python source code to run on stdin."
                    },
                    "timeout_ms": mcp_unsigned_integer_schema("Optional wall-clock timeout in milliseconds."),
                    "disk_bytes": mcp_unsigned_integer_schema("Optional private workspace disk limit in bytes.")
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
                "$defs": mcp_json_schema_defs(),
                "properties": {
                    "code": {
                        "type": "string",
                        "description": "JavaScript source code to run on stdin."
                    },
                    "input": mcp_json_input_schema(),
                    "timeout_ms": mcp_unsigned_integer_schema("Optional wall-clock timeout in milliseconds."),
                    "memory_bytes": mcp_unsigned_integer_schema("Optional memory limit in bytes.")
                }
            }
        },
        {
            "name": "get_capabilities",
            "description": "Return beatbox lane availability.",
            "inputSchema": {"type": "object", "additionalProperties": false}
        }
    ])
}

fn mcp_json_schema_defs() -> Value {
    json!({
        "json_value": {
            "description": "Any JSON value accepted by the MCP parser.",
            "oneOf": [
                {"type": "null"},
                {"type": "boolean"},
                {"type": "number"},
                {"type": "string"},
                {"type": "array", "items": {"$ref": "#/$defs/json_value"}},
                {"type": "object", "additionalProperties": {"$ref": "#/$defs/json_value"}}
            ]
        }
    })
}

fn mcp_json_input_schema() -> Value {
    json!({
        "description": "Optional arbitrary JSON value passed as execution input. Defaults to null.",
        "default": null,
        "allOf": [{"$ref": "#/$defs/json_value"}]
    })
}

fn mcp_unsigned_integer_schema(description: &'static str) -> Value {
    json!({
        "type": "integer",
        "minimum": 0,
        "description": description
    })
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
        "run_wasm" => {
            let request = mcp_run_wasm_request(&arguments)?;
            let result = execute_sync(state, request)
                .await
                .map_err(api_error_to_rpc)?;
            tool_result(result)
        }
        "run_python" => {
            let request = mcp_run_code_request(&arguments, Lane::PythonNative, "run_python")?;
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
    let accepts_input = lane != Lane::PythonNative;
    let allowed = if accepts_input {
        &["code", "input", "timeout_ms", "memory_bytes"][..]
    } else {
        &["code", "timeout_ms", "disk_bytes"][..]
    };
    let arguments = mcp_tool_arguments(arguments, tool, allowed)?;
    let mut policy = Policy::default();
    if let Some(timeout_ms) = mcp_u64_arg(arguments, "timeout_ms", tool)? {
        policy.limits.wall_ms = timeout_ms;
    }
    if accepts_input && let Some(memory_bytes) = mcp_u64_arg(arguments, "memory_bytes", tool)? {
        policy.limits.memory_bytes = memory_bytes;
    }
    if lane == Lane::PythonNative
        && let Some(disk_bytes) = mcp_u64_arg(arguments, "disk_bytes", tool)?
    {
        policy.limits.disk_bytes = disk_bytes;
    }

    Ok(ExecuteRequest {
        lane,
        source: Source::Inline {
            code: mcp_string_arg(arguments, "code", tool)?,
        },
        entrypoint: None,
        input: if accepts_input {
            arguments.get("input").cloned().unwrap_or(Value::Null)
        } else {
            Value::Null
        },
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

fn tool_result(result: ExecutionResult) -> Result<Value, (i64, String)> {
    let text = execution_result_summary(&result);
    let is_error = result.status != ExecutionStatus::Ok;
    let structured = serde_json::to_value(&result)
        .map_err(|error| (-32000, format!("failed to encode result: {error}")))?;
    Ok(json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": structured,
        "isError": is_error,
    }))
}

fn execution_result_summary(result: &ExecutionResult) -> String {
    let status = execution_status_label(&result.status);
    let lane = lane_label(&result.lane);
    match &result.error {
        Some(error) => format!("beatbox execution {status} on {lane}: {}", error.code),
        None => format!("beatbox execution {status} on {lane}"),
    }
}

fn execution_status_label(status: &ExecutionStatus) -> &'static str {
    match status {
        ExecutionStatus::Ok => "ok",
        ExecutionStatus::Error => "error",
        ExecutionStatus::Timeout => "timeout",
        ExecutionStatus::Oom => "oom",
        ExecutionStatus::Killed => "killed",
        ExecutionStatus::Denied => "denied",
    }
}

fn lane_label(lane: &Lane) -> &'static str {
    match lane {
        Lane::Wasm => "wasm",
        Lane::PythonWasi => "python_wasi",
        Lane::PythonNative => "python_native",
        Lane::JsWasm => "js_wasm",
        Lane::JsNative => "js_native",
        Lane::Exec => "exec",
    }
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

#[cfg(test)]
mod tests {
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    use tokio::sync::{oneshot, Semaphore};

    use super::*;

    #[tokio::test]
    async fn dropped_sync_waiter_cancels_worker_without_releasing_permit_early(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let semaphore = Arc::new(Semaphore::new(1));
        let permit = semaphore.clone().try_acquire_owned()?;
        let cancellation = CancellationToken::new();
        let (started_tx, started_rx) = oneshot::channel();
        let (canceled_tx, canceled_rx) = oneshot::channel();
        let (release_tx, release_rx) = mpsc::channel();

        let task = tokio::spawn(spawn_blocking_with_owned_permit(
            permit,
            cancellation,
            move |cancellation| {
                let _ = started_tx.send(());
                while !cancellation.is_canceled() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                let _ = canceled_tx.send(());
                let _ = release_rx.recv_timeout(Duration::from_secs(2));
            },
        ));

        tokio::time::timeout(Duration::from_secs(1), started_rx).await??;
        task.abort();
        tokio::task::yield_now().await;
        tokio::time::timeout(Duration::from_secs(1), canceled_rx).await??;

        assert!(
            semaphore.clone().try_acquire_owned().is_err(),
            "sync permit was released while the blocking worker was still alive"
        );

        release_tx.send(())?;
        let permit =
            tokio::time::timeout(Duration::from_secs(1), semaphore.acquire_owned()).await??;
        drop(permit);
        Ok(())
    }
}
