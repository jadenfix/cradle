mod jobs;

use std::collections::{BTreeMap, HashMap};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{Body, to_bytes};
use axum::extract::{Path as AxumPath, State};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use beatbox_core::{
    AetherPaymentContextCapabilities, BROWSER_ADAPTER_LAUNCH_LEASE_SECONDS,
    BrowserAdapterCapabilityIssueRequest, BrowserAdapterCapabilityIssueResponse,
    BrowserAdapterCompletionReport, BrowserAdapterCompletionValidationDecision,
    BrowserAdapterCompletionValidationResponse, BrowserAdapterConformanceCase,
    BrowserAdapterConformanceExpectation, BrowserAdapterConformanceProfile, BrowserAdapterContract,
    BrowserAdapterContractResponse, BrowserAdapterHandoff, BrowserAdapterLaunchClaimDecision,
    BrowserAdapterLaunchClaimRequest, BrowserAdapterLaunchClaimResponse,
    BrowserAdapterLaunchPlanDecision, BrowserAdapterLaunchPlanRequest,
    BrowserAdapterLaunchPlanResponse, BrowserAdapterLaunchRequest, BrowserAdapterManifestRequest,
    BrowserAdapterManifestResponse, BrowserAdapterRegistrationDecision,
    BrowserAdapterRegistrationRequest, BrowserAdapterRegistrationResponse,
    BrowserAdapterValidationDecision, BrowserAdmissionDecision, BrowserAdmissionGuardPlan,
    BrowserAdmissionRequest, BrowserAdmissionResponse, BrowserArtifactMode,
    BrowserCredentialGuardPlan, BrowserCredentialMode, BrowserIntegrationContract,
    BrowserNetworkGuardPlan, BrowserProfilesResponse, BrowserSandboxAvailability,
    BrowserSandboxControl, BrowserSandboxLevel, BrowserSandboxProfile,
    BrowserSensitiveActivityMode, BrowserSensitivity, BrowserSessionActor, BrowserStorageGuardPlan,
    BrowserSuppressionGuardPlan, CapabilitiesResponse, CapabilityLane, CapabilityLimits,
    CreateJobResponse, ErrorBody, ErrorResponse, ExecuteRequest, ExecutionResult, ExecutionStatus,
    JobRecord, Lane, Policy, Source, browser_adapter_launch_template_expires_at,
    browser_adapter_launch_template_issued_at,
};
use beatbox_engine::{BeatboxEngine, CancelFlag, EngineError};
use bytes::Bytes;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
pub use jobs::JobStore;
use jobs::{CancelOutcome, JobStoreError};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
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
pub const BROWSER_ADAPTER_CAPABILITY_TTL_SECONDS: u64 = 5 * 60;
pub const MAX_BROWSER_ADAPTER_CAPABILITIES: usize = 128;
pub const MAX_BROWSER_ADAPTER_LAUNCH_REQUESTS: usize = 128;
pub const AETHER_PAYMENT_HEADER: &str = "x-payment";
pub const AETHER_PAYMENT_HASH_HEADER: &str = "x-aether-payment-hash";
pub const MAX_AETHER_PAYMENT_HEADER_BYTES: usize = 8192;
const MCP_ACCESS_CONTROL_ALLOW_METHODS: &str = "POST, GET, OPTIONS";
const MCP_ACCESS_CONTROL_ALLOW_HEADERS: &str = concat!(
    "authorization, content-type, accept, mcp-protocol-version, mcp-session-id, ",
    "x-payment, x-aether-payment-hash"
);
const MCP_ACCESS_CONTROL_EXPOSE_HEADERS: &str = "www-authenticate, x-aether-payment-hash";

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
    browser_adapter_capabilities: Arc<Mutex<HashMap<[u8; 32], BrowserAdapterCapabilityRecord>>>,
    browser_adapter_launch_requests: Arc<Mutex<HashMap<String, BrowserAdapterLaunchRequestRecord>>>,
}

#[derive(Clone, Debug)]
struct BrowserAdapterCapabilityRecord {
    actor: BrowserSessionActor,
    sensitivity: BrowserSensitivity,
    sensitive_activity_mode: Option<BrowserSensitiveActivityMode>,
    adapter_id: Option<String>,
    expires_at: Instant,
    used: bool,
}

#[derive(Clone, Debug)]
struct BrowserAdapterLaunchRequestRecord {
    launch_request: BrowserAdapterLaunchRequest,
    expires_at: Instant,
    claimed: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpBrowserAdapterRegistrationRequest {
    actor: BrowserSessionActor,
    sensitivity: BrowserSensitivity,
    manifest: BrowserAdapterManifestRequest,
}

#[derive(Clone, Debug, Default)]
struct BrowserAdapterLaunchClaimValidation {
    server_issued_launch_request: bool,
    canonical_request_matched: bool,
    launch_request_unexpired: bool,
    launch_request_claim_bound: bool,
    launch_request_replay_detected: bool,
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
        browser_adapter_capabilities: Arc::new(Mutex::new(HashMap::new())),
        browser_adapter_launch_requests: Arc::new(Mutex::new(HashMap::new())),
    };
    Router::new()
        .route("/v1/health", get(health))
        .route("/openapi.json", get(openapi))
        .route("/v1/capabilities", get(capabilities))
        .route("/v1/browser/profiles", get(browser_profiles))
        .route("/v1/browser/admit", post(browser_admit))
        .route(
            "/v1/browser/adapter/contract",
            get(browser_adapter_contract_get),
        )
        .route(
            "/v1/browser/adapter/capability",
            post(browser_adapter_capability_issue),
        )
        .route(
            "/v1/browser/adapter/register",
            post(browser_adapter_register),
        )
        .route(
            "/v1/browser/adapter/launch/plan",
            post(browser_adapter_launch_plan),
        )
        .route(
            "/v1/browser/adapter/launch/claim",
            post(browser_adapter_launch_claim),
        )
        .route(
            "/v1/browser/adapter/validate",
            post(browser_adapter_validate),
        )
        .route(
            "/v1/browser/adapter/completion/validate",
            post(browser_adapter_completion_validate),
        )
        .route("/v1/execute", post(execute))
        .route("/v1/jobs", post(create_job))
        .route("/v1/jobs/{id}", get(get_job).delete(cancel_job))
        .route("/mcp", get(mcp_get).post(mcp_post).options(mcp_options))
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

async fn browser_adapter_validate(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request<Body>,
) -> Result<Json<BrowserAdapterManifestResponse>, ApiError> {
    state.authorize(&headers)?;
    let request = parse_json_body(&state, request).await?;
    validate_browser_adapter_manifest_request(&request)
        .map_err(|message| ApiError::bad_request("invalid_browser_adapter_manifest", message))?;
    Ok(Json(browser_adapter_manifest_response(request)))
}

async fn browser_adapter_completion_validate(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request<Body>,
) -> Result<Json<BrowserAdapterCompletionValidationResponse>, ApiError> {
    state.authorize(&headers)?;
    let request = parse_json_body(&state, request).await?;
    validate_browser_adapter_completion_report_request(&request).map_err(|message| {
        ApiError::bad_request("invalid_browser_adapter_completion_report", message)
    })?;
    Ok(Json(browser_adapter_completion_validation_response(
        Some(&state),
        request,
    )))
}

async fn browser_adapter_register(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request<Body>,
) -> Result<Json<BrowserAdapterRegistrationResponse>, ApiError> {
    state.authorize(&headers)?;
    let request = parse_json_body(&state, request).await?;
    validate_browser_adapter_registration_request(&request).map_err(|message| {
        ApiError::bad_request("invalid_browser_adapter_registration", message)
    })?;
    Ok(Json(browser_adapter_registration_response(&state, request)))
}

async fn browser_adapter_launch_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request<Body>,
) -> Result<Json<BrowserAdapterLaunchPlanResponse>, ApiError> {
    state.authorize_required(&headers, "browser adapter launch planning")?;
    let request = parse_json_body(&state, request).await?;
    validate_browser_adapter_launch_plan_request(&request)
        .map_err(|message| ApiError::bad_request("invalid_browser_adapter_launch_plan", message))?;
    Ok(Json(browser_adapter_launch_plan_response(&state, request)))
}

async fn browser_adapter_launch_claim(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request<Body>,
) -> Result<Json<BrowserAdapterLaunchClaimResponse>, ApiError> {
    state.authorize(&headers)?;
    let request_value = parse_json_body::<Value>(&state, request).await?;
    validate_browser_adapter_launch_claim_wire(&request_value).map_err(|message| {
        ApiError::bad_request("invalid_browser_adapter_launch_claim", message)
    })?;
    let request: BrowserAdapterLaunchClaimRequest = serde_json::from_value(request_value)
        .map_err(|error| ApiError::bad_request("invalid_json", error.to_string()))?;
    validate_browser_adapter_launch_claim_request(&request).map_err(|message| {
        ApiError::bad_request("invalid_browser_adapter_launch_claim", message)
    })?;
    Ok(Json(browser_adapter_launch_claim_response(&state, request)))
}

async fn browser_adapter_contract_get(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<BrowserAdapterContractResponse>, ApiError> {
    state.authorize(&headers)?;
    Ok(Json(browser_adapter_contract_response()))
}

async fn browser_adapter_capability_issue(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request<Body>,
) -> Result<Json<BrowserAdapterCapabilityIssueResponse>, ApiError> {
    state.authorize_required(&headers, "browser adapter capability issuance")?;
    let request = parse_json_body(&state, request).await?;
    validate_browser_adapter_capability_issue_request(&request)
        .map_err(|message| ApiError::bad_request("invalid_browser_adapter_capability", message))?;
    Ok(Json(browser_adapter_capability_issue_response(
        &state, request,
    )?))
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

    fn authorize_required(&self, headers: &HeaderMap, surface: &str) -> Result<(), ApiError> {
        match &self.config.auth {
            AuthMode::None => Err(ApiError::unauthorized(format!(
                "{surface} requires daemon authentication to be configured"
            ))),
            AuthMode::Required { .. } => self.authorize(headers),
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
        aether_payment: aether_payment_capabilities(),
    }
}

fn aether_payment_capabilities() -> AetherPaymentContextCapabilities {
    AetherPaymentContextCapabilities {
        max_payment_header_bytes: MAX_AETHER_PAYMENT_HEADER_BYTES,
        ..AetherPaymentContextCapabilities::default()
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
            adapter: browser_adapter_contract(),
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

fn browser_adapter_contract() -> BrowserAdapterContract {
    BrowserAdapterContract {
        status: BrowserSandboxAvailability::Planned,
        ..BrowserAdapterContract::default()
    }
}

fn browser_adapter_handoff(
    launch_request_template: BrowserAdapterLaunchRequest,
) -> BrowserAdapterHandoff {
    let adapter = browser_adapter_contract();
    BrowserAdapterHandoff {
        contract_version: adapter.version,
        launch_endpoint: adapter.launch_endpoint,
        launchable: false,
        handoff_fields: adapter.handoff_fields,
        launch_request_template,
        required_completion_proofs: adapter.required_completion_proofs,
        completion_proof_contract: adapter.completion_proof_contract,
        unavailable_reason: adapter.unavailable_reason,
    }
}

fn browser_adapter_launch_request_template(
    request_id: &str,
    adapter_id: Option<String>,
    request: &BrowserAdmissionRequest,
    guard_plan: &BrowserAdmissionGuardPlan,
    required_completion_proofs: Vec<String>,
) -> BrowserAdapterLaunchRequest {
    let lease = browser_adapter_template_launch_lease();
    browser_adapter_launch_request_template_with_lease(
        request_id,
        adapter_id,
        request,
        guard_plan,
        required_completion_proofs,
        lease,
    )
}

fn browser_adapter_live_launch_request_template(
    request_id: &str,
    adapter_id: Option<String>,
    request: &BrowserAdmissionRequest,
    guard_plan: &BrowserAdmissionGuardPlan,
    required_completion_proofs: Vec<String>,
) -> BrowserAdapterLaunchRequest {
    let lease = browser_adapter_live_launch_lease();
    browser_adapter_launch_request_template_with_lease(
        request_id,
        adapter_id,
        request,
        guard_plan,
        required_completion_proofs,
        lease,
    )
}

#[derive(Clone, Debug)]
struct BrowserAdapterLaunchLease {
    issued_at: String,
    expires_at: String,
    max_session_seconds: u64,
}

fn browser_adapter_launch_request_template_with_lease(
    request_id: &str,
    adapter_id: Option<String>,
    request: &BrowserAdmissionRequest,
    guard_plan: &BrowserAdmissionGuardPlan,
    required_completion_proofs: Vec<String>,
    lease: BrowserAdapterLaunchLease,
) -> BrowserAdapterLaunchRequest {
    let adapter_contract = BrowserAdapterContract::default();
    BrowserAdapterLaunchRequest {
        request_id: request_id.to_string(),
        issued_at: lease.issued_at,
        expires_at: lease.expires_at,
        max_session_seconds: lease.max_session_seconds,
        adapter_id: adapter_id.clone(),
        contract_version: adapter_contract.version.clone(),
        requested_level: request.requested_level.clone(),
        actor: request.actor.clone(),
        sensitivity: request.sensitivity.clone(),
        sensitive_activity_mode: request.sensitive_activity_mode.clone(),
        target_origins: request.target_origins.clone(),
        credential_mode: request.credential_mode.clone(),
        artifact_mode: request.artifact_mode.clone(),
        requested_controls: request.required_controls.clone(),
        guard_plan: guard_plan.clone(),
        required_completion_proofs,
        completion_proof_contract: adapter_contract.completion_proof_contract.clone(),
        completion_report_template: browser_adapter_completion_report_template(
            request_id,
            adapter_id.as_deref(),
            &adapter_contract,
        ),
        same_user_capability_required: true,
        endpoint_network_policy_binding_required: true,
        replay_protection_required: true,
        notes: vec![
            "launch request template only; beatbox does not currently call adapter launch endpoints"
                .to_string(),
            "do not treat this envelope as a registration, trust, or launch grant".to_string(),
            "future launchers must enforce expires_at and reject replayed request_id values"
                .to_string(),
        ],
    }
}

fn browser_adapter_template_launch_lease() -> BrowserAdapterLaunchLease {
    BrowserAdapterLaunchLease {
        issued_at: browser_adapter_launch_template_issued_at(),
        expires_at: browser_adapter_launch_template_expires_at(),
        max_session_seconds: BROWSER_ADAPTER_LAUNCH_LEASE_SECONDS,
    }
}

fn browser_adapter_live_launch_lease() -> BrowserAdapterLaunchLease {
    let issued_at = Utc::now();
    let expires_at =
        issued_at + ChronoDuration::seconds(BROWSER_ADAPTER_LAUNCH_LEASE_SECONDS as i64);
    BrowserAdapterLaunchLease {
        issued_at: issued_at.to_rfc3339(),
        expires_at: expires_at.to_rfc3339(),
        max_session_seconds: BROWSER_ADAPTER_LAUNCH_LEASE_SECONDS,
    }
}

fn browser_adapter_completion_report_template(
    request_id: &str,
    adapter_id: Option<&str>,
    adapter_contract: &BrowserAdapterContract,
) -> BrowserAdapterCompletionReport {
    BrowserAdapterCompletionReport {
        request_id: request_id.to_string(),
        adapter_id: adapter_id
            .unwrap_or("adapter-id-bound-at-registration")
            .to_string(),
        contract_version: adapter_contract.version.clone(),
        process_terminated: true,
        temporary_profile_removed: true,
        plaintext_artifacts_removed: true,
        egress_log_sealed_or_discarded: true,
        sealed_artifact_handles: Vec::new(),
        proof_ids: adapter_contract
            .completion_proof_contract
            .iter()
            .map(|proof| proof.proof_id.clone())
            .collect(),
        notes: vec![
            "template only; not evidence of a real browser session".to_string(),
            "production completion must verify these booleans on the teardown path".to_string(),
            "sealed_artifact_handles must contain only storage handles, never raw secrets or browser state"
                .to_string(),
        ],
    }
}

fn browser_adapter_contract_response() -> BrowserAdapterContractResponse {
    let adapter_contract = browser_adapter_contract();
    let required_levels = browser_adapter_required_levels();
    let required_controls = browser_adapter_required_controls(&required_levels);
    let conformance_profile = browser_adapter_conformance_profile(
        &adapter_contract,
        &required_levels,
        &required_controls,
    );
    BrowserAdapterContractResponse {
        adapter_contract,
        conformance_profile,
        required_levels,
        required_controls,
        launchable: false,
        trusted_for_sensitive_work: false,
        endpoint_network_policy_bound: false,
        notes: vec![
            "browser adapter contract discovery is compatibility metadata, not adapter registration"
                .to_string(),
            "no browser adapter is trusted or launchable until same-user registration and endpoint binding are implemented"
                .to_string(),
            "run the conformance_profile cases against REST and MCP before advertising adapter compatibility"
                .to_string(),
        ],
    }
}

fn browser_adapter_capability_issue_response(
    state: &AppState,
    request: BrowserAdapterCapabilityIssueRequest,
) -> Result<BrowserAdapterCapabilityIssueResponse, ApiError> {
    let ttl_seconds = request
        .ttl_seconds
        .unwrap_or(BROWSER_ADAPTER_CAPABILITY_TTL_SECONDS);
    let token = format!(
        "bbx-browser-adapter-cap-v1.{}.{}",
        uuid::Uuid::new_v4(),
        uuid::Uuid::new_v4()
    );
    let digest = browser_adapter_capability_digest(&token);
    let now = Instant::now();
    let expires_at = now + Duration::from_secs(ttl_seconds);
    {
        let mut capabilities = match state.browser_adapter_capabilities.lock() {
            Ok(capabilities) => capabilities,
            Err(poisoned) => poisoned.into_inner(),
        };
        capabilities.retain(|_, record| !record.used && record.expires_at > now);
        if capabilities.len() >= MAX_BROWSER_ADAPTER_CAPABILITIES {
            return Err(ApiError::too_many(
                "browser_adapter_capability_quota",
                format!(
                    "maximum live browser adapter capabilities ({MAX_BROWSER_ADAPTER_CAPABILITIES}) are already issued"
                ),
            ));
        }
        capabilities.insert(
            digest,
            BrowserAdapterCapabilityRecord {
                actor: request.actor.clone(),
                sensitivity: request.sensitivity.clone(),
                sensitive_activity_mode: request.sensitive_activity_mode.clone(),
                adapter_id: request.adapter_id.clone(),
                expires_at,
                used: false,
            },
        );
    }
    Ok(BrowserAdapterCapabilityIssueResponse {
        same_user_capability: token,
        expires_at: (Utc::now() + ChronoDuration::seconds(ttl_seconds as i64)).to_rfc3339(),
        ttl_seconds,
        actor: request.actor,
        sensitivity: request.sensitivity,
        sensitive_activity_mode: request.sensitive_activity_mode,
        adapter_id: request.adapter_id,
        registration_endpoint: "/v1/browser/adapter/register".to_string(),
        notes: vec![
            "same_user_capability is bearer material; keep it out of logs and model-visible transcripts"
                .to_string(),
            "beatbox stores only a digest and consumes the capability on the first matching registration or launch-plan preflight"
                .to_string(),
            "a bound capability still does not make an adapter registered, trusted, or launchable"
                .to_string(),
        ],
    })
}

fn browser_adapter_capability_digest(token: &str) -> [u8; 32] {
    let digest = Sha256::digest(token.as_bytes());
    let mut out = [0_u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn browser_adapter_consume_capability(
    state: &AppState,
    request: &BrowserAdapterRegistrationRequest,
) -> bool {
    browser_adapter_consume_capability_for(
        state,
        &request.same_user_capability,
        &request.actor,
        &request.sensitivity,
        None,
        &request.manifest.adapter_id,
    )
}

fn browser_adapter_consume_capability_for(
    state: &AppState,
    same_user_capability: &str,
    actor: &BrowserSessionActor,
    sensitivity: &BrowserSensitivity,
    sensitive_activity_mode: Option<&BrowserSensitiveActivityMode>,
    adapter_id: &str,
) -> bool {
    let digest = browser_adapter_capability_digest(same_user_capability);
    let now = Instant::now();
    let mut capabilities = match state.browser_adapter_capabilities.lock() {
        Ok(capabilities) => capabilities,
        Err(poisoned) => poisoned.into_inner(),
    };
    capabilities.retain(|_, record| !record.used && record.expires_at > now);
    let Some(record) = capabilities.get_mut(&digest) else {
        return false;
    };
    let adapter_matches = match &record.adapter_id {
        Some(bound_adapter_id) => bound_adapter_id == adapter_id,
        None => true,
    };
    let sensitive_activity_mode_matches = match &record.sensitive_activity_mode {
        Some(bound_mode) => Some(bound_mode) == sensitive_activity_mode,
        None => true,
    };
    if &record.actor == actor
        && &record.sensitivity == sensitivity
        && sensitive_activity_mode_matches
        && adapter_matches
        && !record.used
        && record.expires_at > now
    {
        record.used = true;
        true
    } else {
        false
    }
}

fn browser_adapter_record_launch_request(
    state: &AppState,
    launch_request: &BrowserAdapterLaunchRequest,
) -> bool {
    let now = Instant::now();
    let mut launch_requests = match state.browser_adapter_launch_requests.lock() {
        Ok(launch_requests) => launch_requests,
        Err(poisoned) => poisoned.into_inner(),
    };
    launch_requests.retain(|_, record| record.expires_at > now);
    if launch_requests.len() >= MAX_BROWSER_ADAPTER_LAUNCH_REQUESTS {
        return false;
    }
    if launch_request.adapter_id.is_none() {
        return false;
    }
    launch_requests.insert(
        launch_request.request_id.clone(),
        BrowserAdapterLaunchRequestRecord {
            launch_request: launch_request.clone(),
            expires_at: now + Duration::from_secs(launch_request.max_session_seconds),
            claimed: false,
        },
    );
    true
}

fn browser_adapter_claim_launch_request(
    state: &AppState,
    launch_request: &BrowserAdapterLaunchRequest,
) -> BrowserAdapterLaunchClaimValidation {
    let now = Instant::now();
    let mut launch_requests = match state.browser_adapter_launch_requests.lock() {
        Ok(launch_requests) => launch_requests,
        Err(poisoned) => poisoned.into_inner(),
    };
    launch_requests.retain(|_, record| record.expires_at > now);
    let Some(record) = launch_requests.get_mut(&launch_request.request_id) else {
        return BrowserAdapterLaunchClaimValidation::default();
    };
    let launch_request_unexpired = record.expires_at > now;
    let canonical_request_matched = record.launch_request == *launch_request;
    let launch_request_replay_detected = record.claimed;
    let launch_request_claim_bound =
        launch_request_unexpired && canonical_request_matched && !record.claimed;
    if launch_request_claim_bound {
        record.claimed = true;
    }
    BrowserAdapterLaunchClaimValidation {
        server_issued_launch_request: true,
        canonical_request_matched,
        launch_request_unexpired,
        launch_request_claim_bound,
        launch_request_replay_detected,
    }
}

fn browser_adapter_manifest_response(
    request: BrowserAdapterManifestRequest,
) -> BrowserAdapterManifestResponse {
    let adapter_contract = browser_adapter_contract();
    let required_levels = browser_adapter_required_levels();
    let required_controls = browser_adapter_required_controls(&required_levels);
    let missing_levels = required_levels
        .iter()
        .filter(|level| !request.supported_levels.contains(level))
        .cloned()
        .collect::<Vec<_>>();
    let missing_controls = required_controls
        .iter()
        .filter(|control| !request.supported_controls.contains(control))
        .cloned()
        .collect::<Vec<_>>();
    let missing_guard_fields = adapter_contract
        .required_guard_fields
        .iter()
        .filter(|field| !request.guard_fields.contains(field))
        .cloned()
        .collect::<Vec<_>>();
    let missing_completion_proofs = adapter_contract
        .required_completion_proofs
        .iter()
        .filter(|proof| !request.completion_proofs.contains(proof))
        .cloned()
        .collect::<Vec<_>>();
    let mut reasons = vec![
        "external browser adapter manifests are validation metadata only; beatbox does not trust or launch them yet"
            .to_string(),
    ];
    if request.contract_version != adapter_contract.version {
        reasons.push(format!(
            "adapter contract_version `{}` does not match required `{}`",
            request.contract_version, adapter_contract.version
        ));
    }
    if request.launch_endpoint.is_none() {
        reasons.push("adapter manifest does not provide a launch_endpoint".to_string());
    } else {
        reasons.push(
            "adapter launch_endpoint passed syntax checks only; DNS, proxy, redirect, and retry network-policy binding is not implemented"
                .to_string(),
        );
    }
    if request
        .supported_levels
        .contains(&BrowserSandboxLevel::InstrumentedExternal)
    {
        reasons.push(
            "instrumented_external is not accepted as a sandbox-capable adapter level".to_string(),
        );
    }
    if !missing_levels.is_empty() {
        reasons.push("adapter does not claim every required sandbox level".to_string());
    }
    if !missing_controls.is_empty() {
        reasons.push("adapter does not claim every required sandbox control".to_string());
    }
    if !missing_guard_fields.is_empty() {
        reasons.push("adapter does not bind every required guard_plan field".to_string());
    }
    if !missing_completion_proofs.is_empty() {
        reasons.push("adapter does not report every required completion proof".to_string());
    }
    let endpoint_network_policy_bound = false;
    let contract_fields_complete = request.contract_version == adapter_contract.version
        && request.launch_endpoint.is_some()
        && !request
            .supported_levels
            .contains(&BrowserSandboxLevel::InstrumentedExternal)
        && missing_levels.is_empty()
        && missing_controls.is_empty()
        && missing_guard_fields.is_empty()
        && missing_completion_proofs.is_empty();
    let manifest_complete = contract_fields_complete && endpoint_network_policy_bound;
    if contract_fields_complete {
        reasons.push(
            "adapter manifest satisfies the published field contract, but no trusted adapter registration, endpoint binding, or launch path is implemented"
                .to_string(),
        );
    }
    let conformance_profile = browser_adapter_conformance_profile(
        &adapter_contract,
        &required_levels,
        &required_controls,
    );

    BrowserAdapterManifestResponse {
        decision: BrowserAdapterValidationDecision::Rejected,
        manifest_complete,
        launchable: false,
        trusted_for_sensitive_work: false,
        adapter_id: request.adapter_id,
        launch_endpoint: request.launch_endpoint,
        endpoint_network_policy_bound,
        missing_levels,
        missing_controls,
        missing_guard_fields,
        missing_completion_proofs,
        reasons,
        required_next_steps: vec![
            "implement authenticated adapter registration with same-user capability binding"
                .to_string(),
            "bind the launch endpoint to DNS, proxy, redirect, retry, and production request-builder network policy"
                .to_string(),
            "verify adapter completion proofs on the production browser teardown path".to_string(),
            "run e2e sensitive-browser tests before marking any adapter launchable".to_string(),
        ],
        adapter_contract,
        conformance_profile,
    }
}

fn browser_adapter_manifest_contract_fields_complete(
    request: &BrowserAdapterManifestRequest,
) -> bool {
    let adapter_contract = browser_adapter_contract();
    let required_levels = browser_adapter_required_levels();
    let required_controls = browser_adapter_required_controls(&required_levels);
    request.contract_version == adapter_contract.version
        && request.launch_endpoint.is_some()
        && !request
            .supported_levels
            .contains(&BrowserSandboxLevel::InstrumentedExternal)
        && required_levels
            .iter()
            .all(|level| request.supported_levels.contains(level))
        && required_controls
            .iter()
            .all(|control| request.supported_controls.contains(control))
        && adapter_contract
            .required_guard_fields
            .iter()
            .all(|field| request.guard_fields.contains(field))
        && adapter_contract
            .required_completion_proofs
            .iter()
            .all(|proof| request.completion_proofs.contains(proof))
}

#[derive(Clone, Debug, Default)]
struct BrowserAdapterCompletionLaunchBinding {
    server_issued_launch_request: bool,
    launch_request_claimed: bool,
    launch_request_envelope_matched: bool,
    completion_report_template_matched: bool,
}

fn browser_adapter_completion_launch_binding(
    state: &AppState,
    report: &BrowserAdapterCompletionReport,
) -> BrowserAdapterCompletionLaunchBinding {
    let now = Instant::now();
    let mut launch_requests = match state.browser_adapter_launch_requests.lock() {
        Ok(launch_requests) => launch_requests,
        Err(poisoned) => poisoned.into_inner(),
    };
    launch_requests.retain(|_, record| record.expires_at > now);
    let Some(record) = launch_requests.get(&report.request_id) else {
        return BrowserAdapterCompletionLaunchBinding::default();
    };
    let launch_request_envelope_matched = record
        .launch_request
        .adapter_id
        .as_deref()
        .is_some_and(|adapter_id| adapter_id == report.adapter_id)
        && record.launch_request.contract_version == report.contract_version;
    let completion_report_template_matched =
        record.launch_request.completion_report_template == *report;
    BrowserAdapterCompletionLaunchBinding {
        server_issued_launch_request: true,
        launch_request_claimed: record.claimed,
        launch_request_envelope_matched,
        completion_report_template_matched,
    }
}

fn browser_adapter_completion_validation_response(
    state: Option<&AppState>,
    report: BrowserAdapterCompletionReport,
) -> BrowserAdapterCompletionValidationResponse {
    let adapter_contract = browser_adapter_contract();
    let launch_binding = state
        .map(|state| browser_adapter_completion_launch_binding(state, &report))
        .unwrap_or_default();
    let required_proof_ids = adapter_contract
        .completion_proof_contract
        .iter()
        .map(|proof| proof.proof_id.clone())
        .collect::<Vec<_>>();
    let missing_proof_ids = required_proof_ids
        .iter()
        .filter(|proof_id| !report.proof_ids.contains(proof_id))
        .cloned()
        .collect::<Vec<_>>();
    let unexpected_proof_ids = report
        .proof_ids
        .iter()
        .filter(|proof_id| !required_proof_ids.contains(proof_id))
        .cloned()
        .collect::<Vec<_>>();
    let mut failed_evidence_fields = Vec::new();
    if !report.process_terminated {
        failed_evidence_fields.push("process_terminated".to_string());
    }
    if !report.temporary_profile_removed {
        failed_evidence_fields.push("temporary_profile_removed".to_string());
    }
    if !report.plaintext_artifacts_removed {
        failed_evidence_fields.push("plaintext_artifacts_removed".to_string());
    }
    if !report.egress_log_sealed_or_discarded {
        failed_evidence_fields.push("egress_log_sealed_or_discarded".to_string());
    }

    let mut reasons = vec![
        "browser adapter completion reports are validation metadata only; beatbox does not have a production browser launch or teardown path to bind them to yet"
            .to_string(),
    ];
    if report.contract_version != adapter_contract.version {
        reasons.push(format!(
            "completion report contract_version `{}` does not match required `{}`",
            report.contract_version, adapter_contract.version
        ));
    }
    if !missing_proof_ids.is_empty() {
        reasons.push("completion report does not include every required proof id".to_string());
    }
    if !unexpected_proof_ids.is_empty() {
        reasons.push(
            "completion report includes proof ids outside the published contract".to_string(),
        );
    }
    if !failed_evidence_fields.is_empty() {
        reasons.push(
            "completion report has required teardown evidence fields set to false".to_string(),
        );
    }

    let report_shape_complete = report.contract_version == adapter_contract.version
        && missing_proof_ids.is_empty()
        && unexpected_proof_ids.is_empty()
        && failed_evidence_fields.is_empty();
    if report_shape_complete {
        reasons.push(
            "completion report satisfies the published shape, but Beatbox has not verified it on a real launch request, process, profile directory, artifact store, or egress log"
                .to_string(),
        );
    }
    if state.is_none() {
        reasons.push(
            "MCP completion validation is shape-only and does not expose live launch-ledger state"
                .to_string(),
        );
    } else if launch_binding.server_issued_launch_request {
        if launch_binding.launch_request_envelope_matched {
            reasons.push(
                "completion report request_id, adapter_id, and contract_version match this daemon's recorded launch envelope"
                    .to_string(),
            );
        } else {
            reasons.push(
                "completion report request_id exists in this daemon's launch ledger, but adapter_id or contract_version does not match the recorded envelope"
                    .to_string(),
            );
        }
        if launch_binding.completion_report_template_matched {
            reasons.push(
                "completion report exactly matches the template embedded in the recorded launch envelope"
                    .to_string(),
            );
        } else {
            reasons.push(
                "completion report does not exactly match the template embedded in the recorded launch envelope"
                    .to_string(),
            );
        }
        if launch_binding.launch_request_claimed {
            reasons.push(
                "matching launch request was claimed through the REST launch-claim preflight"
                    .to_string(),
            );
        } else {
            reasons.push(
                "matching launch request has not been claimed through the REST launch-claim preflight"
                    .to_string(),
            );
        }
    } else {
        reasons.push(
            "completion report request_id is not present in this daemon's bounded launch replay ledger"
                .to_string(),
        );
    }
    let completion_bound_to_claimed_launch = report_shape_complete
        && launch_binding.server_issued_launch_request
        && launch_binding.launch_request_claimed
        && launch_binding.launch_request_envelope_matched
        && launch_binding.completion_report_template_matched;

    BrowserAdapterCompletionValidationResponse {
        decision: BrowserAdapterCompletionValidationDecision::Rejected,
        report_shape_complete,
        server_issued_launch_request: launch_binding.server_issued_launch_request,
        launch_request_claimed: launch_binding.launch_request_claimed,
        launch_request_envelope_matched: launch_binding.launch_request_envelope_matched,
        completion_report_template_matched: launch_binding.completion_report_template_matched,
        completion_bound_to_claimed_launch,
        verified_on_production_path: false,
        trusted_for_sensitive_work: false,
        request_id: report.request_id,
        adapter_id: report.adapter_id,
        contract_version: report.contract_version,
        missing_proof_ids,
        unexpected_proof_ids,
        failed_evidence_fields,
        required_completion_proofs: adapter_contract.required_completion_proofs.clone(),
        completion_proof_contract: adapter_contract.completion_proof_contract.clone(),
        reasons,
        required_next_steps: vec![
            "bind completion reports to a claimed server-issued launch request and trusted registered adapter"
                .to_string(),
            "verify process termination, profile deletion, artifact cleanup, and egress log handling on the production teardown path"
                .to_string(),
            "reject completion reports that cannot be derived from the concrete launched browser session"
                .to_string(),
        ],
        adapter_contract,
    }
}

fn browser_adapter_registration_response(
    state: &AppState,
    request: BrowserAdapterRegistrationRequest,
) -> BrowserAdapterRegistrationResponse {
    let same_user_capability_bound = browser_adapter_consume_capability(state, &request);
    let actor = request.actor.clone();
    let sensitivity = request.sensitivity.clone();
    let manifest_validation = browser_adapter_manifest_response(request.manifest);
    let adapter_id = manifest_validation.adapter_id.clone();
    let mut reasons = vec![
        "browser adapter registration is a fail-closed preflight; beatbox does not persist or trust adapters yet"
            .to_string(),
        "launch endpoint binding to DNS, proxy, redirect, retry, and request-builder policy is not implemented"
            .to_string(),
        "browser launch, teardown proof verification, and storage sealing are not implemented"
            .to_string(),
    ];
    if same_user_capability_bound {
        reasons.push(
            "same-user capability matched this registration preflight, but adapter persistence and launch trust remain disabled"
                .to_string(),
        );
    } else {
        reasons.push(
            "same-user capability was not issued by this daemon, was already used, expired, or did not match actor, sensitivity, adapter_id, and any bound sensitive_activity_mode"
                .to_string(),
        );
    }
    let mut required_next_steps = vec![
        "bind adapter registration to the concrete endpoint used after DNS, proxy, redirects, and retries"
            .to_string(),
        "store adapter registrations only after conformance and endpoint policy checks pass"
            .to_string(),
        "verify teardown and artifact proofs on the production browser completion path".to_string(),
    ];
    if !same_user_capability_bound {
        required_next_steps.insert(
            0,
            "issue and submit a live same-user capability from the local authenticated control plane"
                .to_string(),
        );
    }
    BrowserAdapterRegistrationResponse {
        decision: BrowserAdapterRegistrationDecision::Rejected,
        adapter_id,
        actor,
        sensitivity,
        registered: false,
        launchable: false,
        trusted_for_sensitive_work: false,
        endpoint_network_policy_bound: false,
        same_user_capability_bound,
        manifest_validation,
        reasons,
        required_next_steps,
    }
}

fn browser_adapter_mcp_registration_response(
    request: McpBrowserAdapterRegistrationRequest,
) -> BrowserAdapterRegistrationResponse {
    let actor = request.actor;
    let sensitivity = request.sensitivity;
    let manifest_validation = browser_adapter_manifest_response(request.manifest);
    let adapter_id = manifest_validation.adapter_id.clone();
    BrowserAdapterRegistrationResponse {
        decision: BrowserAdapterRegistrationDecision::Rejected,
        adapter_id,
        actor,
        sensitivity,
        registered: false,
        launchable: false,
        trusted_for_sensitive_work: false,
        endpoint_network_policy_bound: false,
        same_user_capability_bound: false,
        manifest_validation,
        reasons: vec![
            "MCP register_browser_adapter is manifest-only because same-user capabilities are bearer material and must stay out of model-visible transcripts"
                .to_string(),
            "capability-bound adapter registration is available only through the authenticated REST control plane"
                .to_string(),
            "browser launch, teardown proof verification, and storage sealing are not implemented"
                .to_string(),
        ],
        required_next_steps: vec![
            "issue and submit a live same-user capability through the REST registration endpoint, not MCP"
                .to_string(),
            "bind adapter registration to the concrete endpoint used after DNS, proxy, redirects, and retries"
                .to_string(),
            "verify teardown and artifact proofs on the production browser completion path".to_string(),
        ],
    }
}

fn browser_adapter_launch_plan_response(
    state: &AppState,
    request: BrowserAdapterLaunchPlanRequest,
) -> BrowserAdapterLaunchPlanResponse {
    let same_user_capability_bound = browser_adapter_consume_capability_for(
        state,
        &request.same_user_capability,
        &request.admission.actor,
        &request.admission.sensitivity,
        Some(&request.admission.sensitive_activity_mode),
        &request.manifest.adapter_id,
    );
    let request_id = format!("bbx-browser-launch-plan-v1.{}", uuid::Uuid::new_v4());
    let adapter_id = request.manifest.adapter_id.clone();
    let actor = request.admission.actor.clone();
    let sensitivity = request.admission.sensitivity.clone();
    let adapter_contract_fields_complete =
        browser_adapter_manifest_contract_fields_complete(&request.manifest);
    let admission = browser_admission_response(request.admission.clone());
    let manifest_validation = browser_adapter_manifest_response(request.manifest);
    let launch_request = browser_adapter_live_launch_request_template(
        &request_id,
        Some(adapter_id.clone()),
        &request.admission,
        &admission.guard_plan,
        browser_adapter_contract().required_completion_proofs,
    );
    let replay_protection_bound = same_user_capability_bound
        && adapter_contract_fields_complete
        && browser_adapter_record_launch_request(state, &launch_request);
    let mut reasons = vec![
        "browser adapter launch planning is a fail-closed compatibility preflight; beatbox does not call adapter launch endpoints yet"
            .to_string(),
        "browser launcher, endpoint request-builder binding, teardown verification, and storage sealing are not implemented"
            .to_string(),
    ];
    if same_user_capability_bound {
        reasons.push(
            "same-user capability matched this launch plan preflight, but it is not registration, endpoint trust, or launch permission"
                .to_string(),
        );
        if replay_protection_bound {
            reasons.push(
                "launch request id was recorded in this daemon's bounded replay ledger for the REST claim preflight"
                    .to_string(),
            );
        } else {
            reasons.push(
                "launch request id was not recorded in this daemon's replay ledger because the adapter field contract was incomplete or the ledger was full"
                    .to_string(),
            );
        }
    } else {
        reasons.push(
            "same-user capability was not issued by this daemon, was already used, expired, or did not match the admission actor, sensitivity, sensitive_activity_mode, and adapter_id"
                .to_string(),
        );
    }
    let mut required_next_steps = vec![
        "persist trusted adapter registrations only after conformance checks pass".to_string(),
        "bind the launch endpoint to DNS, proxy, redirect, retry, and production request-builder network policy"
            .to_string(),
        "execute the launch_request only through a real isolated browser launcher".to_string(),
        "verify completion reports against the concrete launched process, profile directory, artifact store, and egress log"
            .to_string(),
    ];
    if !same_user_capability_bound {
        required_next_steps.insert(
            0,
            "issue and submit a live same-user capability from the local authenticated control plane"
                .to_string(),
        );
    }

    BrowserAdapterLaunchPlanResponse {
        decision: BrowserAdapterLaunchPlanDecision::Rejected,
        request_id,
        adapter_id,
        actor,
        sensitivity,
        launchable: false,
        trusted_for_sensitive_work: false,
        endpoint_network_policy_bound: false,
        adapter_contract_fields_complete,
        same_user_capability_bound,
        replay_protection_bound,
        admission,
        manifest_validation,
        launch_request,
        completion_validation_endpoint: "/v1/browser/adapter/completion/validate".to_string(),
        reasons,
        required_next_steps,
    }
}

fn browser_adapter_launch_claim_response(
    state: &AppState,
    request: BrowserAdapterLaunchClaimRequest,
) -> BrowserAdapterLaunchClaimResponse {
    let launch_request = request.launch_request;
    let claim = browser_adapter_claim_launch_request(state, &launch_request);
    let mut reasons = vec![
        "browser adapter launch claim is a REST-only replay preflight; beatbox still does not call adapter launch endpoints"
            .to_string(),
    ];
    if claim.launch_request_claim_bound {
        reasons.push(
            "launch request matched this daemon's canonical envelope and was claimed exactly once"
                .to_string(),
        );
    } else if claim.launch_request_replay_detected {
        reasons.push("launch request id was already claimed".to_string());
    } else if !claim.server_issued_launch_request {
        reasons.push(
            "launch request id is not present in this daemon's bounded launch replay ledger"
                .to_string(),
        );
    } else if !claim.launch_request_unexpired {
        reasons.push("launch request lease has expired".to_string());
    } else if !claim.canonical_request_matched {
        reasons.push(
            "launch request does not match the canonical envelope recorded during launch planning"
                .to_string(),
        );
    }

    BrowserAdapterLaunchClaimResponse {
        decision: if claim.launch_request_claim_bound {
            BrowserAdapterLaunchClaimDecision::Claimed
        } else {
            BrowserAdapterLaunchClaimDecision::Rejected
        },
        request_id: launch_request.request_id,
        adapter_id: launch_request.adapter_id,
        server_issued_launch_request: claim.server_issued_launch_request,
        canonical_request_matched: claim.canonical_request_matched,
        launch_request_unexpired: claim.launch_request_unexpired,
        launch_request_claim_bound: claim.launch_request_claim_bound,
        launch_request_replay_detected: claim.launch_request_replay_detected,
        launchable: false,
        trusted_for_sensitive_work: false,
        endpoint_network_policy_bound: false,
        reasons,
        required_next_steps: vec![
            "bind the adapter launch endpoint to the production request builder after DNS, proxy, redirects, and retries"
                .to_string(),
            "execute the claimed launch request only through a real isolated browser launcher"
                .to_string(),
            "verify process, profile, artifact, and egress teardown before trusting completion"
                .to_string(),
        ],
    }
}

fn browser_adapter_conformance_profile(
    adapter_contract: &BrowserAdapterContract,
    required_levels: &[BrowserSandboxLevel],
    required_controls: &[BrowserSandboxControl],
) -> BrowserAdapterConformanceProfile {
    let field_complete_manifest = browser_adapter_field_complete_manifest(
        adapter_contract,
        required_levels,
        required_controls,
        "tempo-conformance-adapter-v1",
        Some("https://adapter.example/launch".to_string()),
    );
    let field_complete_launch_request = browser_adapter_field_complete_launch_request(
        adapter_contract,
        required_controls,
        &field_complete_manifest.adapter_id,
    );

    let mut missing_level_manifest = field_complete_manifest.clone();
    missing_level_manifest.supported_levels = vec![BrowserSandboxLevel::NetworkSuppressed];
    let missing_level_expectation = browser_adapter_conformance_expectation(
        vec![
            BrowserSandboxLevel::EphemeralProfile,
            BrowserSandboxLevel::SealedState,
            BrowserSandboxLevel::OsIsolated,
            BrowserSandboxLevel::RemoteIsolated,
        ],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );

    let mut external_level_manifest = field_complete_manifest.clone();
    external_level_manifest
        .supported_levels
        .push(BrowserSandboxLevel::InstrumentedExternal);

    let mut http_endpoint_manifest = field_complete_manifest.clone();
    http_endpoint_manifest.launch_endpoint = Some("http://adapter.example/launch".to_string());

    let mut dns_rebinding_manifest = field_complete_manifest.clone();
    dns_rebinding_manifest.launch_endpoint = Some("https://127.0.0.1.nip.io/launch".to_string());

    BrowserAdapterConformanceProfile {
        profile_version: "browser-adapter-conformance-v1".to_string(),
        field_complete_manifest: field_complete_manifest.clone(),
        field_complete_launch_request,
        field_complete_expectation: browser_adapter_conformance_expectation(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
        required_cases: vec![
            BrowserAdapterConformanceCase {
                name: "field_complete_manifest_stays_fail_closed".to_string(),
                manifest: field_complete_manifest,
                expected_rest_status: StatusCode::OK.as_u16(),
                expected_rest_error_code: None,
                expected_mcp_error_code: None,
                expected_mcp_error_message_contains: Vec::new(),
                expected_validation: Some(browser_adapter_conformance_expectation(
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                )),
                notes: vec![
                    "A field-complete manifest is conformance metadata only; it must not become launchable until endpoint binding and trusted registration exist."
                        .to_string(),
                ],
            },
            BrowserAdapterConformanceCase {
                name: "missing_required_level_reports_gap".to_string(),
                manifest: missing_level_manifest,
                expected_rest_status: StatusCode::OK.as_u16(),
                expected_rest_error_code: None,
                expected_mcp_error_code: None,
                expected_mcp_error_message_contains: Vec::new(),
                expected_validation: Some(missing_level_expectation),
                notes: vec![
                    "The response must remain rejected and list missing sandbox levels instead of silently downgrading."
                        .to_string(),
                ],
            },
            BrowserAdapterConformanceCase {
                name: "instrumented_external_not_sandbox_capable".to_string(),
                manifest: external_level_manifest,
                expected_rest_status: StatusCode::OK.as_u16(),
                expected_rest_error_code: None,
                expected_mcp_error_code: None,
                expected_mcp_error_message_contains: Vec::new(),
                expected_validation: Some(browser_adapter_conformance_expectation(
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                )),
                notes: vec![
                    "Adapters must not claim instrumented_external as a sensitive-work sandbox level."
                        .to_string(),
                ],
            },
            BrowserAdapterConformanceCase {
                name: "insecure_scheme_rejected_before_validation".to_string(),
                manifest: http_endpoint_manifest,
                expected_rest_status: StatusCode::BAD_REQUEST.as_u16(),
                expected_rest_error_code: Some("invalid_browser_adapter_manifest".to_string()),
                expected_mcp_error_code: Some(-32602),
                expected_mcp_error_message_contains: vec!["must use https".to_string()],
                expected_validation: None,
                notes: vec![
                    "Endpoint shape errors fail at request validation before a manifest response is emitted; MCP reports the same parser failure as a JSON-RPC invalid-params error."
                        .to_string(),
                ],
            },
            BrowserAdapterConformanceCase {
                name: "dns_rebinding_hostname_stays_incomplete".to_string(),
                manifest: dns_rebinding_manifest,
                expected_rest_status: StatusCode::OK.as_u16(),
                expected_rest_error_code: None,
                expected_mcp_error_code: None,
                expected_mcp_error_message_contains: Vec::new(),
                expected_validation: Some(browser_adapter_conformance_expectation(
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                )),
                notes: vec![
                    "A syntactically valid hostname can still resolve to local/private space; conformance requires endpoint_network_policy_bound=false."
                        .to_string(),
                ],
            },
        ],
        notes: vec![
            "Run these cases against both REST and MCP integrations before treating an adapter as compatible; use the protocol-specific expected_rest_* and expected_mcp_* fields."
                .to_string(),
            "The profile is not a registration grant and does not authorize browser launch."
                .to_string(),
            "Production registration must bind DNS, proxy, redirects, retries, and the request builder to the same endpoint policy."
                .to_string(),
        ],
    }
}

fn browser_adapter_conformance_expectation(
    missing_levels: Vec<BrowserSandboxLevel>,
    missing_controls: Vec<BrowserSandboxControl>,
    missing_guard_fields: Vec<String>,
    missing_completion_proofs: Vec<String>,
) -> BrowserAdapterConformanceExpectation {
    BrowserAdapterConformanceExpectation {
        decision: BrowserAdapterValidationDecision::Rejected,
        manifest_complete: false,
        launchable: false,
        trusted_for_sensitive_work: false,
        endpoint_network_policy_bound: false,
        missing_levels,
        missing_controls,
        missing_guard_fields,
        missing_completion_proofs,
    }
}

fn browser_adapter_field_complete_manifest(
    adapter_contract: &BrowserAdapterContract,
    required_levels: &[BrowserSandboxLevel],
    required_controls: &[BrowserSandboxControl],
    adapter_id: &str,
    launch_endpoint: Option<String>,
) -> BrowserAdapterManifestRequest {
    BrowserAdapterManifestRequest {
        adapter_id: adapter_id.to_string(),
        contract_version: adapter_contract.version.clone(),
        launch_endpoint,
        supported_levels: required_levels.to_vec(),
        supported_controls: required_controls.to_vec(),
        guard_fields: adapter_contract.required_guard_fields.clone(),
        completion_proofs: adapter_contract.required_completion_proofs.clone(),
    }
}

fn browser_adapter_field_complete_launch_request(
    adapter_contract: &BrowserAdapterContract,
    required_controls: &[BrowserSandboxControl],
    adapter_id: &str,
) -> BrowserAdapterLaunchRequest {
    let request = BrowserAdmissionRequest {
        requested_level: BrowserSandboxLevel::OsIsolated,
        actor: BrowserSessionActor::Agent,
        sensitivity: BrowserSensitivity::Sensitive,
        sensitive_activity_mode: BrowserSensitiveActivityMode::NetworkSuppressed,
        target_origins: vec!["https://example.com".to_string()],
        credential_mode: BrowserCredentialMode::NoCredentials,
        artifact_mode: BrowserArtifactMode::Discard,
        required_controls: required_controls.to_vec(),
        allow_downgrade: false,
        task_label: Some("browser adapter conformance launch".to_string()),
    };
    let requested_profile_controls = browser_profile_controls(&request.requested_level);
    let guard_plan = browser_admission_guard_plan(&request, &requested_profile_controls);
    browser_adapter_launch_request_template(
        "browser-adapter-conformance-launch-v1",
        Some(adapter_id.to_string()),
        &request,
        &guard_plan,
        adapter_contract.required_completion_proofs.clone(),
    )
}

fn browser_adapter_required_levels() -> Vec<BrowserSandboxLevel> {
    vec![
        BrowserSandboxLevel::EphemeralProfile,
        BrowserSandboxLevel::NetworkSuppressed,
        BrowserSandboxLevel::SealedState,
        BrowserSandboxLevel::OsIsolated,
        BrowserSandboxLevel::RemoteIsolated,
    ]
}

fn browser_adapter_required_controls(
    required_levels: &[BrowserSandboxLevel],
) -> Vec<BrowserSandboxControl> {
    let mut controls = Vec::new();
    for level in required_levels {
        for control in browser_profile_controls(level) {
            if !controls.contains(&control) {
                controls.push(control);
            }
        }
    }
    controls
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
    match &request.sensitive_activity_mode {
        BrowserSensitiveActivityMode::Standard => {}
        BrowserSensitiveActivityMode::Private => reasons.push(format!(
            "sensitive activity mode `{}` requires verified browser-state, credential, and persistence suppression before launch",
            browser_sensitive_activity_mode_wire_name(&request.sensitive_activity_mode)
        )),
        BrowserSensitiveActivityMode::NetworkSuppressed => reasons.push(format!(
            "sensitive activity mode `{}` requires deny-by-default egress before launch",
            browser_sensitive_activity_mode_wire_name(&request.sensitive_activity_mode)
        )),
        BrowserSensitiveActivityMode::Sealed => reasons.push(format!(
            "sensitive activity mode `{}` requires deny-by-default egress plus encrypted artifact sealing before launch",
            browser_sensitive_activity_mode_wire_name(&request.sensitive_activity_mode)
        )),
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
    let guard_plan = browser_admission_guard_plan(&request, &requested_profile_controls);
    let adapter_handoff = browser_adapter_handoff(browser_adapter_launch_request_template(
        "browser-admission-launch-template-v1",
        None,
        &request,
        &guard_plan,
        browser_adapter_contract().required_completion_proofs,
    ));

    BrowserAdmissionResponse {
        decision: BrowserAdmissionDecision::Rejected,
        runnable_browser_sessions: false,
        requested_level: request.requested_level.clone(),
        selected_level: None,
        actor: request.actor,
        sensitivity: request.sensitivity,
        sensitive_activity_mode: request.sensitive_activity_mode,
        target_origins: request.target_origins,
        credential_mode: request.credential_mode,
        artifact_mode: request.artifact_mode,
        requested_controls: request.required_controls,
        requested_profile_controls,
        missing_controls,
        level_satisfies_requested_controls,
        intent_warnings,
        guard_plan,
        adapter_handoff,
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

fn browser_admission_guard_plan(
    request: &BrowserAdmissionRequest,
    requested_profile_controls: &[BrowserSandboxControl],
) -> BrowserAdmissionGuardPlan {
    let profile_requires_egress_boundary = requested_profile_controls
        .contains(&BrowserSandboxControl::EgressPolicy)
        || matches!(&request.sensitivity, BrowserSensitivity::Sensitive)
        || matches!(
            &request.sensitive_activity_mode,
            BrowserSensitiveActivityMode::NetworkSuppressed | BrowserSensitiveActivityMode::Sealed
        );
    BrowserAdmissionGuardPlan {
        network: BrowserNetworkGuardPlan {
            allowed_origins: request.target_origins.clone(),
            deny_private_networks: true,
            deny_localhost: true,
            deny_metadata_endpoints: true,
            require_dns_rebinding_protection: true,
            require_redirect_revalidation: true,
            require_proxy_enforcement: profile_requires_egress_boundary,
            outbound_network_disabled_without_proxy: profile_requires_egress_boundary,
        },
        credentials: BrowserCredentialGuardPlan {
            mode: request.credential_mode.clone(),
            ambient_credentials_allowed: false,
            user_mediation_required: matches!(
                &request.credential_mode,
                BrowserCredentialMode::UserMediated
            ),
            scoped_secret_channel_required: matches!(
                &request.credential_mode,
                BrowserCredentialMode::ScopedSecrets
            ),
        },
        storage: BrowserStorageGuardPlan {
            mode: request.artifact_mode.clone(),
            plaintext_persistence_allowed: false,
            explicit_artifact_allowlist_required: matches!(
                &request.artifact_mode,
                BrowserArtifactMode::ExplicitDownloads | BrowserArtifactMode::SealedArtifacts
            ),
            encryption_required_for_persistence: matches!(
                &request.artifact_mode,
                BrowserArtifactMode::SealedArtifacts
            ),
            teardown_proof_required: requested_profile_controls
                .contains(&BrowserSandboxControl::TeardownProof),
        },
        suppression: browser_suppression_guard_plan(request),
        required_runtime_guards: browser_required_runtime_guards(
            &request.requested_level,
            &request.sensitive_activity_mode,
            requested_profile_controls,
        ),
    }
}

fn browser_suppression_guard_plan(
    request: &BrowserAdmissionRequest,
) -> BrowserSuppressionGuardPlan {
    let private_or_stronger = matches!(
        &request.sensitive_activity_mode,
        BrowserSensitiveActivityMode::Private
            | BrowserSensitiveActivityMode::NetworkSuppressed
            | BrowserSensitiveActivityMode::Sealed
    );
    let network_suppressed_or_stronger = matches!(
        &request.sensitive_activity_mode,
        BrowserSensitiveActivityMode::NetworkSuppressed | BrowserSensitiveActivityMode::Sealed
    );
    let sealed = matches!(
        &request.sensitive_activity_mode,
        BrowserSensitiveActivityMode::Sealed
    );
    let mut required_operator_confirmations = Vec::new();
    if private_or_stronger {
        required_operator_confirmations.push(
            "no ambient browser profile, cookies, extensions, or password store are reused"
                .to_string(),
        );
    }
    if network_suppressed_or_stronger {
        required_operator_confirmations.push(
            "egress is deny-by-default and revalidated after DNS, redirects, proxying, and retries"
                .to_string(),
        );
    }
    if sealed {
        required_operator_confirmations.push(
            "persisted artifacts are explicitly allowlisted and encrypted before plaintext cleanup"
                .to_string(),
        );
    }
    BrowserSuppressionGuardPlan {
        mode: request.sensitive_activity_mode.clone(),
        suppress_ambient_browser_state: private_or_stronger,
        suppress_ambient_credentials: private_or_stronger,
        suppress_unapproved_network: network_suppressed_or_stronger,
        suppress_persistent_artifacts: private_or_stronger,
        downgrade_requires_user_approval: true,
        required_operator_confirmations,
    }
}

fn browser_required_runtime_guards(
    requested_level: &BrowserSandboxLevel,
    sensitive_activity_mode: &BrowserSensitiveActivityMode,
    requested_profile_controls: &[BrowserSandboxControl],
) -> Vec<String> {
    let mut guards = vec![
        "browser launcher bound to the selected sandbox profile".to_string(),
        "production-path admission check before launch".to_string(),
        "teardown proof before reporting session completion".to_string(),
    ];
    if requested_profile_controls.contains(&BrowserSandboxControl::FreshProfile) {
        guards.push("fresh profile directory with no host browser state".to_string());
    }
    if requested_profile_controls.contains(&BrowserSandboxControl::EgressPolicy) {
        guards.push(
            "deny-by-default egress proxy that revalidates DNS, redirects, and final socket targets"
                .to_string(),
        );
    }
    if requested_profile_controls.contains(&BrowserSandboxControl::LocalNetworkBlock) {
        guards.push("loopback, LAN, shared, link-local, and metadata address block".to_string());
    }
    if requested_profile_controls.contains(&BrowserSandboxControl::SealedArtifacts) {
        guards.push("explicit artifact allowlist with configured sealing key".to_string());
    }
    if requested_profile_controls.contains(&BrowserSandboxControl::OsProcessIsolation) {
        guards.push("OS jail or microVM boundary around the browser process".to_string());
    }
    if requested_profile_controls.contains(&BrowserSandboxControl::RemoteWorkerIsolation) {
        guards.push("authenticated remote worker with disposable workspace".to_string());
    }
    if matches!(requested_level, BrowserSandboxLevel::InstrumentedExternal) {
        guards.push("external browser mode must remain unavailable for sensitive work".to_string());
    }
    match sensitive_activity_mode {
        BrowserSensitiveActivityMode::Standard => {}
        BrowserSensitiveActivityMode::Private => {
            guards.push(
                "private activity mode suppresses ambient browser state and persistence"
                    .to_string(),
            );
        }
        BrowserSensitiveActivityMode::NetworkSuppressed => {
            guards.push(
                "network_suppressed activity mode blocks unapproved egress before site interaction"
                    .to_string(),
            );
        }
        BrowserSensitiveActivityMode::Sealed => {
            guards.push(
                "sealed activity mode requires encrypted allowlisted artifacts and plaintext cleanup"
                    .to_string(),
            );
        }
    }
    guards
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

fn validate_browser_adapter_manifest_request(
    request: &BrowserAdapterManifestRequest,
) -> Result<(), String> {
    const MAX_ADAPTER_ID_LEN: usize = 128;
    const MAX_LIST_ITEMS: usize = 64;
    if request.adapter_id.is_empty() || request.adapter_id.trim() != request.adapter_id {
        return Err(
            "browser adapter manifest adapter_id must be non-empty without surrounding whitespace"
                .to_string(),
        );
    }
    if request.adapter_id.len() > MAX_ADAPTER_ID_LEN {
        return Err(format!(
            "browser adapter manifest adapter_id must be at most {MAX_ADAPTER_ID_LEN} bytes"
        ));
    }
    if request.contract_version.is_empty()
        || request.contract_version.trim() != request.contract_version
    {
        return Err(
            "browser adapter manifest contract_version must be non-empty without surrounding whitespace"
                .to_string(),
        );
    }
    if let Some(endpoint) = &request.launch_endpoint {
        validate_browser_adapter_launch_endpoint(endpoint)?;
    }
    if request.supported_levels.len() > MAX_LIST_ITEMS
        || request.supported_controls.len() > MAX_LIST_ITEMS
        || request.guard_fields.len() > MAX_LIST_ITEMS
        || request.completion_proofs.len() > MAX_LIST_ITEMS
    {
        return Err(format!(
            "browser adapter manifest arrays must contain at most {MAX_LIST_ITEMS} entries"
        ));
    }
    validate_non_empty_string_list(&request.guard_fields, "guard_fields")?;
    validate_non_empty_string_list(&request.completion_proofs, "completion_proofs")?;
    Ok(())
}

fn validate_browser_adapter_registration_request(
    request: &BrowserAdapterRegistrationRequest,
) -> Result<(), String> {
    const MAX_SAME_USER_CAPABILITY_LEN: usize = 256;
    if request.same_user_capability.is_empty()
        || request.same_user_capability.trim() != request.same_user_capability
    {
        return Err(
            "browser adapter registration same_user_capability must be non-empty without surrounding whitespace"
                .to_string(),
        );
    }
    if request.same_user_capability.len() > MAX_SAME_USER_CAPABILITY_LEN {
        return Err(format!(
            "browser adapter registration same_user_capability must be at most {MAX_SAME_USER_CAPABILITY_LEN} bytes"
        ));
    }
    validate_browser_adapter_manifest_request(&request.manifest)
}

fn validate_browser_adapter_launch_plan_request(
    request: &BrowserAdapterLaunchPlanRequest,
) -> Result<(), String> {
    const MAX_SAME_USER_CAPABILITY_LEN: usize = 256;
    if request.same_user_capability.is_empty()
        || request.same_user_capability.trim() != request.same_user_capability
    {
        return Err(
            "browser adapter launch plan same_user_capability must be non-empty without surrounding whitespace"
                .to_string(),
        );
    }
    if request.same_user_capability.len() > MAX_SAME_USER_CAPABILITY_LEN {
        return Err(format!(
            "browser adapter launch plan same_user_capability must be at most {MAX_SAME_USER_CAPABILITY_LEN} bytes"
        ));
    }
    validate_browser_admission_request(&request.admission)?;
    validate_browser_adapter_manifest_request(&request.manifest)
}

fn validate_browser_adapter_launch_claim_wire(value: &Value) -> Result<(), String> {
    let object = exact_json_object(value, "browser adapter launch claim", &["launch_request"])?;
    let launch_request = object
        .get("launch_request")
        .ok_or_else(|| "browser adapter launch claim must include launch_request".to_string())?;
    validate_browser_adapter_launch_request_wire(launch_request)
}

fn validate_browser_adapter_launch_request_wire(value: &Value) -> Result<(), String> {
    let object = exact_json_object(
        value,
        "browser adapter launch claim launch_request",
        &[
            "request_id",
            "issued_at",
            "expires_at",
            "max_session_seconds",
            "adapter_id",
            "contract_version",
            "requested_level",
            "actor",
            "sensitivity",
            "sensitive_activity_mode",
            "target_origins",
            "credential_mode",
            "artifact_mode",
            "requested_controls",
            "guard_plan",
            "required_completion_proofs",
            "completion_proof_contract",
            "completion_report_template",
            "same_user_capability_required",
            "endpoint_network_policy_binding_required",
            "replay_protection_required",
            "notes",
        ],
    )?;
    validate_browser_guard_plan_wire(object.get("guard_plan").ok_or_else(|| {
        "browser adapter launch claim launch_request must include guard_plan".to_string()
    })?)?;
    validate_completion_proof_contract_wire(object.get("completion_proof_contract").ok_or_else(
        || {
            "browser adapter launch claim launch_request must include completion_proof_contract"
                .to_string()
        },
    )?)?;
    validate_completion_report_template_wire(
        object.get("completion_report_template").ok_or_else(|| {
            "browser adapter launch claim launch_request must include completion_report_template"
                .to_string()
        })?,
    )?;
    Ok(())
}

fn validate_browser_guard_plan_wire(value: &Value) -> Result<(), String> {
    let object = exact_json_object(
        value,
        "browser adapter launch claim guard_plan",
        &[
            "network",
            "credentials",
            "storage",
            "suppression",
            "required_runtime_guards",
        ],
    )?;
    exact_json_object(
        object.get("network").ok_or_else(|| {
            "browser adapter launch claim guard_plan must include network".to_string()
        })?,
        "browser adapter launch claim guard_plan.network",
        &[
            "allowed_origins",
            "deny_private_networks",
            "deny_localhost",
            "deny_metadata_endpoints",
            "require_dns_rebinding_protection",
            "require_redirect_revalidation",
            "require_proxy_enforcement",
            "outbound_network_disabled_without_proxy",
        ],
    )?;
    exact_json_object(
        object.get("credentials").ok_or_else(|| {
            "browser adapter launch claim guard_plan must include credentials".to_string()
        })?,
        "browser adapter launch claim guard_plan.credentials",
        &[
            "mode",
            "ambient_credentials_allowed",
            "user_mediation_required",
            "scoped_secret_channel_required",
        ],
    )?;
    exact_json_object(
        object.get("storage").ok_or_else(|| {
            "browser adapter launch claim guard_plan must include storage".to_string()
        })?,
        "browser adapter launch claim guard_plan.storage",
        &[
            "mode",
            "plaintext_persistence_allowed",
            "explicit_artifact_allowlist_required",
            "encryption_required_for_persistence",
            "teardown_proof_required",
        ],
    )?;
    exact_json_object(
        object.get("suppression").ok_or_else(|| {
            "browser adapter launch claim guard_plan must include suppression".to_string()
        })?,
        "browser adapter launch claim guard_plan.suppression",
        &[
            "mode",
            "suppress_ambient_browser_state",
            "suppress_ambient_credentials",
            "suppress_unapproved_network",
            "suppress_persistent_artifacts",
            "downgrade_requires_user_approval",
            "required_operator_confirmations",
        ],
    )?;
    Ok(())
}

fn validate_completion_proof_contract_wire(value: &Value) -> Result<(), String> {
    let proofs = value.as_array().ok_or_else(|| {
        "browser adapter launch claim completion_proof_contract must be an array".to_string()
    })?;
    for proof in proofs {
        exact_json_object(
            proof,
            "browser adapter launch claim completion_proof_contract entry",
            &["proof_id", "label", "evidence_field", "required_invariant"],
        )?;
    }
    Ok(())
}

fn validate_completion_report_template_wire(value: &Value) -> Result<(), String> {
    exact_json_object(
        value,
        "browser adapter launch claim completion_report_template",
        &[
            "request_id",
            "adapter_id",
            "contract_version",
            "process_terminated",
            "temporary_profile_removed",
            "plaintext_artifacts_removed",
            "egress_log_sealed_or_discarded",
            "sealed_artifact_handles",
            "proof_ids",
            "notes",
        ],
    )?;
    Ok(())
}

fn exact_json_object<'a>(
    value: &'a Value,
    context: &str,
    required_keys: &[&str],
) -> Result<&'a serde_json::Map<String, Value>, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{context} must be an object"))?;
    for key in object.keys() {
        if !required_keys.contains(&key.as_str()) {
            return Err(format!("{context} does not accept field `{key}`"));
        }
    }
    for key in required_keys {
        if !object.contains_key(*key) {
            return Err(format!("{context} must include `{key}`"));
        }
    }
    Ok(object)
}

fn validate_browser_adapter_launch_claim_request(
    request: &BrowserAdapterLaunchClaimRequest,
) -> Result<(), String> {
    const MAX_ID_LEN: usize = 128;
    const MAX_LIST_ITEMS: usize = 64;
    let launch_request = &request.launch_request;
    validate_non_empty_trimmed(
        &launch_request.request_id,
        "browser adapter launch claim request_id",
    )?;
    if launch_request.request_id.len() > MAX_ID_LEN {
        return Err(format!(
            "browser adapter launch claim request_id must be at most {MAX_ID_LEN} bytes"
        ));
    }
    let Some(adapter_id) = &launch_request.adapter_id else {
        return Err("browser adapter launch claim adapter_id must be present".to_string());
    };
    validate_non_empty_trimmed(adapter_id, "browser adapter launch claim adapter_id")?;
    if adapter_id.len() > MAX_ID_LEN {
        return Err(format!(
            "browser adapter launch claim adapter_id must be at most {MAX_ID_LEN} bytes"
        ));
    }
    validate_non_empty_trimmed(
        &launch_request.contract_version,
        "browser adapter launch claim contract_version",
    )?;
    if launch_request.contract_version.len() > MAX_ID_LEN {
        return Err(format!(
            "browser adapter launch claim contract_version must be at most {MAX_ID_LEN} bytes"
        ));
    }
    if launch_request.required_completion_proofs.len() > MAX_LIST_ITEMS
        || launch_request.notes.len() > MAX_LIST_ITEMS
    {
        return Err(format!(
            "browser adapter launch claim arrays must contain at most {MAX_LIST_ITEMS} entries"
        ));
    }
    DateTime::parse_from_rfc3339(&launch_request.issued_at)
        .map_err(|_| "browser adapter launch claim issued_at must be RFC3339".to_string())?;
    DateTime::parse_from_rfc3339(&launch_request.expires_at)
        .map_err(|_| "browser adapter launch claim expires_at must be RFC3339".to_string())?;
    validate_non_empty_string_list_with_context(
        &launch_request.required_completion_proofs,
        "browser adapter launch claim required_completion_proofs",
    )?;
    Ok(())
}

fn validate_browser_adapter_completion_report_request(
    request: &BrowserAdapterCompletionReport,
) -> Result<(), String> {
    const MAX_ID_LEN: usize = 128;
    const MAX_LIST_ITEMS: usize = 64;
    validate_non_empty_trimmed(
        &request.request_id,
        "browser adapter completion report request_id",
    )?;
    validate_non_empty_trimmed(
        &request.adapter_id,
        "browser adapter completion report adapter_id",
    )?;
    validate_non_empty_trimmed(
        &request.contract_version,
        "browser adapter completion report contract_version",
    )?;
    if request.request_id.len() > MAX_ID_LEN {
        return Err(format!(
            "browser adapter completion report request_id must be at most {MAX_ID_LEN} bytes"
        ));
    }
    if request.adapter_id.len() > MAX_ID_LEN {
        return Err(format!(
            "browser adapter completion report adapter_id must be at most {MAX_ID_LEN} bytes"
        ));
    }
    if request.contract_version.len() > MAX_ID_LEN {
        return Err(format!(
            "browser adapter completion report contract_version must be at most {MAX_ID_LEN} bytes"
        ));
    }
    if request.proof_ids.len() > MAX_LIST_ITEMS
        || request.sealed_artifact_handles.len() > MAX_LIST_ITEMS
        || request.notes.len() > MAX_LIST_ITEMS
    {
        return Err(format!(
            "browser adapter completion report arrays must contain at most {MAX_LIST_ITEMS} entries"
        ));
    }
    validate_non_empty_string_list_with_context(
        &request.proof_ids,
        "browser adapter completion report proof_ids",
    )?;
    validate_non_empty_string_list_with_context(
        &request.sealed_artifact_handles,
        "browser adapter completion report sealed_artifact_handles",
    )?;
    Ok(())
}

fn validate_browser_adapter_capability_issue_request(
    request: &BrowserAdapterCapabilityIssueRequest,
) -> Result<(), String> {
    const MAX_ADAPTER_ID_LEN: usize = 128;
    if let Some(adapter_id) = &request.adapter_id {
        if adapter_id.is_empty() || adapter_id.trim() != adapter_id {
            return Err(
                "browser adapter capability adapter_id must be non-empty without surrounding whitespace"
                    .to_string(),
            );
        }
        if adapter_id.len() > MAX_ADAPTER_ID_LEN {
            return Err(format!(
                "browser adapter capability adapter_id must be at most {MAX_ADAPTER_ID_LEN} bytes"
            ));
        }
    }
    if let Some(ttl_seconds) = request.ttl_seconds
        && (ttl_seconds == 0 || ttl_seconds > BROWSER_ADAPTER_CAPABILITY_TTL_SECONDS)
    {
        return Err(format!(
            "browser adapter capability ttl_seconds must be between 1 and {BROWSER_ADAPTER_CAPABILITY_TTL_SECONDS}"
        ));
    }
    Ok(())
}

fn validate_non_empty_string_list(values: &[String], field: &str) -> Result<(), String> {
    validate_non_empty_string_list_with_context(
        values,
        &format!("browser adapter manifest {field}"),
    )
}

fn validate_non_empty_string_list_with_context(
    values: &[String],
    field: &str,
) -> Result<(), String> {
    for value in values {
        validate_non_empty_trimmed(value, &format!("{field} entries"))?;
    }
    Ok(())
}

fn validate_non_empty_trimmed(value: &str, field: &str) -> Result<(), String> {
    if value.is_empty() || value.trim() != value {
        return Err(format!(
            "{field} must be non-empty without surrounding whitespace"
        ));
    }
    Ok(())
}

fn validate_browser_adapter_launch_endpoint(endpoint: &str) -> Result<(), String> {
    if endpoint.is_empty() || endpoint.trim() != endpoint {
        return Err(
            "browser adapter manifest launch_endpoint must be non-empty without surrounding whitespace"
                .to_string(),
        );
    }
    let url = Url::parse(endpoint)
        .map_err(|error| format!("browser adapter manifest launch_endpoint is invalid: {error}"))?;
    if url.scheme() != "https" {
        return Err("browser adapter manifest launch_endpoint must use https".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(
            "browser adapter manifest launch_endpoint must not contain credentials".to_string(),
        );
    }
    if url.host().is_none() {
        return Err("browser adapter manifest launch_endpoint must include a host".to_string());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(
            "browser adapter manifest launch_endpoint must not contain query or fragment components"
                .to_string(),
        );
    }
    if let Some(host) = url.host() {
        validate_browser_adapter_launch_host(host)?;
    }
    Ok(())
}

fn validate_browser_adapter_launch_host(host: Host<&str>) -> Result<(), String> {
    match host {
        Host::Domain(domain) => {
            let domain = domain.trim_end_matches('.').to_ascii_lowercase();
            if domain == "localhost" || domain.ends_with(".localhost") {
                return Err(
                    "browser adapter manifest launch_endpoint must not target localhost"
                        .to_string(),
                );
            }
        }
        Host::Ipv4(addr) => {
            if ipv4_is_restricted_browser_target(addr) {
                return Err(
                    "browser adapter manifest launch_endpoint must not target local or private IPv4 space"
                        .to_string(),
                );
            }
        }
        Host::Ipv6(addr) => {
            if let Some(mapped) = addr.to_ipv4_mapped()
                && ipv4_is_restricted_browser_target(mapped)
            {
                return Err(
                    "browser adapter manifest launch_endpoint must not target local or private IPv4-mapped IPv6 space"
                        .to_string(),
                );
            }
            if addr.is_loopback()
                || addr.is_unspecified()
                || ipv6_is_unique_local(addr)
                || ipv6_is_unicast_link_local(addr)
            {
                return Err(
                    "browser adapter manifest launch_endpoint must not target local or private IPv6 space"
                        .to_string(),
                );
            }
        }
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

fn browser_sensitive_activity_mode_wire_name(mode: &BrowserSensitiveActivityMode) -> &'static str {
    match mode {
        BrowserSensitiveActivityMode::Standard => "standard",
        BrowserSensitiveActivityMode::Private => "private",
        BrowserSensitiveActivityMode::NetworkSuppressed => "network_suppressed",
        BrowserSensitiveActivityMode::Sealed => "sealed",
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
        openapi_paths::browser_adapter_contract_get,
        openapi_paths::browser_adapter_capability_issue,
        openapi_paths::browser_adapter_register,
        openapi_paths::browser_adapter_launch_plan,
        openapi_paths::browser_adapter_launch_claim,
        openapi_paths::browser_adapter_validate,
        openapi_paths::browser_adapter_completion_validate,
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
        beatbox_core::BrowserAdapterCapabilityIssueRequest,
        beatbox_core::BrowserAdapterCapabilityIssueResponse,
        beatbox_core::BrowserAdapterCompletionReport,
        beatbox_core::BrowserAdapterCompletionProofRequirement,
        beatbox_core::BrowserAdapterCompletionValidationDecision,
        beatbox_core::BrowserAdapterCompletionValidationResponse,
        beatbox_core::BrowserAdapterContract,
        beatbox_core::BrowserAdapterContractResponse,
        beatbox_core::BrowserAdapterConformanceCase,
        beatbox_core::BrowserAdapterConformanceExpectation,
        beatbox_core::BrowserAdapterConformanceProfile,
        beatbox_core::BrowserAdapterLaunchClaimDecision,
        beatbox_core::BrowserAdapterLaunchClaimRequest,
        beatbox_core::BrowserAdapterLaunchClaimResponse,
        beatbox_core::BrowserAdapterLaunchRequest,
        beatbox_core::BrowserAdapterLaunchPlanDecision,
        beatbox_core::BrowserAdapterLaunchPlanRequest,
        beatbox_core::BrowserAdapterLaunchPlanResponse,
        beatbox_core::BrowserAdapterManifestRequest,
        beatbox_core::BrowserAdapterManifestResponse,
        beatbox_core::BrowserAdapterRegistrationDecision,
        beatbox_core::BrowserAdapterRegistrationRequest,
        beatbox_core::BrowserAdapterRegistrationResponse,
        beatbox_core::BrowserAdapterValidationDecision,
        beatbox_core::BrowserSandboxProfile,
        beatbox_core::BrowserSandboxLevel,
        beatbox_core::BrowserSandboxAvailability,
        beatbox_core::BrowserSandboxControl,
        beatbox_core::BrowserCredentialMode,
        beatbox_core::BrowserArtifactMode,
        beatbox_core::BrowserAdmissionRequest,
        beatbox_core::BrowserAdmissionResponse,
        beatbox_core::BrowserAdmissionDecision,
        beatbox_core::BrowserAdmissionGuardPlan,
        beatbox_core::BrowserAdapterHandoff,
        beatbox_core::BrowserNetworkGuardPlan,
        beatbox_core::BrowserCredentialGuardPlan,
        beatbox_core::BrowserStorageGuardPlan,
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
        BrowserAdapterCapabilityIssueRequest, BrowserAdapterCapabilityIssueResponse,
        BrowserAdapterCompletionReport, BrowserAdapterCompletionValidationResponse,
        BrowserAdapterContractResponse, BrowserAdapterLaunchClaimRequest,
        BrowserAdapterLaunchClaimResponse, BrowserAdapterLaunchPlanRequest,
        BrowserAdapterLaunchPlanResponse, BrowserAdapterManifestRequest,
        BrowserAdapterManifestResponse, BrowserAdapterRegistrationRequest,
        BrowserAdapterRegistrationResponse, BrowserAdmissionRequest, BrowserAdmissionResponse,
        BrowserProfilesResponse, CapabilitiesResponse, CreateJobResponse, ErrorResponse,
        ExecuteRequest, ExecutionResult, JobRecord,
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
        operation_id = "getCapabilities",
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
        operation_id = "getBrowserProfiles",
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
        operation_id = "admitBrowserSession",
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
        get,
        path = "/v1/browser/adapter/contract",
        operation_id = "getBrowserAdapterContract",
        tag = "v1",
        responses(
            (status = 200, description = "Fail-closed browser adapter contract and conformance profile discovery", body = BrowserAdapterContractResponse),
            (status = 401, description = "Missing or invalid bearer token", body = ErrorResponse)
        )
    )]
    pub fn browser_adapter_contract_get() {}

    #[utoipa::path(
        post,
        path = "/v1/browser/adapter/capability",
        operation_id = "issueBrowserAdapterCapability",
        tag = "v1",
        request_body = BrowserAdapterCapabilityIssueRequest,
        responses(
            (status = 200, description = "Issue a short-lived one-time browser adapter same-user capability", body = BrowserAdapterCapabilityIssueResponse),
            (status = 400, description = "Invalid capability issuance request", body = ErrorResponse),
            (status = 401, description = "Missing auth or daemon auth is not configured", body = ErrorResponse),
            (status = 429, description = "Live capability quota exhausted", body = ErrorResponse)
        )
    )]
    pub fn browser_adapter_capability_issue() {}

    #[utoipa::path(
        post,
        path = "/v1/browser/adapter/register",
        operation_id = "registerBrowserAdapter",
        tag = "v1",
        request_body = BrowserAdapterRegistrationRequest,
        responses(
            (status = 200, description = "Fail-closed browser adapter registration preflight", body = BrowserAdapterRegistrationResponse),
            (status = 400, description = "Invalid adapter registration request", body = ErrorResponse),
            (status = 401, description = "Missing or invalid bearer token", body = ErrorResponse)
        )
    )]
    pub fn browser_adapter_register() {}

    #[utoipa::path(
        post,
        path = "/v1/browser/adapter/launch/plan",
        operation_id = "planBrowserAdapterLaunch",
        tag = "v1",
        request_body = BrowserAdapterLaunchPlanRequest,
        responses(
            (status = 200, description = "Fail-closed browser adapter launch plan preflight", body = BrowserAdapterLaunchPlanResponse),
            (status = 400, description = "Invalid adapter launch plan request", body = ErrorResponse),
            (status = 401, description = "Missing auth or daemon auth is not configured", body = ErrorResponse)
        )
    )]
    pub fn browser_adapter_launch_plan() {}

    #[utoipa::path(
        post,
        path = "/v1/browser/adapter/launch/claim",
        operation_id = "claimBrowserAdapterLaunch",
        tag = "v1",
        request_body = BrowserAdapterLaunchClaimRequest,
        responses(
            (status = 200, description = "REST-only browser adapter launch request replay claim", body = BrowserAdapterLaunchClaimResponse),
            (status = 400, description = "Invalid adapter launch claim request", body = ErrorResponse),
            (status = 401, description = "Missing or invalid bearer token", body = ErrorResponse)
        )
    )]
    pub fn browser_adapter_launch_claim() {}

    #[utoipa::path(
        post,
        path = "/v1/browser/adapter/validate",
        operation_id = "validateBrowserAdapter",
        tag = "v1",
        request_body = BrowserAdapterManifestRequest,
        responses(
            (status = 200, description = "Fail-closed browser adapter manifest validation", body = BrowserAdapterManifestResponse),
            (status = 400, description = "Invalid adapter manifest", body = ErrorResponse),
            (status = 401, description = "Missing or invalid bearer token", body = ErrorResponse)
        )
    )]
    pub fn browser_adapter_validate() {}

    #[utoipa::path(
        post,
        path = "/v1/browser/adapter/completion/validate",
        operation_id = "validateBrowserAdapterCompletion",
        tag = "v1",
        request_body = BrowserAdapterCompletionReport,
        responses(
            (status = 200, description = "Fail-closed browser adapter completion report validation", body = BrowserAdapterCompletionValidationResponse),
            (status = 400, description = "Invalid adapter completion report", body = ErrorResponse),
            (status = 401, description = "Missing or invalid bearer token", body = ErrorResponse)
        )
    )]
    pub fn browser_adapter_completion_validate() {}

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
        operation_id = "createJob",
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
        operation_id = "getJob",
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
        operation_id = "cancelJob",
        tag = "v1",
        params(("id" = String, Path, description = "Job id")),
        responses(
            (status = 204, description = "Canceled job (or already canceled)"),
            (status = 401, description = "Missing or invalid bearer token", body = ErrorResponse),
            (status = 404, description = "Unknown job", body = ErrorResponse),
            (status = 409, description = "Job already finished and cannot be canceled", body = ErrorResponse)
        )
    )]
    pub fn cancel_job() {}

    #[utoipa::path(
        post,
        path = "/mcp",
        operation_id = "postMcp",
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

async fn mcp_options(headers: HeaderMap) -> Response {
    if !origin_allowed(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let cors_origin = mcp_cors_origin(&headers);
    let mut response = StatusCode::NO_CONTENT.into_response();
    add_mcp_cors_headers(&mut response, cors_origin.as_deref(), true);
    response
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
    let cors_origin = mcp_cors_origin(&headers);
    if state.config.auth.is_required()
        && let Err(error) = state.authorize(&headers)
    {
        return mcp_json_response(
            StatusCode::UNAUTHORIZED,
            json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32001, "message": error.body.message}}),
            cors_origin.as_deref(),
        );
    }
    if let Err(error) = require_json_content_type(&headers) {
        return mcp_json_response(
            StatusCode::BAD_REQUEST,
            json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32600, "message": error.body.message}}),
            cors_origin.as_deref(),
        );
    }

    let body = match to_bytes(body, state.config.max_request_bytes).await {
        Ok(body) => body,
        Err(error) => {
            return mcp_json_response(
                StatusCode::BAD_REQUEST,
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32600, "message": format!("body limit exceeded: {error}")}}),
                cors_origin.as_deref(),
            );
        }
    };

    let message: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(error) => {
            return mcp_json_response(
                StatusCode::BAD_REQUEST,
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {error}")}}),
                cors_origin.as_deref(),
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
        return mcp_json_response(
            StatusCode::UNAUTHORIZED,
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32001, "message": error.body.message}}),
            cors_origin.as_deref(),
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
    mcp_json_response(StatusCode::OK, body, cors_origin.as_deref())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct AetherPaymentContext {
    payment_present: bool,
    payment_hash: Option<String>,
}

impl AetherPaymentContext {
    fn from_headers(headers: &HeaderMap) -> Result<Self, (i64, String)> {
        let payment = optional_header_value(headers, AETHER_PAYMENT_HEADER)?;
        let payment_hash = optional_header_value(headers, AETHER_PAYMENT_HASH_HEADER)?;
        if let Some(payment) = &payment
            && payment.len() > MAX_AETHER_PAYMENT_HEADER_BYTES
        {
            return Err((
                -32602,
                format!(
                    "{AETHER_PAYMENT_HEADER} must be at most {MAX_AETHER_PAYMENT_HEADER_BYTES} bytes"
                ),
            ));
        }
        if let Some(payment_hash) = &payment_hash {
            validate_aether_payment_hash(payment_hash)?;
        }
        if payment.is_some() != payment_hash.is_some() {
            return Err((
                -32602,
                format!(
                    "{AETHER_PAYMENT_HEADER} and {AETHER_PAYMENT_HASH_HEADER} must be supplied together"
                ),
            ));
        }
        Ok(Self {
            payment_present: payment.is_some(),
            payment_hash,
        })
    }

    fn apply_to_tool_result(&self, result: &mut Value) {
        if !self.payment_present {
            return;
        }
        if let Some(object) = result.as_object_mut() {
            object.insert(
                "_meta".to_string(),
                json!({
                    "aether_payment": {
                        "payment_header": AETHER_PAYMENT_HEADER,
                        "payment_header_present": true,
                        "payment_payload_echoed": false,
                        "payment_hash_header": AETHER_PAYMENT_HASH_HEADER,
                        "payment_hash": self.payment_hash,
                    }
                }),
            );
        }
    }
}

fn optional_header_value(
    headers: &HeaderMap,
    name: &'static str,
) -> Result<Option<String>, (i64, String)> {
    let count = headers.get_all(name).iter().count();
    if count > 1 {
        return Err((-32602, format!("{name} must be supplied at most once")));
    }
    let Some(value) = headers.get(name) else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| (-32602, format!("{name} must be visible ASCII")))?;
    if value.is_empty() {
        return Err((-32602, format!("{name} must not be empty")));
    }
    Ok(Some(value.to_string()))
}

fn validate_aether_payment_hash(hash: &str) -> Result<(), (i64, String)> {
    let digest = hash.strip_prefix("0x").ok_or((
        -32602,
        format!("{AETHER_PAYMENT_HASH_HEADER} must be a 0x-prefixed 32-byte hex digest"),
    ))?;
    if digest.len() != 64 || !digest.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return Err((
            -32602,
            format!("{AETHER_PAYMENT_HASH_HEADER} must be a 0x-prefixed 32-byte hex digest"),
        ));
    }
    Ok(())
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
            "description": "Return beatbox browser sandbox profile discovery metadata and the planned adapter handoff contract for Tempo-style integrations.",
            "inputSchema": {"type": "object", "additionalProperties": false}
        },
        {
            "name": "get_browser_adapter_contract",
            "description": "Return beatbox's planned browser adapter contract and conformance_profile for Tempo-style integrations without trusting or launching an adapter.",
            "inputSchema": {"type": "object", "additionalProperties": false}
        },
        {
            "name": "register_browser_adapter",
            "description": "Submit Tempo-style browser adapter manifest metadata through MCP without bearer capabilities. Capability-bound registration is REST-only because same-user capabilities must stay out of model-visible transcripts.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "required": ["actor", "sensitivity", "manifest"],
                "properties": {
                    "actor": {"type": "string", "enum": ["agent", "human"]},
                    "sensitivity": {"type": "string", "enum": ["public", "sensitive"]},
                    "manifest": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": [
                            "adapter_id",
                            "contract_version",
                            "launch_endpoint",
                            "supported_levels",
                            "supported_controls",
                            "guard_fields",
                            "completion_proofs"
                        ],
                        "properties": {
                            "adapter_id": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": 128,
                                "description": "Stable adapter identifier with no surrounding whitespace."
                            },
                            "contract_version": {
                                "type": "string",
                                "minLength": 1,
                                "description": "Browser adapter contract version with no surrounding whitespace."
                            },
                            "launch_endpoint": {
                                "type": ["string", "null"],
                                "minLength": 1,
                                "description": "Optional HTTPS adapter endpoint. Rejects credentials, query/fragment components, localhost, literal local/private IPs, and empty or whitespace-padded values. DNS, proxy, redirect, and retry binding are not implemented."
                            },
                            "supported_levels": {
                                "type": "array",
                                "maxItems": 64,
                                "items": {
                                    "type": "string",
                                    "enum": [
                                        "instrumented_external",
                                        "ephemeral_profile",
                                        "network_suppressed",
                                        "sealed_state",
                                        "os_isolated",
                                        "remote_isolated"
                                    ]
                                }
                            },
                            "supported_controls": {
                                "type": "array",
                                "maxItems": 64,
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
                            "guard_fields": {
                                "type": "array",
                                "maxItems": 64,
                                "items": {
                                    "type": "string",
                                    "minLength": 1,
                                    "description": "Required guard_plan field name with no surrounding whitespace."
                                }
                            },
                            "completion_proofs": {
                                "type": "array",
                                "maxItems": 64,
                                "items": {
                                    "type": "string",
                                    "minLength": 1,
                                    "description": "Required completion proof label with no surrounding whitespace."
                                }
                            }
                        }
                    }
                }
            }
        },
        {
            "name": "admit_browser_session",
            "description": "Return a fail-closed browser sandbox admission decision, guard plan, and non-launchable adapter handoff for a requested actor, sensitivity, and sandbox level.",
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
                    "sensitive_activity_mode": {
                        "type": "string",
                        "description": "Requested privacy/suppression posture for sensitive browser activity. It is part of the fail-closed handoff contract and does not make the current daemon runnable.",
                        "enum": ["standard", "private", "network_suppressed", "sealed"]
                    },
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
        },
        {
            "name": "validate_browser_adapter",
            "description": "Validate a proposed browser adapter manifest against beatbox's planned Tempo handoff contract without trusting or launching the adapter. Responses include a conformance_profile with canonical fail-closed cases for REST/MCP adapter compatibility tests.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "required": [
                    "adapter_id",
                    "contract_version",
                    "launch_endpoint",
                    "supported_levels",
                    "supported_controls",
                    "guard_fields",
                    "completion_proofs"
                ],
                "properties": {
                    "adapter_id": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": 128,
                        "description": "Stable adapter identifier with no surrounding whitespace."
                    },
                    "contract_version": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": 128,
                        "description": "Browser adapter contract version with no surrounding whitespace."
                    },
                    "launch_endpoint": {
                        "type": ["string", "null"],
                        "minLength": 1,
                        "description": "HTTPS endpoint string a future trusted request builder might call. Credentials, queries, fragments, localhost names, and literal local/private IP addresses are rejected; DNS, proxy, redirect, and retry binding is not implemented by this validator."
                    },
                    "supported_levels": {
                        "type": "array",
                        "maxItems": 64,
                        "items": {
                            "type": "string",
                            "enum": [
                                "instrumented_external",
                                "ephemeral_profile",
                                "network_suppressed",
                                "sealed_state",
                                "os_isolated",
                                "remote_isolated"
                            ]
                        }
                    },
                    "supported_controls": {
                        "type": "array",
                        "maxItems": 64,
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
                    "guard_fields": {
                        "type": "array",
                        "maxItems": 64,
                        "items": {
                            "type": "string",
                            "minLength": 1,
                            "description": "Required guard_plan field name with no surrounding whitespace."
                        }
                    },
                    "completion_proofs": {
                        "type": "array",
                        "maxItems": 64,
                        "items": {
                            "type": "string",
                            "minLength": 1,
                            "description": "Required completion proof label with no surrounding whitespace."
                        }
                    }
                }
            }
        },
        {
            "name": "validate_browser_adapter_completion",
            "description": "Validate a browser adapter completion report against beatbox's typed proof contract without trusting the report or marking any browser session complete.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "required": [
                    "request_id",
                    "adapter_id",
                    "contract_version",
                    "process_terminated",
                    "temporary_profile_removed",
                    "plaintext_artifacts_removed",
                    "egress_log_sealed_or_discarded",
                    "sealed_artifact_handles",
                    "proof_ids",
                    "notes"
                ],
                "properties": {
                    "request_id": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": 128,
                        "description": "Server-issued launch request id with no surrounding whitespace."
                    },
                    "adapter_id": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": 128,
                        "description": "Adapter id from the launch request with no surrounding whitespace."
                    },
                    "contract_version": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": 128,
                        "description": "Browser adapter contract version with no surrounding whitespace."
                    },
                    "process_terminated": {"type": "boolean"},
                    "temporary_profile_removed": {"type": "boolean"},
                    "plaintext_artifacts_removed": {"type": "boolean"},
                    "egress_log_sealed_or_discarded": {"type": "boolean"},
                    "sealed_artifact_handles": {
                        "type": "array",
                        "maxItems": 64,
                        "items": {
                            "type": "string",
                            "minLength": 1,
                            "description": "Opaque storage handle, never raw browser state or secrets."
                        }
                    },
                    "proof_ids": {
                        "type": "array",
                        "maxItems": 64,
                        "items": {
                            "type": "string",
                            "minLength": 1,
                            "description": "Machine proof id from completion_proof_contract."
                        }
                    },
                    "notes": {
                        "type": "array",
                        "maxItems": 64,
                        "items": {"type": "string"}
                    }
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
    let payment_context = AetherPaymentContext::from_headers(headers)?;
    let name = params["name"]
        .as_str()
        .ok_or((-32602, "tools/call requires params.name".to_string()))?;
    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

    let mut result = match name {
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
        "get_browser_adapter_contract" => {
            mcp_tool_arguments(&arguments, "get_browser_adapter_contract", &[])?;
            let contract =
                serde_json::to_value(browser_adapter_contract_response()).map_err(|error| {
                    (
                        -32603,
                        format!("failed to serialize browser adapter contract: {error}"),
                    )
                })?;
            Ok(json!({
                "content": [{"type": "text", "text": "beatbox browser adapter contract"}],
                "structuredContent": contract,
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
        "register_browser_adapter" => {
            let request = mcp_browser_adapter_registration_request(&arguments)?;
            let registration = browser_adapter_mcp_registration_response(request);
            let registration = serde_json::to_value(registration).map_err(|error| {
                (
                    -32603,
                    format!("failed to serialize browser adapter registration: {error}"),
                )
            })?;
            Ok(json!({
                "content": [{"type": "text", "text": "beatbox browser adapter registration preflight"}],
                "structuredContent": registration,
                "isError": true,
            }))
        }
        "validate_browser_adapter" => {
            let request = mcp_browser_adapter_manifest_request(&arguments)?;
            let validation = browser_adapter_manifest_response(request);
            let validation = serde_json::to_value(validation).map_err(|error| {
                (
                    -32603,
                    format!("failed to serialize browser adapter validation: {error}"),
                )
            })?;
            Ok(json!({
                "content": [{"type": "text", "text": "beatbox browser adapter validation"}],
                "structuredContent": validation,
                "isError": true,
            }))
        }
        "validate_browser_adapter_completion" => {
            let request = mcp_browser_adapter_completion_report(&arguments)?;
            let validation = browser_adapter_completion_validation_response(None, request);
            let validation = serde_json::to_value(validation).map_err(|error| {
                (
                    -32603,
                    format!("failed to serialize browser adapter completion validation: {error}"),
                )
            })?;
            Ok(json!({
                "content": [{"type": "text", "text": "beatbox browser adapter completion validation"}],
                "structuredContent": validation,
                "isError": true,
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
    }?;
    payment_context.apply_to_tool_result(&mut result);
    Ok(result)
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
            "sensitive_activity_mode",
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
        sensitive_activity_mode: mcp_browser_sensitive_activity_mode_arg(
            arguments,
            "sensitive_activity_mode",
            "admit_browser_session",
        )?
        .unwrap_or_default(),
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

fn mcp_browser_adapter_manifest_request(
    arguments: &Value,
) -> Result<BrowserAdapterManifestRequest, (i64, String)> {
    let arguments = mcp_tool_arguments(
        arguments,
        "validate_browser_adapter",
        &[
            "adapter_id",
            "contract_version",
            "launch_endpoint",
            "supported_levels",
            "supported_controls",
            "guard_fields",
            "completion_proofs",
        ],
    )?;
    let request: BrowserAdapterManifestRequest =
        serde_json::from_value(Value::Object(arguments.clone())).map_err(|error| {
            (
                -32602,
                format!("validate_browser_adapter arguments are invalid: {error}"),
            )
        })?;
    validate_browser_adapter_manifest_request(&request).map_err(|message| (-32602, message))?;
    Ok(request)
}

fn mcp_browser_adapter_completion_report(
    arguments: &Value,
) -> Result<BrowserAdapterCompletionReport, (i64, String)> {
    let arguments = mcp_tool_arguments(
        arguments,
        "validate_browser_adapter_completion",
        &[
            "request_id",
            "adapter_id",
            "contract_version",
            "process_terminated",
            "temporary_profile_removed",
            "plaintext_artifacts_removed",
            "egress_log_sealed_or_discarded",
            "sealed_artifact_handles",
            "proof_ids",
            "notes",
        ],
    )?;
    let request: BrowserAdapterCompletionReport =
        serde_json::from_value(Value::Object(arguments.clone())).map_err(|error| {
            (
                -32602,
                format!("validate_browser_adapter_completion arguments are invalid: {error}"),
            )
        })?;
    validate_browser_adapter_completion_report_request(&request)
        .map_err(|message| (-32602, message))?;
    Ok(request)
}

fn mcp_browser_adapter_registration_request(
    arguments: &Value,
) -> Result<McpBrowserAdapterRegistrationRequest, (i64, String)> {
    let arguments = mcp_tool_arguments(
        arguments,
        "register_browser_adapter",
        &["actor", "sensitivity", "manifest"],
    )?;
    let request: McpBrowserAdapterRegistrationRequest =
        serde_json::from_value(Value::Object(arguments.clone())).map_err(|error| {
            (
                -32602,
                format!("register_browser_adapter arguments are invalid: {error}"),
            )
        })?;
    validate_browser_adapter_manifest_request(&request.manifest)
        .map_err(|message| (-32602, message))?;
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

fn mcp_browser_sensitive_activity_mode_arg(
    arguments: &serde_json::Map<String, Value>,
    key: &'static str,
    tool: &'static str,
) -> Result<Option<BrowserSensitiveActivityMode>, (i64, String)> {
    arguments
        .get(key)
        .map(|_| match mcp_string_arg(arguments, key, tool)?.as_str() {
            "standard" => Ok(BrowserSensitiveActivityMode::Standard),
            "private" => Ok(BrowserSensitiveActivityMode::Private),
            "network_suppressed" => Ok(BrowserSensitiveActivityMode::NetworkSuppressed),
            "sealed" => Ok(BrowserSensitiveActivityMode::Sealed),
            other => Err((
                -32602,
                format!(
                    "{tool} argument `{key}` has unsupported sensitive activity mode `{other}`"
                ),
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

fn mcp_json_response(status: StatusCode, body: Value, cors_origin: Option<&str>) -> Response {
    let mut response = json_response(status, body);
    add_mcp_cors_headers(&mut response, cors_origin, false);
    response
}

fn mcp_cors_origin(headers: &HeaderMap) -> Option<String> {
    headers
        .get("origin")
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string)
}

fn add_mcp_cors_headers(response: &mut Response, cors_origin: Option<&str>, preflight: bool) {
    let headers = response.headers_mut();
    headers.insert(
        "access-control-expose-headers",
        HeaderValue::from_static(MCP_ACCESS_CONTROL_EXPOSE_HEADERS),
    );
    if preflight {
        headers.insert(
            "access-control-allow-methods",
            HeaderValue::from_static(MCP_ACCESS_CONTROL_ALLOW_METHODS),
        );
        headers.insert(
            "access-control-allow-headers",
            HeaderValue::from_static(MCP_ACCESS_CONTROL_ALLOW_HEADERS),
        );
    }
    if let Some(origin) = cors_origin.and_then(|origin| HeaderValue::from_str(origin).ok()) {
        headers.insert("access-control-allow-origin", origin);
        headers.insert("vary", HeaderValue::from_static("origin"));
    }
}
