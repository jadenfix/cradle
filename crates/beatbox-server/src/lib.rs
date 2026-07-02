mod jobs;

use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use axum::body::{Body, to_bytes};
use axum::extract::{Path as AxumPath, State};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use beatbox_core::{
    CreateJobResponse, ErrorBody, ErrorResponse, ExecuteRequest, ExecutionResult, JobRecord, Lane,
    Policy, Source,
};
use beatbox_engine::{BeatboxEngine, EngineError};
use bytes::Bytes;
pub use jobs::JobStore;
use jobs::JobStoreError;
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
            max_fuel: DEFAULT_MAX_FUEL,
            max_request_bytes: DEFAULT_MAX_REQUEST_BYTES,
            max_concurrent_sync: DEFAULT_MAX_CONCURRENT_SYNC,
            max_concurrent_jobs: DEFAULT_MAX_CONCURRENT_JOBS,
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
}

pub fn router(config: ServerConfig) -> Router {
    let sync_permits = Arc::new(Semaphore::new(config.max_concurrent_sync));
    let job_permits = Arc::new(Semaphore::new(config.max_concurrent_jobs));
    let state = AppState {
        started: Instant::now(),
        config,
        sync_permits,
        job_permits,
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
) -> Result<Json<Value>, ApiError> {
    state.authorize(&headers)?;
    Ok(Json(capabilities_json(&state.config)))
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

    let store = state.config.jobs.clone();
    let lookup = Arc::clone(&request);
    if let Some(job_id) = blocking_store(move || store.find_idempotent(&lookup))
        .await
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
    let store = state.config.jobs.clone();
    let create = Arc::clone(&request);
    let created = blocking_store(move || store.create_or_get(&create))
        .await
        .map_err(ApiError::job_store)?;
    let job_id = created.job_id;
    if created.inserted {
        let request = Arc::try_unwrap(request).unwrap_or_else(|arc| (*arc).clone());
        spawn_job(state, job_id.clone(), request, permit);
    }
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
    let exists = blocking_store(move || store.cancel(&cancel_id))
        .await
        .map_err(ApiError::job_store)?;
    if exists {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found(format!("unknown job: {id}")))
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
        let result = tokio::task::spawn_blocking(move || engine.execute(request)).await;
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
                fail_job(&state, &job_id, ErrorBody::new("job_worker", error.to_string())).await;
            }
        }
    });
}

async fn fail_job(state: &AppState, job_id: &str, body: ErrorBody) {
    let store = state.config.jobs.clone();
    let fail_id = job_id.to_string();
    if let Err(store_error) = blocking_store(move || store.fail(&fail_id, &body)).await {
        tracing::warn!(%job_id, %store_error, "failed to persist job failure");
    }
}

impl AppState {
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

fn capabilities_json(config: &ServerConfig) -> Value {
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "lanes": [
            {
                "lane": "wasm",
                "available": true,
                "substrate": "wasmtime",
                "grade": {"linux": "prod", "macos": "prod"},
                "mechanisms": ["empty-linker", "fuel", "epoch-interruption", "store-limits"]
            },
            {"lane": "python_wasi", "available": false, "substrate": "wasmtime", "grade": {"linux": "planned", "macos": "planned"}},
            {"lane": "js_wasm", "available": false, "substrate": "wasmtime", "grade": {"linux": "planned", "macos": "planned"}},
            {"lane": "python_native", "available": false, "substrate": "os_jail", "grade": {"linux": "planned", "macos": "planned_dev"}},
            {"lane": "js_native", "available": false, "substrate": "os_jail", "grade": {"linux": "planned", "macos": "planned_dev"}},
            {"lane": "exec", "available": false, "substrate": "os_jail", "grade": {"linux": "planned", "macos": "planned_dev"}}
        ],
        "limits": {
            "sync_wall_ms": config.sync_wall_ms,
            "job_wall_ms": config.job_wall_ms,
            "default_wall_ms": Policy::default().limits.wall_ms,
            "default_memory_bytes": Policy::default().limits.memory_bytes,
            "default_output_bytes": Policy::default().limits.output_bytes,
            "max_request_bytes": config.max_request_bytes,
            "max_memory_bytes": config.max_memory_bytes,
            "max_output_bytes": config.max_output_bytes,
            "max_fuel": config.max_fuel,
            "max_concurrent_sync": config.max_concurrent_sync,
            "max_concurrent_jobs": config.max_concurrent_jobs
        },
        "engines": {"wasmtime": "45"}
    })
}

async fn openapi() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
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
        openapi_paths::mcp_post
    ),
    tags(
        (name = "v1", description = "beatbox REST API"),
        (name = "mcp", description = "stateless MCP JSON-RPC endpoint")
    )
)]
struct ApiDoc;

#[allow(dead_code)]
mod openapi_paths {
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
            (status = 401, description = "Missing or invalid bearer token")
        )
    )]
    pub fn capabilities() {}

    #[utoipa::path(
        post,
        path = "/v1/execute",
        tag = "v1",
        responses(
            (status = 200, description = "ExecutionResult"),
            (status = 401, description = "Missing or invalid bearer token"),
            (status = 422, description = "Protocol, source, policy, or sync-limit rejection")
        )
    )]
    pub fn execute() {}

    #[utoipa::path(
        post,
        path = "/v1/jobs",
        tag = "v1",
        responses(
            (status = 202, description = "Created asynchronous job"),
            (status = 401, description = "Missing or invalid bearer token")
        )
    )]
    pub fn create_job() {}

    #[utoipa::path(
        get,
        path = "/v1/jobs/{id}",
        tag = "v1",
        params(("id" = String, Path, description = "Job id")),
        responses(
            (status = 200, description = "JobRecord"),
            (status = 401, description = "Missing or invalid bearer token"),
            (status = 404, description = "Unknown job")
        )
    )]
    pub fn get_job() {}

    #[utoipa::path(
        delete,
        path = "/v1/jobs/{id}",
        tag = "v1",
        params(("id" = String, Path, description = "Job id")),
        responses(
            (status = 204, description = "Canceled job"),
            (status = 401, description = "Missing or invalid bearer token"),
            (status = 404, description = "Unknown job")
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
            Ok(json!({
                "content": [{"type": "text", "text": capabilities_json(&state.config).to_string()}],
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
    let text = serde_json::to_string(&result)
        .map_err(|error| (-32000, format!("failed to encode result: {error}")))?;
    Ok(json!({
        "content": [{"type": "text", "text": text}],
        "isError": false,
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
