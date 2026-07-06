use axum::body::{to_bytes, Body};
use axum::http::header::{CONTENT_TYPE, HOST, ORIGIN};
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode, Uri};
use beatbox_core::{
    CreateJobResponse, ErrorResponse, ExecuteRequest, ExecutionResult, ExecutionStatus, JobRecord,
    JobStatus, Lane, Policy, Source,
};
use beatbox_engine::{BeatboxEngine, MAX_PYTHON_SOURCE_BYTES, MAX_WASM_MODULE_BYTES};
use beatbox_server::{
    host_allowed, origin_allowed, request_target_allowed, router, AuthMode, JobStore, ServerConfig,
    DEFAULT_JOB_WALL_MS, DEFAULT_SYNC_WALL_MS,
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
async fn v1_execute_accepts_compact_limits_with_default_fill(
) -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "lane": "wasm",
        "source": {"kind": "wasm_wat", "text": add_one_wat()},
        "input": {"n": 41},
        "policy": {
            "limits": {
                "wall_ms": 1000
            }
        }
    });
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/execute")
                .header("content-type", "application/json")
                .body(Body::from(request.to_string()))?,
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
async fn v1_execute_rejects_unknown_compact_limit_fields() -> Result<(), Box<dyn std::error::Error>>
{
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "lane": "wasm",
        "source": {"kind": "wasm_wat", "text": add_one_wat()},
        "policy": {
            "limits": {
                "wall_mz": 1000
            }
        }
    });
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/execute")
                .header("content-type", "application/json")
                .body(Body::from(request.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let error: ErrorResponse = serde_json::from_slice(&body)?;
    assert_eq!(error.error.code, "invalid_json");
    assert!(error.error.message.contains("unknown field"));
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
async fn v1_execute_rejects_ambiguous_inline_wasm_sources() -> Result<(), Box<dyn std::error::Error>>
{
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let mut request = add_one_request(41);
    request.source = Source::Inline {
        code: add_one_wat().to_string(),
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
    assert_eq!(error.error.code, "source_lane_mismatch");
    assert!(error.error.message.contains("wasm_wat"));
    Ok(())
}

#[tokio::test]
async fn v1_execute_rejects_language_lane_wasm_sources() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let mut request = add_one_request(41);
    request.lane = Lane::PythonWasi;
    request.source = Source::WasmWat {
        text: add_one_wat().to_string(),
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
    assert_eq!(error.error.code, "source_lane_mismatch");
    assert!(error.error.message.contains("inline source code"));
    Ok(())
}

#[tokio::test]
async fn v1_execute_rejects_unimplemented_module_refs() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let mut request = add_one_request(41);
    request.source = Source::ModuleRef {
        sha256: "sha256:deadbeef".to_string(),
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
    assert_eq!(error.error.code, "module_ref_unavailable");
    Ok(())
}

#[tokio::test]
async fn v1_execute_rejects_unavailable_inline_lanes_before_worker(
) -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    for lane in [Lane::PythonWasi, Lane::JsWasm, Lane::JsNative, Lane::Exec] {
        let request = inline_request(lane);
        let response = app
            .clone()
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
        assert_eq!(error.error.code, "lane_unavailable");
        assert!(error.error.message.contains("not available"));
    }
    Ok(())
}

#[tokio::test]
async fn v1_execute_rejects_unsupported_wasm_stdin() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let mut request = add_one_request(41);
    request.stdin = "ignored input must fail".to_string();
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
    assert_eq!(error.error.code, "unsupported_request_field");
    assert!(error.error.message.contains("stdin"));
    Ok(())
}

#[tokio::test]
async fn v1_execute_rejects_wasm_source_over_memory_budget(
) -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let mut request = add_one_request(41);
    request.policy.limits.memory_bytes = 1;
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
    assert_eq!(error.error.code, "source_limit");
    assert!(error.error.message.contains("source"));
    Ok(())
}

#[tokio::test]
async fn v1_execute_rejects_over_disk_ceiling_before_worker(
) -> Result<(), Box<dyn std::error::Error>> {
    let app = router(
        ServerConfig::new(BeatboxEngine::new()?)
            .with_max_concurrent_sync(0)
            .with_max_disk_bytes(1024),
    );
    let mut request = add_one_request(41);
    request.policy.limits.disk_bytes = 1025;
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
    assert_eq!(error.error.code, "limit_exceeded");
    assert!(error.error.message.contains("disk_bytes"));
    Ok(())
}

#[tokio::test]
async fn v1_execute_rejects_unknown_json_fields() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    for body in [
        json!({
            "lane": "wasm",
            "source": {"kind": "wasm_wat", "text": add_one_wat()},
            "input": {"n": 41},
            "policy": {},
            "unknown_top_level": true
        }),
        json!({
            "lane": "wasm",
            "source": {"kind": "wasm_wat", "text": add_one_wat()},
            "input": {"n": 41},
            "policy": {"fs": {"workspace": null, "moutns": []}}
        }),
        json!({
            "lane": "wasm",
            "source": {
                "kind": "wasm_wat",
                "text": add_one_wat(),
                "path": "/etc/passwd"
            },
            "input": {"n": 41},
            "policy": {}
        }),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/execute")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await?;
        let error: ErrorResponse = serde_json::from_slice(&body)?;
        assert_eq!(error.error.code, "invalid_json");
        assert!(error.error.message.contains("unknown field"));
    }
    Ok(())
}

#[tokio::test]
async fn v1_execute_rejects_unenforceable_policy_before_worker(
) -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?).with_max_concurrent_sync(0));
    let mut request = add_one_request(41);
    request.policy.env = std::collections::BTreeMap::from([(
        "TOKEN".to_string(),
        "must-not-reach-worker".to_string(),
    )]);
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
    assert_eq!(error.error.code, "policy_unenforceable");
    assert!(error.error.message.contains("env"));
    Ok(())
}

#[tokio::test]
async fn v1_execute_rejects_python_native_unenforced_limits_before_worker(
) -> Result<(), Box<dyn std::error::Error>> {
    if !beatbox_engine::python_native_available() {
        return Ok(());
    }
    let app = router(ServerConfig::new(BeatboxEngine::new()?).with_max_concurrent_sync(0));
    let mut request = inline_request(Lane::PythonNative);
    request.policy.limits.memory_bytes += 1;
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
    assert_eq!(error.error.code, "policy_unenforceable");
    assert!(error.error.message.contains("limits.memory_bytes"));
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
async fn v1_execute_rejects_when_sync_concurrency_cap_is_exhausted(
) -> Result<(), Box<dyn std::error::Error>> {
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
    for _ in 0..120 {
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
async fn jobs_dedupe_idempotency_keys_after_normalization() -> Result<(), Box<dyn std::error::Error>>
{
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let mut first_request = add_one_request(41);
    first_request.idempotency_key = Some(" retry-key-normalized ".to_string());
    let mut second_request = add_one_request(41);
    second_request.idempotency_key = Some("retry-key-normalized".to_string());

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
    let first: CreateJobResponse =
        serde_json::from_slice(&to_bytes(first.into_body(), usize::MAX).await?)?;

    let second = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/jobs")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&second_request)?))?,
        )
        .await?;
    assert_eq!(second.status(), StatusCode::ACCEPTED);
    let second: CreateJobResponse =
        serde_json::from_slice(&to_bytes(second.into_body(), usize::MAX).await?)?;
    assert_eq!(first.job_id, second.job_id);

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/v1/jobs/{}", first.job_id))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let job: JobRecord =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await?)?;
    assert_eq!(
        job.request.idempotency_key.as_deref(),
        Some("retry-key-normalized")
    );
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
async fn jobs_reject_unavailable_inline_lanes_before_queueing(
) -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = inline_request(Lane::Exec);
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
    assert_eq!(error.error.code, "lane_unavailable");
    assert!(error.error.message.contains("exec"));
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
async fn jobs_reject_unenforceable_policy_before_queueing() -> Result<(), Box<dyn std::error::Error>>
{
    let app = router(ServerConfig::new(BeatboxEngine::new()?).with_max_concurrent_jobs(0));
    let mut request = add_one_request(41);
    request.policy.double_jail = true;
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
    assert_eq!(error.error.code, "policy_unenforceable");
    assert!(error.error.message.contains("double_jail"));
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
async fn jobs_reject_when_stored_job_quota_is_exhausted() -> Result<(), Box<dyn std::error::Error>>
{
    let app = router(ServerConfig::new(BeatboxEngine::new()?).with_max_stored_jobs(1));

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/jobs")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&add_one_request(41))?))?,
        )
        .await?;
    assert_eq!(first.status(), StatusCode::ACCEPTED);

    let second = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/jobs")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&add_one_request(42))?))?,
        )
        .await?;
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = to_bytes(second.into_body(), usize::MAX).await?;
    let error: ErrorResponse = serde_json::from_slice(&body)?;
    assert_eq!(error.error.code, "job_store_full");
    Ok(())
}

#[tokio::test]
async fn jobs_reuse_idempotent_request_when_stored_job_quota_is_exhausted(
) -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?).with_max_stored_jobs(1));
    let mut request = add_one_request(41);
    request.idempotency_key = Some("same-request".to_string());
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
    let first_created: CreateJobResponse =
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
    let second_created: CreateJobResponse =
        serde_json::from_slice(&to_bytes(second.into_body(), usize::MAX).await?)?;
    assert_eq!(first_created.job_id, second_created.job_id);
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
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(format!("/v1/jobs/{}", created.job_id))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

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
async fn jobs_reject_canceling_terminal_job() -> Result<(), Box<dyn std::error::Error>> {
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
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let created: CreateJobResponse =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await?)?;

    let mut succeeded = None;
    for _ in 0..120 {
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
        let job: JobRecord =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await?)?;
        if job.status == JobStatus::Succeeded {
            succeeded = Some(job);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let job = succeeded.ok_or("job did not succeed")?;
    assert_eq!(
        job.result.as_ref().map(|result| &result.value),
        Some(&json!(42))
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(format!("/v1/jobs/{}", created.job_id))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let error: ErrorResponse =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await?)?;
    assert_eq!(error.error.code, "job_not_cancelable");
    assert!(error.error.message.contains("succeeded"));

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/v1/jobs/{}", created.job_id))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let job: JobRecord =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await?)?;
    assert_eq!(job.status, JobStatus::Succeeded);
    Ok(())
}

#[tokio::test]
async fn canceling_running_job_interrupts_worker_and_releases_permit(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?).with_max_concurrent_jobs(1);
    config.max_fuel = 10_000_000_000;
    let app = router(config);
    let request = spin_request();
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
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let created: CreateJobResponse = serde_json::from_slice(&body)?;

    let mut running = false;
    for _ in 0..100 {
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
        let job: JobRecord =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await?)?;
        match job.status {
            JobStatus::Running => {
                running = true;
                break;
            }
            JobStatus::Queued => {}
            other => return Err(format!("long-running job reached {other:?} before cancel").into()),
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(running, "long-running job did not start");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(format!("/v1/jobs/{}", created.job_id))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let mut accepted_second_job = false;
    let second_request = add_one_request(41);
    for _ in 0..80 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/jobs")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&second_request)?))?,
            )
            .await?;
        match response.status() {
            StatusCode::ACCEPTED => {
                accepted_second_job = true;
                break;
            }
            StatusCode::TOO_MANY_REQUESTS => {
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            status => return Err(format!("unexpected second job status {status}").into()),
        }
    }
    assert!(
        accepted_second_job,
        "canceled running job did not release its worker permit"
    );

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/v1/jobs/{}", created.job_id))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let job: JobRecord =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await?)?;
    assert_eq!(job.status, JobStatus::Canceled);
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
                .header("x-beatbox-api-key", "secret")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn auth_required_rejects_empty_configured_token() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::Required {
        token: String::new(),
    };
    let app = router(config);
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/capabilities")
                .header("x-beatbox-api-key", "")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let error: ErrorResponse = serde_json::from_slice(&body)?;
    assert_eq!(error.error.code, "unauthorized");
    assert!(error.error.message.contains("empty"));
    Ok(())
}

#[tokio::test]
async fn auth_required_keeps_bearer_compatibility() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::Required {
        token: "secret".to_string(),
    };
    let app = router(config);
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
async fn auth_required_rejects_ambiguous_credential_headers(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::Required {
        token: "secret".to_string(),
    };
    let app = router(config);

    for request in [
        Request::builder()
            .method(Method::GET)
            .uri("/v1/capabilities")
            .header("x-beatbox-api-key", "wrong")
            .header("x-beatbox-api-key", "secret")
            .body(Body::empty())?,
        Request::builder()
            .method(Method::GET)
            .uri("/v1/capabilities")
            .header("authorization", "Bearer wrong")
            .header("authorization", "Bearer secret")
            .body(Body::empty())?,
        Request::builder()
            .method(Method::GET)
            .uri("/v1/capabilities")
            .header("x-beatbox-api-key", "secret")
            .header("authorization", "Bearer secret")
            .body(Body::empty())?,
    ] {
        let response = app.clone().oneshot(request).await?;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = to_bytes(response.into_body(), usize::MAX).await?;
        let error: ErrorResponse = serde_json::from_slice(&body)?;
        assert_eq!(error.error.code, "unauthorized");
    }
    Ok(())
}

#[tokio::test]
async fn auth_required_keeps_health_public_but_guards_openapi(
) -> Result<(), Box<dyn std::error::Error>> {
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
                .uri("/v1/health")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let value: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(value, json!({"status": "ok"}));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/openapi.json")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/openapi.json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let value: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(value["openapi"], "3.1.0");
    Ok(())
}

#[tokio::test]
async fn rest_control_plane_rejects_duplicate_boundary_headers_before_handling(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::Required {
        token: "secret".to_string(),
    };
    let app = router(config);

    for request in [
        Request::builder()
            .method(Method::POST)
            .uri("/v1/execute")
            .header(ORIGIN, "http://localhost:3000")
            .header(ORIGIN, "https://attacker.example")
            .header("x-beatbox-api-key", "secret")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from("{not json"))?,
        Request::builder()
            .method(Method::POST)
            .uri("/v1/execute")
            .header(HOST, "localhost:7300")
            .header(HOST, "attacker.example")
            .header("x-beatbox-api-key", "secret")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from("{not json"))?,
    ] {
        let response = app.clone().oneshot(request).await?;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(response.into_body(), usize::MAX).await?;
        let error: ErrorResponse = serde_json::from_slice(&body)?;
        assert_eq!(error.error.code, "forbidden");
    }
    Ok(())
}

#[tokio::test]
async fn rest_control_plane_rejects_cross_origin_requests_before_handling(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::Required {
        token: "secret".to_string(),
    };
    let app = router(config);

    for (method, uri, body) in [
        (Method::GET, "/v1/capabilities", ""),
        (Method::GET, "/openapi.json", ""),
        (Method::POST, "/v1/execute", "{not json"),
        (Method::POST, "/v1/jobs", "{not json"),
        (Method::GET, "/v1/jobs/missing", ""),
        (Method::DELETE, "/v1/jobs/missing", ""),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method.clone())
                    .uri(uri)
                    .header("origin", "https://attacker.example")
                    .header("x-beatbox-api-key", "secret")
                    .body(Body::from(body))?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {uri}");
        let body = to_bytes(response.into_body(), usize::MAX).await?;
        let error: ErrorResponse = serde_json::from_slice(&body)?;
        assert_eq!(error.error.code, "forbidden");
        assert!(error.error.message.contains("origin"));
    }
    Ok(())
}

#[tokio::test]
async fn rest_control_plane_rejects_cross_host_requests_before_handling(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::Required {
        token: "secret".to_string(),
    };
    let app = router(config);

    for (method, uri, body) in [
        (Method::GET, "/v1/capabilities", ""),
        (Method::GET, "/openapi.json", ""),
        (Method::POST, "/v1/execute", "{not json"),
        (Method::POST, "/v1/jobs", "{not json"),
        (Method::GET, "/v1/jobs/missing", ""),
        (Method::DELETE, "/v1/jobs/missing", ""),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method.clone())
                    .uri(uri)
                    .header(HOST, "attacker.example")
                    .header("x-beatbox-api-key", "secret")
                    .body(Body::from(body))?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {uri}");
        let body = to_bytes(response.into_body(), usize::MAX).await?;
        let error: ErrorResponse = serde_json::from_slice(&body)?;
        assert_eq!(error.error.code, "forbidden");
        assert!(error.error.message.contains("host"));
    }
    Ok(())
}

#[tokio::test]
async fn rest_control_plane_rejects_cross_request_target_before_handling(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::Required {
        token: "secret".to_string(),
    };
    let app = router(config);

    for (method, uri, body) in [
        (Method::GET, "/v1/capabilities", ""),
        (Method::GET, "/openapi.json", ""),
        (Method::POST, "/v1/execute", "{not json"),
        (Method::POST, "/v1/jobs", "{not json"),
        (Method::GET, "/v1/jobs/missing", ""),
        (Method::DELETE, "/v1/jobs/missing", ""),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method.clone())
                    .uri(format!("http://attacker.example{uri}"))
                    .header(HOST, "localhost:7300")
                    .header("x-beatbox-api-key", "secret")
                    .body(Body::from(body))?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {uri}");
        let body = to_bytes(response.into_body(), usize::MAX).await?;
        let error: ErrorResponse = serde_json::from_slice(&body)?;
        assert_eq!(error.error.code, "forbidden");
        assert!(error.error.message.contains("request target"));
    }
    Ok(())
}

#[tokio::test]
async fn rest_rejects_duplicate_content_type_before_parsing(
) -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/execute")
                .header(CONTENT_TYPE, "application/json")
                .header(CONTENT_TYPE, "text/plain")
                .body(Body::from("{not json"))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let error: ErrorResponse = serde_json::from_slice(&body)?;
    assert_eq!(error.error.code, "unsupported_media_type");
    Ok(())
}

#[tokio::test]
async fn capabilities_report_python_native_runtime_availability(
) -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/capabilities")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let value: serde_json::Value = serde_json::from_slice(&body)?;
    let lanes = value["lanes"]
        .as_array()
        .ok_or("capabilities lanes must be an array")?;
    assert_eq!(
        value["limits"]["max_wasm_module_bytes"],
        json!(MAX_WASM_MODULE_BYTES)
    );
    assert_eq!(
        value["limits"]["max_python_source_bytes"],
        json!(MAX_PYTHON_SOURCE_BYTES)
    );
    assert_eq!(
        value["limits"]["default_disk_bytes"],
        json!(Policy::default().limits.disk_bytes)
    );
    assert_eq!(
        value["limits"]["max_disk_bytes"],
        json!(beatbox_server::DEFAULT_MAX_DISK_BYTES)
    );
    let wasm = lanes
        .iter()
        .find(|lane| lane["lane"] == "wasm")
        .ok_or("missing wasm capabilities")?;
    assert!(wasm["mechanisms"]
        .as_array()
        .is_some_and(|mechanisms| mechanisms
            .iter()
            .any(|mechanism| mechanism == "module-byte-limit")));
    assert!(wasm["mechanisms"]
        .as_array()
        .is_some_and(|mechanisms| mechanisms
            .iter()
            .any(|mechanism| mechanism == "precompile-import-scan")));
    let python_native = lanes
        .iter()
        .find(|lane| lane["lane"] == "python_native")
        .ok_or("missing python_native capabilities")?;
    assert_eq!(
        python_native["available"],
        json!(beatbox_engine::python_native_available())
    );
    if beatbox_engine::python_native_available() {
        assert!(python_native["mechanisms"]
            .as_array()
            .is_some_and(|mechanisms| mechanisms
                .iter()
                .any(|mechanism| mechanism == "sandbox-exec")));
        assert!(python_native["mechanisms"]
            .as_array()
            .is_some_and(|mechanisms| mechanisms
                .iter()
                .any(|mechanism| mechanism == "trusted-python-binary")));
        assert!(python_native["mechanisms"]
            .as_array()
            .is_some_and(|mechanisms| mechanisms
                .iter()
                .any(|mechanism| mechanism == "runtime-read-allowlist")));
        assert!(python_native["mechanisms"]
            .as_array()
            .is_some_and(|mechanisms| mechanisms
                .iter()
                .any(|mechanism| mechanism == "mach-lookup-deny")));
        assert!(python_native["mechanisms"]
            .as_array()
            .is_some_and(|mechanisms| mechanisms
                .iter()
                .any(|mechanism| mechanism == "sysctl-read-deny")));
        assert!(python_native["mechanisms"]
            .as_array()
            .is_some_and(|mechanisms| mechanisms
                .iter()
                .any(|mechanism| mechanism == "source-byte-limit")));
        assert!(python_native["mechanisms"]
            .as_array()
            .is_some_and(|mechanisms| mechanisms
                .iter()
                .any(|mechanism| mechanism == "stdin-delivery-watchdog")));
        assert!(python_native["mechanisms"]
            .as_array()
            .is_some_and(|mechanisms| mechanisms
                .iter()
                .any(|mechanism| mechanism == "workspace-disk-quota")));
        assert!(python_native["downgrades"]
            .as_array()
            .is_some_and(|downgrades| downgrades
                .iter()
                .any(|downgrade| downgrade == "macos_native_lane_dev_grade")));
        assert!(!python_native["downgrades"]
            .as_array()
            .is_some_and(|downgrades| downgrades
                .iter()
                .any(|downgrade| downgrade == "disk_limit_not_enforced")));
    }
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
async fn auth_required_rejects_mcp_handshake_without_key() -> Result<(), Box<dyn std::error::Error>>
{
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::Required {
        token: "secret".to_string(),
    };
    let app = router(config);

    for request in [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "ping"}),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/mcp")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(request.to_string()))?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = to_bytes(response.into_body(), usize::MAX).await?;
        let value: serde_json::Value = serde_json::from_slice(&body)?;
        assert_eq!(value["error"]["code"], -32001);
        assert_eq!(value["id"], serde_json::Value::Null);
    }

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header(CONTENT_TYPE, "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(
                    json!({"jsonrpc": "2.0", "id": 3, "method": "initialize"}).to_string(),
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let value: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(value["result"]["serverInfo"]["name"], "beatbox");
    Ok(())
}

#[tokio::test]
async fn auth_required_rejects_mcp_get_without_key() -> Result<(), Box<dyn std::error::Error>> {
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
                .uri("/mcp")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let error: ErrorResponse = serde_json::from_slice(&body)?;
    assert_eq!(error.error.code, "unauthorized");

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/mcp")
                .header("x-beatbox-api-key", "secret")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
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
async fn auth_required_rejects_mcp_ambiguous_credentials_before_tool_dispatch(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::Required {
        token: "secret".to_string(),
    };
    let app = router(config);
    let request = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header(CONTENT_TYPE, "application/json")
                .header("x-beatbox-api-key", "secret")
                .header("authorization", "Bearer secret")
                .body(Body::from(request.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let value: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(value["error"]["code"], -32001);
    Ok(())
}

#[tokio::test]
async fn mcp_rejects_duplicate_content_type_before_json_parse(
) -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header(CONTENT_TYPE, "application/json")
                .header(CONTENT_TYPE, "text/plain")
                .body(Body::from("{not json"))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let value: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(value["error"]["code"], -32600);
    assert!(value["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("content-type")));
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
    assert_openapi_components(
        &value,
        &[
            "CreateJobResponse",
            "ErrorResponse",
            "ExecuteRequest",
            "ExecutionResult",
            "JobRecord",
            "Policy",
            "Source",
        ],
    );
    assert_openapi_schema_ref(
        &value,
        "/paths/~1v1~1execute/post/requestBody/content/application~1json/schema",
        "ExecuteRequest",
    );
    assert_openapi_schema_ref(
        &value,
        "/paths/~1v1~1execute/post/responses/200/content/application~1json/schema",
        "ExecutionResult",
    );
    assert_openapi_schema_ref(
        &value,
        "/paths/~1v1~1execute/post/responses/422/content/application~1json/schema",
        "ErrorResponse",
    );
    assert_openapi_schema_ref(
        &value,
        "/paths/~1v1~1jobs/post/requestBody/content/application~1json/schema",
        "ExecuteRequest",
    );
    assert_openapi_schema_ref(
        &value,
        "/paths/~1v1~1jobs/post/responses/202/content/application~1json/schema",
        "CreateJobResponse",
    );
    assert_openapi_schema_ref(
        &value,
        "/paths/~1v1~1jobs~1{id}/get/responses/200/content/application~1json/schema",
        "JobRecord",
    );
    assert!(
        value["components"]["schemas"]["ExecuteRequest"]["properties"]["lane"].is_object(),
        "ExecuteRequest schema must expose lane"
    );
    assert!(
        value["components"]["schemas"]["ExecuteRequest"]["properties"]["source"].is_object(),
        "ExecuteRequest schema must expose source"
    );
    assert!(
        value["components"]["schemas"]["ExecuteRequest"]["properties"]["policy"].is_object(),
        "ExecuteRequest schema must expose policy"
    );
    assert_openapi_additional_properties_false(&value, "ExecuteRequest");
    assert_openapi_additional_properties_false(&value, "Policy");
    assert_openapi_additional_properties_false(&value, "FsPolicy");
    assert_openapi_additional_properties_false(&value, "Mount");
    assert_openapi_additional_properties_false(&value, "Limits");
    assert!(
        value["components"]["schemas"]["Limits"]["required"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "default-filled Limits fields must not be required in OpenAPI"
    );
    assert_openapi_additional_properties_false(&value, "Source");
    assert_openapi_statuses(&value, "/v1/capabilities", "get", &["200", "401", "403"]);
    assert_openapi_statuses(
        &value,
        "/v1/execute",
        "post",
        &["200", "400", "401", "403", "422", "429"],
    );
    assert_openapi_statuses(
        &value,
        "/v1/jobs",
        "post",
        &["202", "400", "401", "403", "409", "422", "429"],
    );
    assert_openapi_statuses(
        &value,
        "/v1/jobs/{id}",
        "get",
        &["200", "401", "403", "404"],
    );
    assert_openapi_statuses(
        &value,
        "/v1/jobs/{id}",
        "delete",
        &["204", "401", "403", "409", "404"],
    );
    assert_openapi_statuses(&value, "/mcp", "get", &["405", "401", "403"]);
    assert_openapi_statuses(&value, "/mcp", "post", &["200", "202", "400", "401", "403"]);
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

    headers.insert(ORIGIN, HeaderValue::from_static(" http://localhost:3000"));
    assert!(!origin_allowed(&headers));

    headers.insert(ORIGIN, HeaderValue::from_static("http://localhost:3000 "));
    assert!(!origin_allowed(&headers));

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

    let mut headers = HeaderMap::new();
    headers.append(ORIGIN, HeaderValue::from_static("http://localhost:3000"));
    headers.append(ORIGIN, HeaderValue::from_static("https://attacker.example"));
    assert!(!origin_allowed(&headers));
}

#[test]
fn host_validation_rejects_localhost_prefix_bypass() {
    let mut headers = HeaderMap::new();
    headers.insert(HOST, HeaderValue::from_static("localhost:7300"));
    assert!(host_allowed(&headers));

    headers.insert(HOST, HeaderValue::from_static("LOCALHOST"));
    assert!(host_allowed(&headers));

    headers.insert(HOST, HeaderValue::from_static("127.0.0.1:7300"));
    assert!(host_allowed(&headers));

    headers.insert(HOST, HeaderValue::from_static("[::1]:7300"));
    assert!(host_allowed(&headers));

    headers.insert(HOST, HeaderValue::from_static("localhost:65535"));
    assert!(host_allowed(&headers));

    headers.insert(HOST, HeaderValue::from_static("[::1]:65535"));
    assert!(host_allowed(&headers));

    headers.insert(HOST, HeaderValue::from_static(" localhost:7300"));
    assert!(!host_allowed(&headers));

    headers.insert(HOST, HeaderValue::from_static("localhost:7300 "));
    assert!(!host_allowed(&headers));

    headers.insert(HOST, HeaderValue::from_static("localhost:"));
    assert!(!host_allowed(&headers));

    headers.insert(HOST, HeaderValue::from_static("localhost:65536"));
    assert!(!host_allowed(&headers));

    headers.insert(HOST, HeaderValue::from_static("127.0.0.1:65536"));
    assert!(!host_allowed(&headers));

    headers.insert(HOST, HeaderValue::from_static("[::1]:65536"));
    assert!(!host_allowed(&headers));

    headers.insert(HOST, HeaderValue::from_static("localhost.evil.com"));
    assert!(!host_allowed(&headers));

    headers.insert(HOST, HeaderValue::from_static("127.0.0.1.evil.com"));
    assert!(!host_allowed(&headers));

    headers.insert(HOST, HeaderValue::from_static("localhost/path"));
    assert!(!host_allowed(&headers));

    headers.insert(HOST, HeaderValue::from_static("localhost@evil.com"));
    assert!(!host_allowed(&headers));

    headers.insert(HOST, HeaderValue::from_static("[::1].evil.com"));
    assert!(!host_allowed(&headers));

    let mut headers = HeaderMap::new();
    headers.append(HOST, HeaderValue::from_static("localhost:7300"));
    headers.append(HOST, HeaderValue::from_static("attacker.example"));
    assert!(!host_allowed(&headers));
}

#[test]
fn request_target_validation_rejects_cross_authority_bypass() {
    assert!(request_target_allowed(&uri("/v1/capabilities")));
    assert!(request_target_allowed(&uri(
        "http://localhost:7300/v1/capabilities"
    )));
    assert!(request_target_allowed(&uri(
        "http://127.0.0.1:7300/v1/capabilities"
    )));
    assert!(request_target_allowed(&uri(
        "http://[::1]:7300/v1/capabilities"
    )));

    assert!(!request_target_allowed(&uri(
        "http://localhost.evil.com/v1/capabilities"
    )));
    assert!(!request_target_allowed(&uri(
        "http://127.0.0.1.evil.com/v1/capabilities"
    )));
    assert!(!request_target_allowed(&uri(
        "http://localhost@evil.com/v1/capabilities"
    )));
    assert!(!request_target_allowed(&uri(
        "http://localhost:65536/v1/capabilities"
    )));
    assert!(!request_target_allowed(&uri(
        "http://[::1]:65536/v1/capabilities"
    )));
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
async fn mcp_rejects_cross_host_before_json_parse() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header(HOST, "attacker.example")
                .header("content-type", "application/json")
                .body(Body::from("{not json"))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let value: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(value["error"]["code"], -32600);
    assert!(value["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("host")));
    Ok(())
}

#[tokio::test]
async fn mcp_rejects_cross_request_target_before_json_parse(
) -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("http://attacker.example/mcp")
                .header(HOST, "localhost:7300")
                .header("content-type", "application/json")
                .body(Body::from("{not json"))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let value: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(value["error"]["code"], -32600);
    assert!(value["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("request target")));
    Ok(())
}

#[tokio::test]
async fn mcp_tool_schemas_are_self_contained() -> Result<(), Box<dyn std::error::Error>> {
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
    let tools = value["result"]["tools"]
        .as_array()
        .ok_or("missing MCP tools array")?;

    for tool in tools {
        let schema = &tool["inputSchema"];
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], false);
        assert_no_empty_schema_objects(schema);
        assert_local_schema_refs_resolve(schema, schema);
    }

    let run_wasm = tools
        .iter()
        .find(|tool| tool["name"] == "run_wasm")
        .ok_or("missing run_wasm tool")?;
    assert!(run_wasm["inputSchema"]["oneOf"].is_array());
    assert_eq!(
        run_wasm["inputSchema"]["properties"]["input"]["allOf"][0]["$ref"],
        "#/$defs/json_value"
    );
    assert_eq!(
        run_wasm["inputSchema"]["properties"]["timeout_ms"]["minimum"],
        0
    );
    assert_eq!(run_wasm["inputSchema"]["properties"]["fuel"]["minimum"], 0);

    let run_python = tools
        .iter()
        .find(|tool| tool["name"] == "run_python")
        .ok_or("missing run_python tool")?;
    assert!(run_python["inputSchema"]["properties"]
        .get("input")
        .is_none());
    assert!(run_python["inputSchema"]["properties"]
        .get("memory_bytes")
        .is_none());
    assert_eq!(
        run_python["inputSchema"]["properties"]["disk_bytes"]["minimum"],
        0
    );
    Ok(())
}

#[tokio::test]
async fn mcp_rejects_missing_content_type() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .body(Body::from(request.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let value: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(value["error"]["code"], -32600);
    assert!(value["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("content-type")));
    Ok(())
}

#[tokio::test]
async fn mcp_rejects_text_plain_json_posts() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header("content-type", "text/plain")
                .body(Body::from(request.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let value: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(value["error"]["code"], -32600);
    assert!(value["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("content-type")));
    Ok(())
}

#[tokio::test]
async fn mcp_get_capabilities_rejects_unknown_arguments() -> Result<(), Box<dyn std::error::Error>>
{
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "get_capabilities",
            "arguments": {"ignored": true}
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
    assert!(value["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("ignored")));
    Ok(())
}

#[tokio::test]
async fn mcp_get_capabilities_returns_structured_content() -> Result<(), Box<dyn std::error::Error>>
{
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "get_capabilities",
            "arguments": {}
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
    let result = &value["result"];
    assert_eq!(result["content"][0]["text"], "beatbox capabilities");
    assert!(result["structuredContent"]["lanes"].is_array());
    assert!(result["structuredContent"]["limits"].is_object());
    assert_eq!(
        result["structuredContent"]["limits"]["max_wasm_module_bytes"],
        json!(MAX_WASM_MODULE_BYTES)
    );
    assert_eq!(
        result["structuredContent"]["limits"]["max_python_source_bytes"],
        json!(MAX_PYTHON_SOURCE_BYTES)
    );
    assert_eq!(
        result["structuredContent"]["limits"]["max_disk_bytes"],
        json!(beatbox_server::DEFAULT_MAX_DISK_BYTES)
    );
    assert!(result["structuredContent"]["limits"]["max_stored_jobs"].is_number());
    assert_eq!(result["isError"], false);
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
    assert!(value["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("sync_limit_exceeded")));
    Ok(())
}

#[tokio::test]
async fn mcp_run_wasm_returns_structured_content_once() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "run_wasm",
            "arguments": {
                "wat": add_one_wat(),
                "input": {"n": 41}
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
    let result: ExecutionResult =
        serde_json::from_value(value["result"]["structuredContent"].clone())?;
    assert_eq!(result.status, ExecutionStatus::Ok, "{}", result.stderr);
    assert_eq!(result.lane, Lane::Wasm);
    assert_eq!(result.value, json!(42));
    assert_eq!(value["result"]["isError"], false);

    let text = value["result"]["content"][0]["text"]
        .as_str()
        .ok_or("missing MCP text fallback")?;
    assert!(text.contains("beatbox execution ok on wasm"));
    assert!(
        serde_json::from_str::<serde_json::Value>(text).is_err(),
        "text fallback must not duplicate the structured JSON payload"
    );
    Ok(())
}

#[tokio::test]
async fn mcp_run_wasm_rejects_ambiguous_sources() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "run_wasm",
            "arguments": {
                "wat": add_one_wat(),
                "wasm_base64": "AGFzbQE="
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
    assert!(value["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("exactly one")));
    Ok(())
}

#[tokio::test]
async fn mcp_run_wasm_rejects_mistyped_limits() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "run_wasm",
            "arguments": {
                "wat": add_one_wat(),
                "fuel": "1000"
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
    assert!(value["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("unsigned integer")));
    Ok(())
}

#[tokio::test]
async fn mcp_run_python_requires_code() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "run_python",
            "arguments": {}
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
    assert!(value["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("code")));
    Ok(())
}

#[tokio::test]
async fn mcp_run_python_rejects_unsupported_input_argument(
) -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "run_python",
            "arguments": {
                "code": "print(42)",
                "input": {"n": 42}
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
    assert!(value["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("input")));
    Ok(())
}

#[tokio::test]
async fn mcp_run_python_rejects_memory_limit_argument() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "run_python",
            "arguments": {
                "code": "print(42)",
                "memory_bytes": 1048576
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
    assert!(value["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("memory_bytes")));
    Ok(())
}

#[tokio::test]
async fn mcp_run_python_rejects_mistyped_disk_limit_argument(
) -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "run_python",
            "arguments": {
                "code": "print(42)",
                "disk_bytes": "1048576"
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
    assert!(value["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("disk_bytes")));
    Ok(())
}

#[tokio::test]
async fn mcp_run_javascript_rejects_unavailable_lane() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "run_javascript",
            "arguments": {"code": "console.log(42)"}
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
            .is_some_and(
                |message| message.contains("lane_unavailable") && message.contains("js_wasm")
            )
    );
    Ok(())
}

#[tokio::test]
async fn mcp_run_python_reports_unavailable_lane() -> Result<(), Box<dyn std::error::Error>> {
    if beatbox_engine::python_native_available() {
        return Ok(());
    }
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "run_python",
            "arguments": {
                "code": "print(41 + 1)",
                "timeout_ms": 5000,
                "disk_bytes": 1048576
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
    if value.get("result").is_some() {
        let result: ExecutionResult =
            serde_json::from_value(value["result"]["structuredContent"].clone())?;
        assert_eq!(result.status, ExecutionStatus::Denied);
        assert_eq!(result.lane, Lane::PythonNative);
        assert!(!result.deterministic);
        assert_eq!(value["result"]["isError"], true);
        assert!(matches!(
            result.error.as_ref().map(|error| error.code.as_str()),
            Some("lane_unavailable" | "python_spawn")
        ));
    } else {
        assert_eq!(value["error"]["code"], -32602);
        assert!(value["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("python_native")));
    }
    Ok(())
}

#[tokio::test]
async fn mcp_run_python_runs_native_lane_when_available() -> Result<(), Box<dyn std::error::Error>>
{
    if !beatbox_engine::python_native_available() {
        return Ok(());
    }
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "run_python",
            "arguments": {
                "code": "print(41 + 1)",
                "timeout_ms": 5000,
                "disk_bytes": 1048576
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
    let result: ExecutionResult =
        serde_json::from_value(value["result"]["structuredContent"].clone())?;
    assert_eq!(result.status, ExecutionStatus::Ok, "{}", result.stderr);
    assert_eq!(result.lane, Lane::PythonNative);
    assert_eq!(result.stdout.trim(), "42");
    assert_eq!(value["result"]["isError"], false);
    assert!(result
        .effective_isolation
        .mechanisms
        .contains(&"sandbox-exec".to_string()));
    Ok(())
}

fn assert_no_empty_schema_objects(value: &serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            assert!(
                !object.is_empty(),
                "MCP schema contains an empty object placeholder"
            );
            for child in object.values() {
                assert_no_empty_schema_objects(child);
            }
        }
        serde_json::Value::Array(array) => {
            for child in array {
                assert_no_empty_schema_objects(child);
            }
        }
        _ => {}
    }
}

fn assert_local_schema_refs_resolve(root: &serde_json::Value, value: &serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(reference) = object.get("$ref").and_then(serde_json::Value::as_str) {
                let def_name = reference
                    .strip_prefix("#/$defs/")
                    .unwrap_or_else(|| panic!("unexpected non-local MCP schema ref: {reference}"));
                assert!(
                    root["$defs"].get(def_name).is_some(),
                    "unresolved MCP schema ref: {reference}"
                );
            }
            for child in object.values() {
                assert_local_schema_refs_resolve(root, child);
            }
        }
        serde_json::Value::Array(array) => {
            for child in array {
                assert_local_schema_refs_resolve(root, child);
            }
        }
        _ => {}
    }
}

fn assert_openapi_statuses(
    value: &serde_json::Value,
    path: &'static str,
    method: &'static str,
    statuses: &[&'static str],
) {
    let responses = value["paths"][path][method]["responses"]
        .as_object()
        .unwrap_or_else(|| panic!("missing OpenAPI responses for {method} {path}"));
    for status in statuses {
        assert!(
            responses.contains_key(*status),
            "missing OpenAPI status {status} for {method} {path}"
        );
    }
}

fn assert_openapi_components(value: &serde_json::Value, schemas: &[&'static str]) {
    let components = value["components"]["schemas"]
        .as_object()
        .unwrap_or_else(|| panic!("missing OpenAPI schema components"));
    for schema in schemas {
        assert!(
            components.contains_key(*schema),
            "missing OpenAPI schema component {schema}"
        );
    }
}

fn assert_openapi_additional_properties_false(value: &serde_json::Value, schema: &'static str) {
    let component = &value["components"]["schemas"][schema];
    assert!(
        schema_disallows_additional_properties(component),
        "OpenAPI schema component {schema} must set additionalProperties=false: {component}"
    );
}

fn schema_disallows_additional_properties(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(object) => {
            if object.get("additionalProperties") == Some(&serde_json::Value::Bool(false)) {
                return true;
            }
            object.values().any(schema_disallows_additional_properties)
        }
        serde_json::Value::Array(array) => array.iter().any(schema_disallows_additional_properties),
        _ => false,
    }
}

fn assert_openapi_schema_ref(value: &serde_json::Value, pointer: &'static str, schema: &str) {
    let actual = value
        .pointer(pointer)
        .unwrap_or_else(|| panic!("missing OpenAPI schema at {pointer}"));
    let expected = format!("#/components/schemas/{schema}");
    assert!(
        schema_contains_ref(actual, &expected),
        "OpenAPI schema at {pointer} does not reference {expected}: {actual}"
    );
}

fn schema_contains_ref(value: &serde_json::Value, expected: &str) -> bool {
    match value {
        serde_json::Value::Object(object) => {
            object
                .get("$ref")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|reference| reference == expected)
                || object
                    .values()
                    .any(|child| schema_contains_ref(child, expected))
        }
        serde_json::Value::Array(array) => array
            .iter()
            .any(|child| schema_contains_ref(child, expected)),
        _ => false,
    }
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

fn uri(value: &'static str) -> Uri {
    value
        .parse()
        .unwrap_or_else(|error| panic!("test URI must parse: {error}"))
}

fn inline_request(lane: Lane) -> ExecuteRequest {
    ExecuteRequest {
        lane,
        source: Source::Inline {
            code: "print('hello')".to_string(),
        },
        entrypoint: None,
        input: serde_json::Value::Null,
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

fn spin_request() -> ExecuteRequest {
    let mut request = ExecuteRequest {
        lane: Lane::Wasm,
        source: Source::WasmWat {
            text: r#"
            (module
              (func (export "run") (param i64) (result i64)
                (loop
                  br 0)
                i64.const 0))
            "#
            .to_string(),
        },
        entrypoint: None,
        input: json!({"n": 0}),
        stdin: String::new(),
        policy: Policy::default(),
        idempotency_key: None,
    };
    request.policy.limits.wall_ms = 5_000;
    request.policy.limits.fuel = Some(10_000_000_000);
    request
}
