use axum::body::{Body, to_bytes};
use axum::http::header::ORIGIN;
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode};
use beatbox_core::{
    CreateJobResponse, ErrorResponse, ExecuteRequest, ExecutionStatus, JobRecord, JobStatus, Lane,
    Policy, Source,
};
use beatbox_engine::BeatboxEngine;
use beatbox_server::{
    AuthMode, DEFAULT_JOB_WALL_MS, DEFAULT_SYNC_WALL_MS, JobStore, ServerConfig, origin_allowed,
    router,
};
use serde_json::json;
use tower::ServiceExt;

#[tokio::test]
async fn v1_execute_runs_wasm() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = add_one_request(41);
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/execute")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request)?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let result: beatbox_core::ExecutionResult = serde_json::from_slice(&body)?;
    assert_eq!(result.status, ExecutionStatus::Ok);
    assert_eq!(result.value, json!(42));
    Ok(())
}

#[tokio::test]
async fn v1_execute_rejects_over_sync_ceiling() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let mut request = add_one_request(41);
    request.policy.limits.wall_ms = DEFAULT_SYNC_WALL_MS + 1;
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/execute")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request)?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let error: ErrorResponse = serde_json::from_slice(&body)?;
    assert_eq!(error.error.code, "sync_limit_exceeded");
    Ok(())
}

#[tokio::test]
async fn v1_execute_rejects_daemon_local_file_sources() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let mut request = add_one_request(41);
    request.source = Source::WasmFile {
        path: std::path::PathBuf::from("/etc/passwd"),
    };
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/execute")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request)?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let error: ErrorResponse = serde_json::from_slice(&body)?;
    assert_eq!(error.error.code, "host_file_source_denied");
    Ok(())
}

#[tokio::test]
async fn v1_execute_rejects_simple_text_plain_json_posts() -> Result<(), Box<dyn std::error::Error>>
{
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = add_one_request(41);
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/execute")
                .header("content-type", "text/plain")
                .body(Body::from(serde_json::to_vec(&request)?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let error: ErrorResponse = serde_json::from_slice(&body)?;
    assert_eq!(error.error.code, "unsupported_media_type");
    Ok(())
}

#[tokio::test]
async fn v1_execute_rejects_missing_content_type() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = add_one_request(41);
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/execute")
                .body(Body::from(serde_json::to_vec(&request)?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let error: ErrorResponse = serde_json::from_slice(&body)?;
    assert_eq!(error.error.code, "unsupported_media_type");
    Ok(())
}

#[tokio::test]
async fn v1_execute_rejects_when_sync_concurrency_cap_is_exhausted()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?).with_max_concurrent_sync(0));
    let request = add_one_request(41);
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/execute")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request)?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let error: ErrorResponse = serde_json::from_slice(&body)?;
    assert_eq!(error.error.code, "sync_concurrency_exceeded");
    Ok(())
}

#[tokio::test]
async fn auth_required_rejects_execute_before_json_parse() -> Result<(), Box<dyn std::error::Error>>
{
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::Required {
        token: "secret".to_string(),
    };
    let app = router(config);
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/execute")
                .header("content-type", "application/json")
                .body(Body::from("{not json"))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn jobs_complete_and_persist_to_sqlite() -> Result<(), Box<dyn std::error::Error>> {
    let db_path =
        std::env::temp_dir().join(format!("beatbox-jobs-{}.sqlite3", uuid::Uuid::new_v4()));
    let store = JobStore::open(&db_path)?;
    let app = router(ServerConfig::new(BeatboxEngine::new()?).with_job_store(store));
    let request = add_one_request(41);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/jobs")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request)?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let created: CreateJobResponse = serde_json::from_slice(&body)?;

    let mut completed = None;
    for _ in 0..30 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/v1/jobs/{}", created.job_id))
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await?;
        let job: JobRecord = serde_json::from_slice(&body)?;
        if job.status == JobStatus::Succeeded {
            completed = Some(job);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let job = completed.ok_or("job did not complete")?;
    assert_eq!(
        job.result.as_ref().map(|result| &result.value),
        Some(&json!(42))
    );

    let reopened = JobStore::open(&db_path)?;
    let persisted = reopened
        .get(&created.job_id)?
        .ok_or("missing persisted job")?;
    assert_eq!(persisted.status, JobStatus::Succeeded);
    assert_eq!(
        persisted.result.as_ref().map(|result| &result.value),
        Some(&json!(42))
    );
    std::fs::remove_file(db_path).ok();
    Ok(())
}

#[tokio::test]
async fn jobs_dedupe_idempotency_keys() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let mut request = add_one_request(41);
    request.idempotency_key = Some("retry-key-1".to_string());
    let body = serde_json::to_vec(&request)?;

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/jobs")
                .header("content-type", "application/json")
                .body(Body::from(body.clone()))?,
        )
        .await?;
    assert_eq!(first.status(), StatusCode::ACCEPTED);
    let first: CreateJobResponse =
        serde_json::from_slice(&to_bytes(first.into_body(), usize::MAX).await?)?;

    let second = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/jobs")
                .header("content-type", "application/json")
                .body(Body::from(body))?,
        )
        .await?;
    assert_eq!(second.status(), StatusCode::ACCEPTED);
    let second: CreateJobResponse =
        serde_json::from_slice(&to_bytes(second.into_body(), usize::MAX).await?)?;

    assert_eq!(first.job_id, second.job_id);
    Ok(())
}

#[tokio::test]
async fn jobs_reject_idempotency_key_payload_conflicts() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let mut first_request = add_one_request(41);
    first_request.idempotency_key = Some("retry-key-conflict".to_string());
    let mut second_request = add_one_request(42);
    second_request.idempotency_key = first_request.idempotency_key.clone();

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/jobs")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&first_request)?))?,
        )
        .await?;
    assert_eq!(first.status(), StatusCode::ACCEPTED);

    let second = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/jobs")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&second_request)?))?,
        )
        .await?;
    assert_eq!(second.status(), StatusCode::CONFLICT);
    let body = to_bytes(second.into_body(), usize::MAX).await?;
    let error: ErrorResponse = serde_json::from_slice(&body)?;
    assert_eq!(error.error.code, "idempotency_conflict");
    Ok(())
}

#[tokio::test]
async fn jobs_reject_daemon_local_file_sources() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let mut request = add_one_request(41);
    request.source = Source::WasmFile {
        path: std::path::PathBuf::from("/etc/passwd"),
    };
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/jobs")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request)?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let error: ErrorResponse = serde_json::from_slice(&body)?;
    assert_eq!(error.error.code, "host_file_source_denied");
    Ok(())
}

#[tokio::test]
async fn jobs_reject_simple_text_plain_json_posts() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = add_one_request(41);
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/jobs")
                .header("content-type", "text/plain")
                .body(Body::from(serde_json::to_vec(&request)?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let error: ErrorResponse = serde_json::from_slice(&body)?;
    assert_eq!(error.error.code, "unsupported_media_type");
    Ok(())
}

#[tokio::test]
async fn jobs_reject_over_daemon_wall_ceiling() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let mut request = add_one_request(41);
    request.policy.limits.wall_ms = DEFAULT_JOB_WALL_MS + 1;
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/jobs")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request)?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let error: ErrorResponse = serde_json::from_slice(&body)?;
    assert_eq!(error.error.code, "job_limit_exceeded");
    Ok(())
}

#[tokio::test]
async fn jobs_reject_when_concurrency_cap_is_exhausted() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?).with_max_concurrent_jobs(0));
    let request = add_one_request(41);
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/jobs")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request)?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let error: ErrorResponse = serde_json::from_slice(&body)?;
    assert_eq!(error.error.code, "job_concurrency_exceeded");
    Ok(())
}

#[tokio::test]
async fn jobs_can_be_canceled() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = add_one_request(41);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/jobs")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request)?))?,
        )
        .await?;
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let created: CreateJobResponse = serde_json::from_slice(&body)?;
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(format!("/v1/jobs/{}", created.job_id))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    Ok(())
}

#[tokio::test]
async fn auth_required_rejects_keyless_requests() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::Required {
        token: "secret".to_string(),
    };
    let app = router(config);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/capabilities")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/capabilities")
                .header("authorization", "Bearer secret")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn auth_required_rejects_mcp_tools_list_without_key() -> Result<(), Box<dyn std::error::Error>>
{
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::Required {
        token: "secret".to_string(),
    };
    let app = router(config);
    let request = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header("content-type", "application/json")
                .body(Body::from(request.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header("content-type", "application/json")
                .header("authorization", "Bearer secret")
                .body(Body::from(request.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let value: serde_json::Value = serde_json::from_slice(&body)?;
    assert!(value["result"]["tools"].to_string().contains("run_wasm"));
    Ok(())
}

#[tokio::test]
async fn auth_required_rejects_mcp_before_json_parse() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::Required {
        token: "secret".to_string(),
    };
    let app = router(config);
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header("content-type", "application/json")
                .body(Body::from("{not json"))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn openapi_lists_jobs_surface() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/openapi.json")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let value: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(value["openapi"], "3.1.0");
    assert!(
        value["paths"]
            .as_object()
            .is_some_and(|paths| paths.contains_key("/v1/jobs"))
    );
    Ok(())
}

#[test]
fn origin_validation_rejects_localhost_prefix_bypass() {
    let mut headers = HeaderMap::new();
    headers.insert(ORIGIN, HeaderValue::from_static("http://localhost:3000"));
    assert!(origin_allowed(&headers));

    headers.insert(ORIGIN, HeaderValue::from_static("http://127.0.0.1:3000"));
    assert!(origin_allowed(&headers));

    headers.insert(ORIGIN, HeaderValue::from_static("http://[::1]:3000"));
    assert!(origin_allowed(&headers));

    headers.insert(
        ORIGIN,
        HeaderValue::from_static("http://localhost.evil.com"),
    );
    assert!(!origin_allowed(&headers));

    headers.insert(
        ORIGIN,
        HeaderValue::from_static("http://127.0.0.1.evil.com"),
    );
    assert!(!origin_allowed(&headers));

    headers.insert(ORIGIN, HeaderValue::from_static("http://localhost/path"));
    assert!(!origin_allowed(&headers));

    headers.insert(
        ORIGIN,
        HeaderValue::from_static("http://localhost@evil.com"),
    );
    assert!(!origin_allowed(&headers));

    headers.insert(ORIGIN, HeaderValue::from_static("http://[::1].evil.com"));
    assert!(!origin_allowed(&headers));
}

#[tokio::test]
async fn mcp_lists_tools() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header("content-type", "application/json")
                .body(Body::from(request.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let value: serde_json::Value = serde_json::from_slice(&body)?;
    let tools = &value["result"]["tools"];
    assert!(tools.to_string().contains("run_wasm"));
    assert!(tools.to_string().contains("get_capabilities"));
    Ok(())
}

#[tokio::test]
async fn mcp_run_wasm_rejects_over_sync_ceiling() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "run_wasm",
            "arguments": {
                "wat": add_one_wat(),
                "input": {"n": 41},
                "timeout_ms": DEFAULT_SYNC_WALL_MS + 1
            }
        }
    });
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header("content-type", "application/json")
                .body(Body::from(request.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let value: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(value["error"]["code"], -32602);
    assert!(
        value["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("sync_limit_exceeded"))
    );
    Ok(())
}

fn add_one_request(n: i64) -> ExecuteRequest {
    ExecuteRequest {
        lane: Lane::Wasm,
        source: Source::WasmWat {
            text: add_one_wat().to_string(),
        },
        entrypoint: None,
        input: json!({"n": n}),
        stdin: String::new(),
        policy: Policy::default(),
        idempotency_key: None,
    }
}

fn add_one_wat() -> &'static str {
    r#"
            (module
              (func (export "run") (param i64) (result i64)
                local.get 0
                i64.const 1
                i64.add))
            "#
}
