use axum::body::{Body, to_bytes};
use axum::http::header::ORIGIN;
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode};
use beatbox_core::{
    CreateJobResponse, ErrorResponse, ExecuteRequest, ExecutionResult, ExecutionStatus, JobRecord,
    JobStatus, Lane, Policy, Source,
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
    config.auth = AuthMode::required("secret")?;
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
async fn example_request_body_is_accepted_by_v1_execute() -> Result<(), Box<dyn std::error::Error>>
{
    // examples/req-fib.json is the repo's canonical HTTP request body; keep it a
    // request the REST API actually accepts (regression for the old wasm_file
    // example that /v1/execute rejected by design).
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/req-fib.json");
    let body = std::fs::read(&path)?;
    // It must be a valid ExecuteRequest under the strict (deny_unknown_fields) wire contract.
    let _: ExecuteRequest = serde_json::from_slice(&body)?;

    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/execute")
                .header("content-type", "application/json")
                .body(Body::from(body))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let result: ExecutionResult = serde_json::from_slice(&body)?;
    assert_eq!(result.status, ExecutionStatus::Ok);
    assert_eq!(result.value, json!(55));
    Ok(())
}

#[tokio::test]
async fn v1_execute_rejects_unknown_policy_fields() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    // A typo'd policy key (`polcy`) while trying to tighten the sandbox must be a
    // hard error, not a silently-dropped field that runs under default policy.
    let body = json!({
        "lane": "wasm",
        "source": {"kind": "wasm_wat", "text": add_one_wat()},
        "input": {"n": 41},
        "polcy": {"limits": {"wall_ms": 1}}
    });
    let response = app
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
    Ok(())
}

#[tokio::test]
async fn v1_execute_accepts_partial_limits() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    // A caller who wants to change one limit should not have to spell out all seven.
    let body = json!({
        "lane": "wasm",
        "source": {"kind": "wasm_wat", "text": add_one_wat()},
        "input": {"n": 41},
        "policy": {"limits": {"wall_ms": 1000}}
    });
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/execute")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let result: ExecutionResult = serde_json::from_slice(&body)?;
    assert_eq!(result.status, ExecutionStatus::Ok);
    assert_eq!(result.value, json!(42));
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
async fn canceling_terminal_job_conflicts() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let created: CreateJobResponse = serde_json::from_slice(
        &to_bytes(
            app.clone()
                .oneshot(
                    Request::builder()
                        .method(Method::POST)
                        .uri("/v1/jobs")
                        .header("content-type", "application/json")
                        .body(Body::from(serde_json::to_vec(&add_one_request(41))?))?,
                )
                .await?
                .into_body(),
            usize::MAX,
        )
        .await?,
    )?;

    // Wait for the fast job to finish, then a DELETE must report 409 (nothing to
    // cancel) rather than a spurious 204.
    let mut succeeded = false;
    for _ in 0..40 {
        let job = job_status(&app, &created.job_id).await?;
        if job == JobStatus::Succeeded {
            succeeded = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(succeeded, "job did not complete");

    let response = app
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
    assert_eq!(error.error.code, "job_already_terminal");
    Ok(())
}

#[tokio::test]
async fn canceling_running_job_frees_the_concurrency_slot() -> Result<(), Box<dyn std::error::Error>>
{
    // One job slot, and a high fuel/wall ceiling so a spin job stays running
    // (rather than fuel-exhausting) long enough to be canceled mid-flight.
    let mut config = ServerConfig::new(BeatboxEngine::new()?).with_max_concurrent_jobs(1);
    config.max_fuel = 100_000_000_000;
    let app = router(config);

    let long = spin_job_request();
    let created: CreateJobResponse = serde_json::from_slice(
        &to_bytes(submit_job(&app, &long).await?.into_body(), usize::MAX).await?,
    )?;

    // Wait until the worker has taken the slot (job is running).
    let mut running = false;
    for _ in 0..80 {
        if job_status(&app, &created.job_id).await? == JobStatus::Running {
            running = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(running, "long job never started running");

    // A second submission is refused: the only slot is held.
    let refused = submit_job(&app, &add_one_request(1)).await?;
    assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);

    // Cancel the running job.
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

    // The interrupted worker must release its permit promptly: a new submission
    // succeeds well before the 60s wall budget the canceled job would have run.
    let mut freed = false;
    for _ in 0..120 {
        if submit_job(&app, &add_one_request(2)).await?.status() == StatusCode::ACCEPTED {
            freed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(
        freed,
        "canceling the running job did not free the concurrency slot"
    );
    Ok(())
}

#[tokio::test]
async fn duplicate_submission_does_not_consume_a_permit() -> Result<(), Box<dyn std::error::Error>>
{
    // One slot, saturated by a long-running keyed job. A re-submit of the SAME
    // idempotency key must dedupe to 202 (not 429) even though no permit is free.
    let mut config = ServerConfig::new(BeatboxEngine::new()?).with_max_concurrent_jobs(1);
    config.max_fuel = 100_000_000_000;
    let app = router(config);

    let mut long = spin_job_request();
    long.idempotency_key = Some("dupe-key".to_string());
    let created: CreateJobResponse = serde_json::from_slice(
        &to_bytes(submit_job(&app, &long).await?.into_body(), usize::MAX).await?,
    )?;

    let mut running = false;
    for _ in 0..80 {
        if job_status(&app, &created.job_id).await? == JobStatus::Running {
            running = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(running, "long job never started running");

    // A brand-new job is refused (slot full)...
    assert_eq!(
        submit_job(&app, &add_one_request(1)).await?.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    // ...but a duplicate of the running job dedupes to the same id without a permit.
    let dup = submit_job(&app, &long).await?;
    assert_eq!(dup.status(), StatusCode::ACCEPTED);
    let dup: CreateJobResponse =
        serde_json::from_slice(&to_bytes(dup.into_body(), usize::MAX).await?)?;
    assert_eq!(dup.job_id, created.job_id);

    // Stop the long job so its worker doesn't run out the full wall budget.
    app.clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(format!("/v1/jobs/{}", created.job_id))
                .body(Body::empty())?,
        )
        .await?;
    Ok(())
}

async fn submit_job(
    app: &axum::Router,
    request: &ExecuteRequest,
) -> Result<axum::http::Response<Body>, Box<dyn std::error::Error>> {
    Ok(app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/jobs")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(request)?))?,
        )
        .await?)
}

async fn job_status(
    app: &axum::Router,
    job_id: &str,
) -> Result<JobStatus, Box<dyn std::error::Error>> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/v1/jobs/{job_id}"))
                .body(Body::empty())?,
        )
        .await?;
    let job: JobRecord =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await?)?;
    Ok(job.status)
}

fn spin_job_request() -> ExecuteRequest {
    let mut request = ExecuteRequest {
        lane: Lane::Wasm,
        source: Source::WasmWat {
            text: r#"(module (func (export "run") (param i64) (result i64) (loop br 0) (i64.const 0)))"#
                .to_string(),
        },
        entrypoint: None,
        input: json!({"n": 0}),
        stdin: String::new(),
        policy: Policy::default(),
        idempotency_key: None,
    };
    request.policy.limits.wall_ms = 60_000;
    request.policy.limits.fuel = Some(50_000_000_000);
    request
}

#[tokio::test]
async fn auth_required_rejects_keyless_requests() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::required("secret")?;
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

#[test]
fn auth_required_rejects_empty_tokens() {
    assert!(AuthMode::required("").is_err());
    assert!(AuthMode::required("   ").is_err());
    assert!(AuthMode::required("secret").is_ok());
}

#[tokio::test]
async fn auth_required_rejects_empty_api_key_header() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::required("secret")?;
    let app = router(config);
    // An empty header must never authorize, even though constant_time_eq(b"", b"")
    // would be true for an empty configured token (now unrepresentable).
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
    Ok(())
}

#[tokio::test]
async fn auth_required_keeps_bearer_compatibility() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::required("secret")?;
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
async fn auth_required_rejects_mcp_tools_list_without_key() -> Result<(), Box<dyn std::error::Error>>
{
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::required("secret")?;
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
    config.auth = AuthMode::required("secret")?;
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

    // The spec now carries full component schemas so SDKs can be generated from it.
    let schemas = value["components"]["schemas"]
        .as_object()
        .ok_or("openapi should expose components.schemas")?;
    for expected in [
        "ExecuteRequest",
        "ExecutionResult",
        "Policy",
        "Limits",
        "Source",
        "Metrics",
        "JobRecord",
        "ErrorResponse",
    ] {
        assert!(schemas.contains_key(expected), "missing schema: {expected}");
    }
    // The execute path references the ExecutionResult schema, not just a status code.
    let execute_200 = &value["paths"]["/v1/execute"]["post"]["responses"]["200"];
    let schema_ref = execute_200["content"]["application/json"]["schema"]["$ref"]
        .as_str()
        .ok_or("execute 200 should reference a schema")?;
    assert!(schema_ref.ends_with("/ExecutionResult"), "got {schema_ref}");

    // cpu_time_ms is nullable in the schema (honest metrics), not a plain integer.
    let cpu = &schemas["Metrics"]["properties"]["cpu_time_ms"];
    assert!(
        cpu.get("type").is_none()
            || cpu["type"]
                .as_array()
                .is_some_and(|t| t.iter().any(|v| v == "null")),
        "cpu_time_ms should be nullable, got {cpu}"
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
    assert!(
        value["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("content-type"))
    );
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
    assert!(
        value["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("content-type"))
    );
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
    assert!(
        value["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("ignored"))
    );
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
    assert!(
        value["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("exactly one"))
    );
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
    assert!(
        value["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("unsigned integer"))
    );
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
    assert!(
        value["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("code"))
    );
    Ok(())
}

#[tokio::test]
async fn mcp_run_python_reports_unavailable_lane() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "run_python",
            "arguments": {
                "code": "print(41 + 1)",
                "timeout_ms": 1000,
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
    let text = value["result"]["content"][0]["text"]
        .as_str()
        .ok_or("missing MCP tool result text")?;
    // An unavailable lane is a failed tool call, not a silent success.
    assert_eq!(value["result"]["isError"], serde_json::json!(true));
    let result: ExecutionResult = serde_json::from_str(text)?;
    assert_eq!(result.status, ExecutionStatus::Denied);
    assert_eq!(result.lane, Lane::PythonWasi);
    assert!(!result.deterministic);
    assert_eq!(
        result.error.as_ref().map(|error| error.code.as_str()),
        Some("lane_unavailable")
    );
    Ok(())
}

#[tokio::test]
async fn mcp_run_wasm_reports_is_error_on_trap() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "run_wasm",
            "arguments": {
                "wat": r#"(module (func (export "run") (result i64) unreachable))"#
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
    // A trapping guest must surface isError:true rather than looking successful.
    assert_eq!(value["result"]["isError"], serde_json::json!(true));
    let text = value["result"]["content"][0]["text"]
        .as_str()
        .ok_or("missing MCP tool result text")?;
    let result: ExecutionResult = serde_json::from_str(text)?;
    assert_eq!(result.status, ExecutionStatus::Error);

    // A successful run still reports isError:false.
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let ok_request = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": "run_wasm", "arguments": {"wat": add_one_wat(), "input": {"n": 41}}}
    });
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header("content-type", "application/json")
                .body(Body::from(ok_request.to_string()))?,
        )
        .await?;
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let value: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(value["result"]["isError"], serde_json::json!(false));
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
