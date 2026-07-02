mod jobs;

use std::path::Path;
use std::time::Instant;

use axum::body::Body;
use axum::extract::{Path as AxumPath, State};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
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
use utoipa::OpenApi;

pub const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
pub const DEFAULT_SYNC_WALL_MS: u64 = 60_000;

#[derive(Clone)]
pub struct ServerConfig {
    pub auth: AuthMode,
    pub engine: BeatboxEngine,
    pub jobs: JobStore,
    pub sync_wall_ms: u64,
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
    started: Instant,
    config: ServerConfig,
}

pub fn router(config: ServerConfig) -> Router {
    let state = AppState {
        started: Instant::now(),
        config,
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
    Ok(Json(capabilities_json()))
}

async fn execute(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ExecuteRequest>,
) -> Result<Json<ExecutionResult>, ApiError> {
    state.authorize(&headers)?;
    if request.policy.limits.wall_ms > state.config.sync_wall_ms {
        return Err(ApiError::unprocessable_body(ErrorBody::new(
            "sync_limit_exceeded",
            format!(
                "policy.limits.wall_ms={} exceeds synchronous ceiling {}; submit to /v1/jobs",
                request.policy.limits.wall_ms, state.config.sync_wall_ms
            ),
        )));
    }
    state
        .config
        .engine
        .execute(request)
        .map(Json)
        .map_err(ApiError::unprocessable)
}

async fn create_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ExecuteRequest>,
) -> Result<(StatusCode, Json<CreateJobResponse>), ApiError> {
    state.authorize(&headers)?;
    let job_id = state
        .config
        .jobs
        .create(&request)
        .map_err(ApiError::job_store)?;
    spawn_job(state, job_id.clone(), request);
    Ok((StatusCode::ACCEPTED, Json(CreateJobResponse { job_id })))
}

async fn get_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<JobRecord>, ApiError> {
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
    AxumPath(id): AxumPath<String>,
) -> Result<StatusCode, ApiError> {
    state.authorize(&headers)?;
    let exists = state.config.jobs.cancel(&id).map_err(ApiError::job_store)?;
    if exists {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found(format!("unknown job: {id}")))
    }
}

fn spawn_job(state: AppState, job_id: String, request: ExecuteRequest) {
    tokio::spawn(async move {
        if let Err(error) = state.config.jobs.mark_running(&job_id) {
            tracing::warn!(%job_id, %error, "failed to mark job running");
            return;
        }
        let engine = state.config.engine.clone();
        let result = tokio::task::spawn_blocking(move || engine.execute(request)).await;
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
                let expected = format!("Bearer {token}");
                let actual = headers
                    .get(AUTHORIZATION)
                    .and_then(|value| value.to_str().ok());
                if actual
                    .is_some_and(|actual| constant_time_eq(actual.as_bytes(), expected.as_bytes()))
                {
                    Ok(())
                } else {
                    Err(ApiError::unauthorized("missing or invalid bearer token"))
                }
            }
        }
    }
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
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            body: ErrorBody::new("job_store", error.to_string()),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(ErrorResponse { error: self.body })).into_response()
    }
}

fn capabilities_json() -> Value {
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
            "sync_wall_ms": DEFAULT_SYNC_WALL_MS,
            "default_wall_ms": Policy::default().limits.wall_ms,
            "default_memory_bytes": Policy::default().limits.memory_bytes,
            "default_output_bytes": Policy::default().limits.output_bytes
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
    let Some(authority) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    let authority = authority.split('/').next().unwrap_or(authority);
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = if let Some(rest) = authority.strip_prefix('[') {
        let Some((host, _)) = rest.split_once(']') else {
            return false;
        };
        host
    } else {
        authority.split(':').next().unwrap_or(authority)
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
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

async fn mcp_post(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    if !origin_allowed(&headers) {
        return json_response(
            StatusCode::FORBIDDEN,
            json!({"jsonrpc": "2.0", "error": {"code": -32600, "message": "origin not allowed"}}),
        );
    }

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
                "properties": {
                    "wat": {"type": "string"},
                    "wasm_base64": {"type": "string"},
                    "input": {},
                    "timeout_ms": {"type": "integer"},
                    "memory_bytes": {"type": "integer"},
                    "fuel": {"type": "integer"}
                }
            }
        },
        {"name": "run_python", "description": "Planned Python sandbox lane.", "inputSchema": {"type": "object"}},
        {"name": "run_javascript", "description": "Planned JavaScript sandbox lane.", "inputSchema": {"type": "object"}},
        {"name": "get_capabilities", "description": "Return beatbox lane availability.", "inputSchema": {"type": "object"}}
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
        "get_capabilities" => Ok(json!({
            "content": [{"type": "text", "text": capabilities_json().to_string()}],
            "isError": false,
        })),
        "run_wasm" => {
            let request = mcp_run_wasm_request(&arguments)?;
            tool_result(state.config.engine.execute(request))
        }
        "run_python" => {
            let request = ExecuteRequest {
                lane: Lane::PythonWasi,
                source: Source::Inline {
                    code: arguments["code"].as_str().unwrap_or_default().to_string(),
                },
                entrypoint: None,
                input: arguments.get("input").cloned().unwrap_or(Value::Null),
                stdin: String::new(),
                policy: Policy::default(),
                idempotency_key: None,
            };
            tool_result(state.config.engine.execute(request))
        }
        "run_javascript" => {
            let request = ExecuteRequest {
                lane: Lane::JsWasm,
                source: Source::Inline {
                    code: arguments["code"].as_str().unwrap_or_default().to_string(),
                },
                entrypoint: None,
                input: arguments.get("input").cloned().unwrap_or(Value::Null),
                stdin: String::new(),
                policy: Policy::default(),
                idempotency_key: None,
            };
            tool_result(state.config.engine.execute(request))
        }
        other => Err((-32602, format!("unknown tool: {other}"))),
    }
}

fn mcp_run_wasm_request(arguments: &Value) -> Result<ExecuteRequest, (i64, String)> {
    let source = if let Some(wat) = arguments.get("wat").and_then(Value::as_str) {
        Source::WasmWat {
            text: wat.to_string(),
        }
    } else if let Some(bytes) = arguments.get("wasm_base64").and_then(Value::as_str) {
        Source::WasmBytesBase64 {
            bytes: bytes.to_string(),
        }
    } else {
        return Err((
            -32602,
            "run_wasm requires arguments.wat or arguments.wasm_base64".to_string(),
        ));
    };

    let mut policy = Policy::default();
    if let Some(timeout_ms) = arguments.get("timeout_ms").and_then(Value::as_u64) {
        policy.limits.wall_ms = timeout_ms;
    }
    if let Some(memory_bytes) = arguments.get("memory_bytes").and_then(Value::as_u64) {
        policy.limits.memory_bytes = memory_bytes;
    }
    if let Some(fuel) = arguments.get("fuel").and_then(Value::as_u64) {
        policy.limits.fuel = Some(fuel);
    }

    Ok(ExecuteRequest {
        lane: Lane::Wasm,
        source,
        entrypoint: arguments
            .get("entrypoint")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        input: arguments.get("input").cloned().unwrap_or(Value::Null),
        stdin: String::new(),
        policy,
        idempotency_key: None,
    })
}

fn tool_result(result: Result<ExecutionResult, EngineError>) -> Result<Value, (i64, String)> {
    match result {
        Ok(result) => {
            let text = serde_json::to_string(&result)
                .map_err(|error| (-32000, format!("failed to encode result: {error}")))?;
            Ok(json!({
                "content": [{"type": "text", "text": text}],
                "isError": false,
            }))
        }
        Err(error) => Ok(json!({
            "content": [{"type": "text", "text": error.error_body().message}],
            "isError": true,
        })),
    }
}

fn json_response(status: StatusCode, body: Value) -> Response {
    (
        status,
        [(CONTENT_TYPE, "application/json")],
        Body::from(body.to_string()),
    )
        .into_response()
}
