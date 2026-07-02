use axum::body::{Body, to_bytes};
use axum::http::header::ORIGIN;
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode};
use beatbox_core::{
    CreateJobResponse, ErrorResponse, ExecuteRequest, ExecutionStatus, JobRecord, JobStatus, Lane,
    Policy, Source,
};
use beatbox_engine::BeatboxEngine;
use beatbox_server::{
    AuthMode, DEFAULT_SYNC_WALL_MS, JobStore, ServerConfig, origin_allowed, router,
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

fn add_one_request(n: i64) -> ExecuteRequest {
    ExecuteRequest {
        lane: Lane::Wasm,
        source: Source::WasmWat {
            text: r#"
            (module
              (func (export "run") (param i64) (result i64)
                local.get 0
                i64.const 1
                i64.add))
            "#
            .to_string(),
        },
        entrypoint: None,
        input: json!({"n": n}),
        stdin: String::new(),
        policy: Policy::default(),
        idempotency_key: None,
    }
}
