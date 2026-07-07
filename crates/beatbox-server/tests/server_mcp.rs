use axum::body::{Body, to_bytes};
use axum::http::header::ORIGIN;
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode};
use beatbox_core::{
    BROWSER_ADAPTER_LAUNCH_LEASE_SECONDS, BrowserAdapterCapabilityIssueResponse,
    BrowserAdapterCompletionValidationDecision, BrowserAdapterCompletionValidationResponse,
    BrowserAdapterConformanceExpectation, BrowserAdapterContractResponse,
    BrowserAdapterLaunchClaimDecision, BrowserAdapterLaunchClaimResponse,
    BrowserAdapterLaunchPlanResponse, BrowserAdapterManifestResponse,
    BrowserAdapterRegistrationResponse, BrowserAdmissionDecision, BrowserAdmissionRequest,
    BrowserArtifactMode, BrowserCredentialMode, BrowserProfilesResponse,
    BrowserSandboxAvailability, BrowserSandboxControl, BrowserSandboxLevel,
    BrowserSensitiveActivityMode, BrowserSensitivity, BrowserSessionActor, CreateJobResponse,
    ErrorResponse, ExecuteRequest, ExecutionResult, ExecutionStatus, JobRecord, JobStatus, Lane,
    Policy, Source, browser_adapter_launch_template_expires_at,
    browser_adapter_launch_template_issued_at,
};
use beatbox_engine::BeatboxEngine;
use beatbox_server::{
    AETHER_PAYMENT_HASH_HEADER, AETHER_PAYMENT_HEADER, AuthMode, DEFAULT_JOB_WALL_MS,
    DEFAULT_SYNC_WALL_MS, JobStore, ServerConfig, origin_allowed, router,
};
use chrono::{DateTime, Utc};
use serde_json::json;
use tower::ServiceExt;

fn complete_adapter_manifest() -> serde_json::Value {
    json!({
        "adapter_id": "tempo-os-jail-v1",
        "contract_version": "browser-adapter-v1",
        "launch_endpoint": "https://adapter.example/launch",
        "supported_levels": [
            "ephemeral_profile",
            "network_suppressed",
            "sealed_state",
            "os_isolated",
            "remote_isolated"
        ],
        "supported_controls": [
            "fresh_profile",
            "no_ambient_credentials",
            "egress_policy",
            "local_network_block",
            "sealed_artifacts",
            "os_process_isolation",
            "remote_worker_isolation",
            "teardown_proof"
        ],
        "guard_fields": [
            "guard_plan.network.allowed_origins",
            "guard_plan.network.deny_private_networks",
            "guard_plan.network.deny_localhost",
            "guard_plan.network.deny_metadata_endpoints",
            "guard_plan.network.require_dns_rebinding_protection",
            "guard_plan.network.require_redirect_revalidation",
            "guard_plan.network.require_proxy_enforcement",
            "guard_plan.network.outbound_network_disabled_without_proxy",
            "guard_plan.credentials.mode",
            "guard_plan.credentials.ambient_credentials_allowed",
            "guard_plan.credentials.user_mediation_required",
            "guard_plan.credentials.scoped_secret_channel_required",
            "guard_plan.storage.mode",
            "guard_plan.storage.plaintext_persistence_allowed",
            "guard_plan.storage.explicit_artifact_allowlist_required",
            "guard_plan.storage.encryption_required_for_persistence",
            "guard_plan.storage.teardown_proof_required",
            "guard_plan.suppression.mode",
            "guard_plan.suppression.suppress_ambient_browser_state",
            "guard_plan.suppression.suppress_ambient_credentials",
            "guard_plan.suppression.suppress_unapproved_network",
            "guard_plan.suppression.suppress_persistent_artifacts",
            "guard_plan.suppression.downgrade_requires_user_approval",
            "guard_plan.suppression.required_operator_confirmations",
            "guard_plan.required_runtime_guards"
        ],
        "completion_proofs": [
            "browser process exited or was killed",
            "temporary profile directory removed",
            "plaintext artifacts outside the explicit allowlist removed",
            "egress proxy log sealed or discarded according to artifact_mode"
        ]
    })
}

fn complete_adapter_completion_report() -> serde_json::Value {
    json!({
        "request_id": "browser-adapter-conformance-launch-v1",
        "adapter_id": "tempo-conformance-adapter-v1",
        "contract_version": "browser-adapter-v1",
        "process_terminated": true,
        "temporary_profile_removed": true,
        "plaintext_artifacts_removed": true,
        "egress_log_sealed_or_discarded": true,
        "sealed_artifact_handles": [],
        "proof_ids": [
            "browser_process_terminated",
            "temporary_profile_removed",
            "plaintext_artifacts_removed",
            "egress_log_sealed_or_discarded"
        ],
        "notes": ["shape fixture only"]
    })
}

fn complete_adapter_registration() -> serde_json::Value {
    json!({
        "actor": "agent",
        "sensitivity": "sensitive",
        "same_user_capability": "test-capability-fixture",
        "manifest": complete_adapter_manifest()
    })
}

fn complete_adapter_launch_plan(same_user_capability: &str) -> serde_json::Value {
    json!({
        "same_user_capability": same_user_capability,
        "admission": {
            "requested_level": "os_isolated",
            "actor": "agent",
            "sensitivity": "sensitive",
            "sensitive_activity_mode": "network_suppressed",
            "target_origins": ["https://bank.example"],
            "credential_mode": "no_credentials",
            "artifact_mode": "discard",
            "required_controls": ["egress_policy", "teardown_proof"],
            "allow_downgrade": false,
            "task_label": "sensitive browser launch plan"
        },
        "manifest": complete_adapter_manifest()
    })
}

fn assert_adapter_validation_matches_expectation(
    validation: &BrowserAdapterManifestResponse,
    expected: &BrowserAdapterConformanceExpectation,
) {
    assert_eq!(validation.decision, expected.decision);
    assert_eq!(validation.manifest_complete, expected.manifest_complete);
    assert_eq!(validation.launchable, expected.launchable);
    assert_eq!(
        validation.trusted_for_sensitive_work,
        expected.trusted_for_sensitive_work
    );
    assert_eq!(
        validation.endpoint_network_policy_bound,
        expected.endpoint_network_policy_bound
    );
    assert_eq!(validation.missing_levels, expected.missing_levels);
    assert_eq!(validation.missing_controls, expected.missing_controls);
    assert_eq!(
        validation.missing_guard_fields,
        expected.missing_guard_fields
    );
    assert_eq!(
        validation.missing_completion_proofs,
        expected.missing_completion_proofs
    );
}

#[tokio::test]
async fn v1_execute_runs_wasm() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = add_one_request(41);
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
async fn browser_profiles_are_authenticated_control_plane_metadata()
-> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::required("secret")?;
    let app = router(config);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/browser/profiles")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/browser/profiles")
                .header("x-beatbox-api-key", "secret")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let profiles: BrowserProfilesResponse = serde_json::from_slice(&body)?;
    assert!(!profiles.runnable_browser_sessions);
    assert_eq!(profiles.default_level, None);
    assert_eq!(
        profiles.integration.status,
        BrowserSandboxAvailability::Planned
    );
    assert!(
        profiles
            .profiles
            .iter()
            .all(|profile| profile.availability != BrowserSandboxAvailability::Available),
        "no browser profile is runnable until a real browser substrate enforces it"
    );
    let Some(network_suppressed) = profiles
        .profiles
        .iter()
        .find(|profile| profile.level == BrowserSandboxLevel::NetworkSuppressed)
    else {
        panic!("network_suppressed profile should be published");
    };
    assert!(
        network_suppressed
            .controls
            .contains(&BrowserSandboxControl::EgressPolicy)
    );
    assert!(
        network_suppressed
            .controls
            .contains(&BrowserSandboxControl::LocalNetworkBlock)
    );
    Ok(())
}

#[tokio::test]
async fn browser_admission_is_authenticated_and_fails_closed()
-> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::required("secret")?;
    let app = router(config);
    let request = BrowserAdmissionRequest {
        requested_level: BrowserSandboxLevel::OsIsolated,
        actor: BrowserSessionActor::Agent,
        sensitivity: BrowserSensitivity::Sensitive,
        sensitive_activity_mode: BrowserSensitiveActivityMode::Sealed,
        target_origins: vec!["https://bank.example".to_string()],
        credential_mode: BrowserCredentialMode::UserMediated,
        artifact_mode: BrowserArtifactMode::SealedArtifacts,
        required_controls: vec![
            BrowserSandboxControl::EgressPolicy,
            BrowserSandboxControl::RemoteWorkerIsolation,
        ],
        allow_downgrade: true,
        task_label: Some("pay bills".to_string()),
    };

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/admit")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request)?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/admit")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(serde_json::to_vec(&request)?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let decision: beatbox_core::BrowserAdmissionResponse = serde_json::from_slice(&body)?;
    assert_eq!(decision.decision, BrowserAdmissionDecision::Rejected);
    assert!(!decision.runnable_browser_sessions);
    assert_eq!(decision.requested_level, BrowserSandboxLevel::OsIsolated);
    assert_eq!(decision.selected_level, None);
    assert_eq!(decision.actor, BrowserSessionActor::Agent);
    assert_eq!(decision.sensitivity, BrowserSensitivity::Sensitive);
    assert_eq!(
        decision.sensitive_activity_mode,
        BrowserSensitiveActivityMode::Sealed
    );
    assert_eq!(decision.target_origins, vec!["https://bank.example"]);
    assert_eq!(
        decision.credential_mode,
        BrowserCredentialMode::UserMediated
    );
    assert_eq!(decision.artifact_mode, BrowserArtifactMode::SealedArtifacts);
    assert_eq!(
        decision.requested_profile_controls,
        vec![
            BrowserSandboxControl::FreshProfile,
            BrowserSandboxControl::NoAmbientCredentials,
            BrowserSandboxControl::EgressPolicy,
            BrowserSandboxControl::LocalNetworkBlock,
            BrowserSandboxControl::OsProcessIsolation,
            BrowserSandboxControl::TeardownProof,
        ]
    );
    assert_eq!(
        decision.missing_controls,
        vec![BrowserSandboxControl::RemoteWorkerIsolation]
    );
    assert!(!decision.level_satisfies_requested_controls);
    assert!(decision.intent_warnings.is_empty());
    assert_eq!(
        decision.guard_plan.network.allowed_origins,
        vec!["https://bank.example"]
    );
    assert!(decision.guard_plan.network.deny_private_networks);
    assert!(decision.guard_plan.network.require_dns_rebinding_protection);
    assert!(decision.guard_plan.network.require_redirect_revalidation);
    assert!(decision.guard_plan.network.require_proxy_enforcement);
    assert!(
        decision
            .guard_plan
            .network
            .outbound_network_disabled_without_proxy
    );
    assert!(!decision.guard_plan.credentials.ambient_credentials_allowed);
    assert!(decision.guard_plan.credentials.user_mediation_required);
    assert!(
        decision
            .guard_plan
            .storage
            .encryption_required_for_persistence
    );
    assert!(decision.guard_plan.storage.teardown_proof_required);
    assert_eq!(
        decision.guard_plan.suppression.mode,
        BrowserSensitiveActivityMode::Sealed
    );
    assert!(
        decision
            .guard_plan
            .suppression
            .suppress_ambient_browser_state
    );
    assert!(decision.guard_plan.suppression.suppress_unapproved_network);
    assert!(
        decision
            .guard_plan
            .suppression
            .suppress_persistent_artifacts
    );
    assert!(
        decision
            .guard_plan
            .suppression
            .required_operator_confirmations
            .iter()
            .any(|confirmation| confirmation.contains("encrypted"))
    );
    assert!(
        decision
            .guard_plan
            .required_runtime_guards
            .iter()
            .any(|guard| guard.contains("final socket targets"))
    );
    assert!(!decision.adapter_handoff.launchable);
    assert_eq!(decision.adapter_handoff.launch_endpoint, None);
    assert_eq!(
        decision.adapter_handoff.contract_version,
        "browser-adapter-v1"
    );
    assert!(
        decision
            .adapter_handoff
            .handoff_fields
            .iter()
            .any(|field| field == "request_id")
    );
    assert!(
        decision
            .adapter_handoff
            .handoff_fields
            .iter()
            .any(|field| field == "adapter_id")
    );
    assert!(
        decision
            .adapter_handoff
            .handoff_fields
            .iter()
            .any(|field| field == "sensitive_activity_mode")
    );
    assert!(
        decision
            .adapter_handoff
            .handoff_fields
            .iter()
            .any(|field| field == "guard_plan")
    );
    assert_eq!(
        decision.adapter_handoff.launch_request_template.request_id,
        "browser-admission-launch-template-v1"
    );
    assert_eq!(
        decision.adapter_handoff.launch_request_template.adapter_id,
        None
    );
    assert_eq!(
        decision
            .adapter_handoff
            .launch_request_template
            .target_origins,
        vec!["https://bank.example"]
    );
    assert_eq!(
        decision.adapter_handoff.launch_request_template.actor,
        BrowserSessionActor::Agent
    );
    assert_eq!(
        decision.adapter_handoff.launch_request_template.sensitivity,
        BrowserSensitivity::Sensitive
    );
    assert_eq!(
        decision
            .adapter_handoff
            .launch_request_template
            .sensitive_activity_mode,
        BrowserSensitiveActivityMode::Sealed
    );
    assert!(
        decision
            .adapter_handoff
            .launch_request_template
            .same_user_capability_required
    );
    assert!(
        decision
            .adapter_handoff
            .launch_request_template
            .endpoint_network_policy_binding_required
    );
    assert!(
        decision
            .adapter_handoff
            .launch_request_template
            .replay_protection_required
    );
    assert_eq!(
        decision.adapter_handoff.launch_request_template.issued_at,
        browser_adapter_launch_template_issued_at()
    );
    assert_eq!(
        decision.adapter_handoff.launch_request_template.expires_at,
        browser_adapter_launch_template_expires_at()
    );
    assert_eq!(
        decision
            .adapter_handoff
            .launch_request_template
            .max_session_seconds,
        BROWSER_ADAPTER_LAUNCH_LEASE_SECONDS
    );
    assert_eq!(
        decision
            .adapter_handoff
            .launch_request_template
            .guard_plan
            .network
            .allowed_origins,
        decision.guard_plan.network.allowed_origins
    );
    assert!(
        decision
            .adapter_handoff
            .required_completion_proofs
            .iter()
            .any(|proof| proof.contains("temporary profile directory"))
    );
    assert!(
        decision
            .adapter_handoff
            .completion_proof_contract
            .iter()
            .any(|proof| proof.proof_id == "temporary_profile_removed"
                && proof.evidence_field == "temporary_profile_removed")
    );
    assert_eq!(
        decision
            .adapter_handoff
            .launch_request_template
            .completion_report_template
            .request_id,
        decision.adapter_handoff.launch_request_template.request_id
    );
    assert_eq!(
        decision
            .adapter_handoff
            .launch_request_template
            .completion_report_template
            .proof_ids,
        decision
            .adapter_handoff
            .launch_request_template
            .completion_proof_contract
            .iter()
            .map(|proof| proof.proof_id.clone())
            .collect::<Vec<_>>()
    );
    assert!(decision.downgrade_allowed);
    assert_eq!(decision.profiles_endpoint, "/v1/browser/profiles");
    assert!(
        decision
            .reasons
            .iter()
            .any(|reason| reason.contains("no weaker browser profile"))
    );
    Ok(())
}

#[tokio::test]
async fn browser_admission_rejects_unsafe_target_origins() -> Result<(), Box<dyn std::error::Error>>
{
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    for target_origin in [
        "http://127.0.0.1:3000",
        "https://100.64.0.1",
        "http://[::ffff:127.0.0.1]",
        "https://[::ffff:10.0.0.1]",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/browser/admit")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "requested_level": "network_suppressed",
                            "actor": "agent",
                            "sensitivity": "sensitive",
                            "target_origins": [target_origin]
                        })
                        .to_string(),
                    ))?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await?;
        let error: ErrorResponse = serde_json::from_slice(&body)?;
        assert_eq!(error.error.code, "invalid_browser_intent");
        assert!(error.error.message.contains("local or private"));
    }
    Ok(())
}

#[tokio::test]
async fn browser_admission_rejects_unknown_request_fields() -> Result<(), Box<dyn std::error::Error>>
{
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/admit")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "requested_level": "os_isolated",
                        "actor": "agent",
                        "sensitivity": "sensitive",
                        "allow_downgrade": false,
                        "secret_note": "ignored"
                    })
                    .to_string(),
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let error: ErrorResponse = serde_json::from_slice(&body)?;
    assert_eq!(error.error.code, "invalid_json");
    assert!(error.error.message.contains("secret_note"));
    Ok(())
}

#[tokio::test]
async fn browser_adapter_manifest_validation_is_authenticated_and_fail_closed()
-> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::required("secret")?;
    let app = router(config);
    let manifest = complete_adapter_manifest();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/validate")
                .header("content-type", "application/json")
                .body(Body::from(manifest.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/validate")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(manifest.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let validation: BrowserAdapterManifestResponse = serde_json::from_slice(&body)?;
    assert!(!validation.manifest_complete);
    assert!(!validation.launchable);
    assert!(!validation.trusted_for_sensitive_work);
    assert!(!validation.endpoint_network_policy_bound);
    assert_eq!(
        validation.launch_endpoint.as_deref(),
        Some("https://adapter.example/launch")
    );
    assert!(validation.missing_levels.is_empty());
    assert!(validation.missing_controls.is_empty());
    assert!(validation.missing_guard_fields.is_empty());
    assert!(validation.missing_completion_proofs.is_empty());
    assert!(
        validation
            .reasons
            .iter()
            .any(|reason| reason.contains("endpoint binding"))
    );
    assert_eq!(
        validation.conformance_profile.profile_version,
        "browser-adapter-conformance-v1"
    );
    assert_eq!(
        validation
            .conformance_profile
            .field_complete_manifest
            .contract_version,
        validation.adapter_contract.version
    );
    assert_eq!(
        validation
            .conformance_profile
            .field_complete_manifest
            .launch_endpoint
            .as_deref(),
        Some("https://adapter.example/launch")
    );
    assert_eq!(
        validation
            .conformance_profile
            .field_complete_launch_request
            .adapter_id
            .as_deref(),
        Some("tempo-conformance-adapter-v1")
    );
    assert_eq!(
        validation
            .conformance_profile
            .field_complete_launch_request
            .contract_version,
        validation.adapter_contract.version
    );
    assert_eq!(
        validation
            .conformance_profile
            .field_complete_launch_request
            .target_origins,
        vec!["https://example.com"]
    );
    assert!(
        validation
            .conformance_profile
            .field_complete_launch_request
            .same_user_capability_required
    );
    assert!(
        validation
            .conformance_profile
            .field_complete_launch_request
            .endpoint_network_policy_binding_required
    );
    assert!(
        validation
            .conformance_profile
            .field_complete_launch_request
            .replay_protection_required
    );
    assert_eq!(
        validation
            .conformance_profile
            .field_complete_launch_request
            .issued_at,
        browser_adapter_launch_template_issued_at()
    );
    assert_eq!(
        validation
            .conformance_profile
            .field_complete_launch_request
            .expires_at,
        browser_adapter_launch_template_expires_at()
    );
    assert!(
        validation
            .conformance_profile
            .field_complete_launch_request
            .completion_proof_contract
            .iter()
            .any(|proof| proof.proof_id == "egress_log_sealed_or_discarded")
    );
    assert_eq!(
        validation
            .conformance_profile
            .field_complete_launch_request
            .completion_report_template
            .adapter_id,
        "tempo-conformance-adapter-v1"
    );
    assert!(
        validation
            .conformance_profile
            .field_complete_launch_request
            .completion_report_template
            .temporary_profile_removed
    );
    assert!(
        !validation
            .conformance_profile
            .field_complete_expectation
            .launchable
    );
    assert!(
        !validation
            .conformance_profile
            .field_complete_expectation
            .endpoint_network_policy_bound
    );
    assert!(
        validation
            .conformance_profile
            .required_cases
            .iter()
            .any(
                |case| case.name == "dns_rebinding_hostname_stays_incomplete"
                    && case.expected_rest_status == StatusCode::OK.as_u16()
                    && case.expected_mcp_error_code.is_none()
                    && case
                        .expected_validation
                        .as_ref()
                        .is_some_and(|expected| !expected.endpoint_network_policy_bound)
            )
    );
    assert!(
        validation
            .conformance_profile
            .required_cases
            .iter()
            .any(
                |case| case.name == "insecure_scheme_rejected_before_validation"
                    && case.expected_rest_status == StatusCode::BAD_REQUEST.as_u16()
                    && case.expected_rest_error_code.as_deref()
                        == Some("invalid_browser_adapter_manifest")
                    && case.expected_mcp_error_code == Some(-32602)
                    && case
                        .expected_mcp_error_message_contains
                        .iter()
                        .any(|message| message == "must use https")
            )
    );
    assert!(
        validation
            .conformance_profile
            .required_cases
            .iter()
            .any(|case| case.name == "missing_required_level_reports_gap"
                && case
                    .expected_validation
                    .as_ref()
                    .is_some_and(|expected| expected
                        .missing_levels
                        .contains(&BrowserSandboxLevel::OsIsolated)))
    );
    Ok(())
}

#[tokio::test]
async fn browser_adapter_completion_validation_is_authenticated_and_fail_closed()
-> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::required("secret")?;
    let app = router(config);

    let report = complete_adapter_completion_report();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/completion/validate")
                .header("content-type", "application/json")
                .body(Body::from(report.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/completion/validate")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(report.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let validation: BrowserAdapterCompletionValidationResponse = serde_json::from_slice(&body)?;
    assert_eq!(
        validation.decision,
        BrowserAdapterCompletionValidationDecision::Rejected
    );
    assert!(validation.report_shape_complete);
    assert!(!validation.server_issued_launch_request);
    assert!(!validation.launch_request_claimed);
    assert!(!validation.launch_request_envelope_matched);
    assert!(!validation.completion_report_template_matched);
    assert!(!validation.completion_bound_to_claimed_launch);
    assert!(!validation.verified_on_production_path);
    assert!(!validation.trusted_for_sensitive_work);
    assert_eq!(
        validation.request_id,
        "browser-adapter-conformance-launch-v1"
    );
    assert_eq!(validation.adapter_id, "tempo-conformance-adapter-v1");
    assert!(validation.missing_proof_ids.is_empty());
    assert!(validation.unexpected_proof_ids.is_empty());
    assert!(validation.failed_evidence_fields.is_empty());
    assert!(
        validation
            .completion_proof_contract
            .iter()
            .any(|proof| proof.proof_id == "temporary_profile_removed"
                && proof.evidence_field == "temporary_profile_removed")
    );
    assert!(
        validation
            .reasons
            .iter()
            .any(|reason| reason.contains("not verified it on a real launch request"))
    );
    assert!(validation.reasons.iter().any(|reason| {
        reason.contains("not present in this daemon's bounded launch replay ledger")
    }));

    let mut missing = complete_adapter_completion_report();
    missing["proof_ids"] = serde_json::json!(["browser_process_terminated"]);
    missing["temporary_profile_removed"] = serde_json::json!(false);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/completion/validate")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(missing.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let validation: BrowserAdapterCompletionValidationResponse = serde_json::from_slice(&body)?;
    assert!(!validation.report_shape_complete);
    assert!(
        validation
            .missing_proof_ids
            .contains(&"temporary_profile_removed".to_string())
    );
    assert!(
        validation
            .failed_evidence_fields
            .contains(&"temporary_profile_removed".to_string())
    );

    let mut unknown = complete_adapter_completion_report();
    unknown["extra"] = serde_json::json!("nope");
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/completion/validate")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(unknown.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let error: ErrorResponse = serde_json::from_slice(&body)?;
    assert_eq!(error.error.code, "invalid_json");

    let mut whitespace = complete_adapter_completion_report();
    whitespace["request_id"] = serde_json::json!(" bad ");
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/completion/validate")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(whitespace.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let error: ErrorResponse = serde_json::from_slice(&body)?;
    assert_eq!(
        error.error.code,
        "invalid_browser_adapter_completion_report"
    );
    Ok(())
}

#[tokio::test]
async fn browser_adapter_contract_discovery_is_authenticated_and_fail_closed()
-> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::required("secret")?;
    let app = router(config);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/browser/adapter/contract")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/browser/adapter/contract")
                .header("x-beatbox-api-key", "secret")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let contract: BrowserAdapterContractResponse = serde_json::from_slice(&body)?;
    assert!(!contract.launchable);
    assert!(!contract.trusted_for_sensitive_work);
    assert!(!contract.endpoint_network_policy_bound);
    assert_eq!(contract.adapter_contract.version, "browser-adapter-v1");
    assert_eq!(
        contract.adapter_contract.status,
        BrowserSandboxAvailability::Planned
    );
    assert_eq!(contract.adapter_contract.launch_endpoint, None);
    assert!(
        contract
            .adapter_contract
            .completion_proof_contract
            .iter()
            .any(|proof| proof.proof_id == "browser_process_terminated"
                && proof.evidence_field == "process_terminated")
    );
    assert!(
        contract
            .required_levels
            .contains(&BrowserSandboxLevel::OsIsolated)
    );
    assert!(
        contract
            .required_levels
            .contains(&BrowserSandboxLevel::RemoteIsolated)
    );
    assert!(
        contract
            .required_controls
            .contains(&BrowserSandboxControl::LocalNetworkBlock)
    );
    assert!(
        contract
            .required_controls
            .contains(&BrowserSandboxControl::TeardownProof)
    );
    assert_eq!(
        contract.conformance_profile.profile_version,
        "browser-adapter-conformance-v1"
    );
    assert_eq!(
        contract
            .conformance_profile
            .field_complete_manifest
            .contract_version,
        contract.adapter_contract.version
    );
    assert_eq!(
        contract
            .conformance_profile
            .field_complete_launch_request
            .adapter_id
            .as_deref(),
        Some("tempo-conformance-adapter-v1")
    );
    assert!(
        contract
            .conformance_profile
            .field_complete_launch_request
            .required_completion_proofs
            .iter()
            .any(|proof| proof.contains("temporary profile directory"))
    );
    assert_eq!(
        contract
            .conformance_profile
            .field_complete_launch_request
            .completion_report_template
            .proof_ids
            .len(),
        contract.adapter_contract.completion_proof_contract.len()
    );
    assert!(
        contract
            .conformance_profile
            .required_cases
            .iter()
            .any(|case| case.name == "field_complete_manifest_stays_fail_closed")
    );
    assert!(
        contract
            .notes
            .iter()
            .any(|note| note.contains("not adapter registration"))
    );
    Ok(())
}

#[tokio::test]
async fn browser_adapter_capability_issue_requires_configured_auth_and_returns_bearer_once()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let issue = json!({
        "actor": "agent",
        "sensitivity": "sensitive",
        "adapter_id": "tempo-os-jail-v1",
        "ttl_seconds": 60
    });
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/capability")
                .header("content-type", "application/json")
                .body(Body::from(issue.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let error: ErrorResponse = serde_json::from_slice(&body)?;
    assert!(
        error
            .error
            .message
            .contains("requires daemon authentication")
    );

    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::required("secret")?;
    let app = router(config);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/capability")
                .header("content-type", "application/json")
                .body(Body::from("{not json"))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/capability")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(issue.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let issued: BrowserAdapterCapabilityIssueResponse = serde_json::from_slice(&body)?;
    assert!(
        issued
            .same_user_capability
            .starts_with("bbx-browser-adapter-cap-v1.")
    );
    assert!(issued.same_user_capability.len() <= 256);
    assert!(!issued.same_user_capability.chars().any(char::is_whitespace));
    assert_eq!(issued.ttl_seconds, 60);
    assert_eq!(issued.actor, BrowserSessionActor::Agent);
    assert_eq!(issued.sensitivity, BrowserSensitivity::Sensitive);
    assert_eq!(issued.adapter_id.as_deref(), Some("tempo-os-jail-v1"));
    assert_eq!(issued.registration_endpoint, "/v1/browser/adapter/register");
    assert!(issued.notes.iter().any(|note| {
        note.contains("registration or launch-plan preflight") && note.contains("first matching")
    }));
    Ok(())
}

#[tokio::test]
async fn browser_adapter_capability_binds_registration_once_without_trusting_adapter()
-> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::required("secret")?;
    let app = router(config);
    let issue = json!({
        "actor": "agent",
        "sensitivity": "sensitive",
        "adapter_id": "tempo-os-jail-v1"
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/capability")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(issue.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let issued: BrowserAdapterCapabilityIssueResponse = serde_json::from_slice(&body)?;

    let mut registration = complete_adapter_registration();
    registration["same_user_capability"] = json!(issued.same_user_capability);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/register")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(registration.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let raw = String::from_utf8(body.to_vec())?;
    assert!(!raw.contains("bbx-browser-adapter-cap-v1."));
    let bound: BrowserAdapterRegistrationResponse = serde_json::from_str(&raw)?;
    assert!(bound.same_user_capability_bound);
    assert!(!bound.registered);
    assert!(!bound.launchable);
    assert!(!bound.trusted_for_sensitive_work);
    assert!(!bound.endpoint_network_policy_bound);

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/register")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(registration.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let replay: BrowserAdapterRegistrationResponse = serde_json::from_slice(&body)?;
    assert!(!replay.same_user_capability_bound);
    Ok(())
}

#[tokio::test]
async fn browser_adapter_launch_plan_binds_capability_without_launching()
-> Result<(), Box<dyn std::error::Error>> {
    let unauthenticated = router(ServerConfig::new(BeatboxEngine::new()?));
    let response = unauthenticated
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/launch/plan")
                .header("content-type", "application/json")
                .body(Body::from("{not json"))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::required("secret")?;
    let app = router(config);
    let issue = json!({
        "actor": "agent",
        "sensitivity": "sensitive",
        "adapter_id": "tempo-os-jail-v1"
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/capability")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(issue.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let issued: BrowserAdapterCapabilityIssueResponse = serde_json::from_slice(&body)?;

    let launch_plan = complete_adapter_launch_plan(&issued.same_user_capability);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/launch/plan")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(launch_plan.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let raw = String::from_utf8(body.to_vec())?;
    assert!(!raw.contains(&issued.same_user_capability));
    assert!(!raw.contains("bbx-browser-adapter-cap-v1."));
    let plan: BrowserAdapterLaunchPlanResponse = serde_json::from_str(&raw)?;
    assert!(plan.same_user_capability_bound);
    assert!(!plan.launchable);
    assert!(!plan.trusted_for_sensitive_work);
    assert!(!plan.endpoint_network_policy_bound);
    assert!(plan.adapter_contract_fields_complete);
    assert!(plan.replay_protection_bound);
    assert_eq!(plan.adapter_id, "tempo-os-jail-v1");
    assert_eq!(plan.actor, BrowserSessionActor::Agent);
    assert_eq!(plan.sensitivity, BrowserSensitivity::Sensitive);
    assert_eq!(
        plan.admission.sensitive_activity_mode,
        BrowserSensitiveActivityMode::NetworkSuppressed
    );
    assert!(plan.request_id.starts_with("bbx-browser-launch-plan-v1."));
    assert_eq!(plan.launch_request.request_id, plan.request_id);
    let issued_at =
        DateTime::parse_from_rfc3339(&plan.launch_request.issued_at)?.with_timezone(&Utc);
    let expires_at =
        DateTime::parse_from_rfc3339(&plan.launch_request.expires_at)?.with_timezone(&Utc);
    assert!(expires_at > issued_at);
    assert_eq!(
        (expires_at - issued_at).num_seconds(),
        BROWSER_ADAPTER_LAUNCH_LEASE_SECONDS as i64
    );
    assert_eq!(
        plan.launch_request.max_session_seconds,
        BROWSER_ADAPTER_LAUNCH_LEASE_SECONDS
    );
    assert!(plan.launch_request.replay_protection_required);
    assert_eq!(
        plan.launch_request.adapter_id.as_deref(),
        Some("tempo-os-jail-v1")
    );
    assert_eq!(
        plan.launch_request.target_origins,
        vec!["https://bank.example".to_string()]
    );
    assert_eq!(
        plan.launch_request.sensitive_activity_mode,
        BrowserSensitiveActivityMode::NetworkSuppressed
    );
    assert!(
        plan.launch_request
            .guard_plan
            .suppression
            .suppress_unapproved_network
    );
    assert_eq!(
        plan.launch_request.completion_report_template.request_id,
        plan.request_id
    );
    assert_eq!(
        plan.launch_request.completion_report_template.adapter_id,
        "tempo-os-jail-v1"
    );
    assert_eq!(
        plan.completion_validation_endpoint,
        "/v1/browser/adapter/completion/validate"
    );
    assert_eq!(plan.admission.decision, BrowserAdmissionDecision::Rejected);
    assert!(!plan.admission.runnable_browser_sessions);
    assert!(!plan.manifest_validation.launchable);
    assert!(
        plan.reasons
            .iter()
            .any(|reason| reason.contains("not registration"))
    );
    assert!(plan.reasons.iter().any(|reason| {
        reason.contains("bounded replay ledger") && reason.contains("REST claim preflight")
    }));

    let completion_report = plan.launch_request.completion_report_template.clone();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/completion/validate")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(serde_json::to_string(&completion_report)?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let preclaim_completion: BrowserAdapterCompletionValidationResponse =
        serde_json::from_slice(&body)?;
    assert!(preclaim_completion.report_shape_complete);
    assert!(preclaim_completion.server_issued_launch_request);
    assert!(!preclaim_completion.launch_request_claimed);
    assert!(preclaim_completion.launch_request_envelope_matched);
    assert!(preclaim_completion.completion_report_template_matched);
    assert!(!preclaim_completion.completion_bound_to_claimed_launch);
    assert!(!preclaim_completion.verified_on_production_path);

    let claim_request = json!({ "launch_request": plan.launch_request });
    let mut mutated_claim = claim_request.clone();
    mutated_claim["launch_request"]["sensitive_activity_mode"] = json!("sealed");
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/launch/claim")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(mutated_claim.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let mutated_claim: BrowserAdapterLaunchClaimResponse = serde_json::from_slice(&body)?;
    assert_eq!(
        mutated_claim.decision,
        BrowserAdapterLaunchClaimDecision::Rejected
    );
    assert!(mutated_claim.server_issued_launch_request);
    assert!(!mutated_claim.canonical_request_matched);
    assert!(!mutated_claim.launch_request_claim_bound);
    assert!(!mutated_claim.launch_request_replay_detected);

    let mut unknown_claim = claim_request.clone();
    unknown_claim["launch_request"]["guard_plan"]["network"]["extra"] = json!(true);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/launch/claim")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(unknown_claim.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let error: ErrorResponse = serde_json::from_slice(&body)?;
    assert_eq!(error.error.code, "invalid_browser_adapter_launch_claim");
    assert!(
        error
            .error
            .message
            .contains("guard_plan.network does not accept field `extra`")
    );

    let mut omitted_claim = claim_request.clone();
    omitted_claim["launch_request"]
        .as_object_mut()
        .ok_or("claim launch_request should be an object")?
        .remove("completion_report_template");
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/launch/claim")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(omitted_claim.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let error: ErrorResponse = serde_json::from_slice(&body)?;
    assert_eq!(error.error.code, "invalid_browser_adapter_launch_claim");
    assert!(error.error.message.contains("completion_report_template"));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/launch/claim")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(claim_request.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let claim: BrowserAdapterLaunchClaimResponse = serde_json::from_slice(&body)?;
    assert_eq!(claim.decision, BrowserAdapterLaunchClaimDecision::Claimed);
    assert!(claim.server_issued_launch_request);
    assert!(claim.canonical_request_matched);
    assert!(claim.launch_request_unexpired);
    assert!(claim.launch_request_claim_bound);
    assert!(!claim.launch_request_replay_detected);
    assert!(!claim.launchable);
    assert!(!claim.trusted_for_sensitive_work);
    assert!(!claim.endpoint_network_policy_bound);
    assert!(claim.reasons.iter().any(|reason| {
        reason.contains("canonical envelope") && reason.contains("claimed exactly once")
    }));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/completion/validate")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(serde_json::to_string(&completion_report)?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let claimed_completion: BrowserAdapterCompletionValidationResponse =
        serde_json::from_slice(&body)?;
    assert!(claimed_completion.server_issued_launch_request);
    assert!(claimed_completion.launch_request_claimed);
    assert!(claimed_completion.launch_request_envelope_matched);
    assert!(claimed_completion.completion_report_template_matched);
    assert!(claimed_completion.completion_bound_to_claimed_launch);
    assert!(!claimed_completion.verified_on_production_path);
    assert!(
        claimed_completion
            .reasons
            .iter()
            .any(|reason| reason.contains("claimed through the REST launch-claim preflight"))
    );

    let mut mismatched_completion = serde_json::to_value(&completion_report)?;
    mismatched_completion["adapter_id"] = json!("different-adapter");
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/completion/validate")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(mismatched_completion.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let mismatched_completion: BrowserAdapterCompletionValidationResponse =
        serde_json::from_slice(&body)?;
    assert!(mismatched_completion.server_issued_launch_request);
    assert!(mismatched_completion.launch_request_claimed);
    assert!(!mismatched_completion.launch_request_envelope_matched);
    assert!(!mismatched_completion.completion_report_template_matched);
    assert!(!mismatched_completion.completion_bound_to_claimed_launch);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/launch/claim")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(claim_request.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let replayed_claim: BrowserAdapterLaunchClaimResponse = serde_json::from_slice(&body)?;
    assert_eq!(
        replayed_claim.decision,
        BrowserAdapterLaunchClaimDecision::Rejected
    );
    assert!(replayed_claim.server_issued_launch_request);
    assert!(replayed_claim.canonical_request_matched);
    assert!(replayed_claim.launch_request_unexpired);
    assert!(!replayed_claim.launch_request_claim_bound);
    assert!(replayed_claim.launch_request_replay_detected);

    let mut unknown_claim_id = claim_request.clone();
    unknown_claim_id["launch_request"]["request_id"] = json!("bbx-browser-launch-plan-v1.unknown");
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/launch/claim")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(unknown_claim_id.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let unknown_claim: BrowserAdapterLaunchClaimResponse = serde_json::from_slice(&body)?;
    assert_eq!(
        unknown_claim.decision,
        BrowserAdapterLaunchClaimDecision::Rejected
    );
    assert!(!unknown_claim.server_issued_launch_request);
    assert!(!unknown_claim.launch_request_claim_bound);

    let replay = complete_adapter_launch_plan(&issued.same_user_capability);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/launch/plan")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(replay.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let replay: BrowserAdapterLaunchPlanResponse = serde_json::from_slice(&body)?;
    assert!(!replay.same_user_capability_bound);
    assert!(!replay.replay_protection_bound);
    assert!(!replay.launchable);

    let mut unknown = complete_adapter_launch_plan("not-issued");
    unknown["extra"] = json!("nope");
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/launch/plan")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(unknown.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let error: ErrorResponse = serde_json::from_slice(&body)?;
    assert_eq!(error.error.code, "invalid_json");
    Ok(())
}

#[tokio::test]
async fn browser_adapter_launch_plan_binds_sensitive_activity_mode_capability()
-> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::required("secret")?;
    let app = router(config);

    let mismatched_issue = json!({
        "actor": "agent",
        "sensitivity": "sensitive",
        "sensitive_activity_mode": "private",
        "adapter_id": "tempo-os-jail-v1"
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/capability")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(mismatched_issue.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let mismatched: BrowserAdapterCapabilityIssueResponse = serde_json::from_slice(&body)?;
    assert_eq!(
        mismatched.sensitive_activity_mode,
        Some(BrowserSensitiveActivityMode::Private)
    );

    let launch_plan = complete_adapter_launch_plan(&mismatched.same_user_capability);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/launch/plan")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(launch_plan.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let plan: BrowserAdapterLaunchPlanResponse = serde_json::from_slice(&body)?;
    assert_eq!(
        plan.admission.sensitive_activity_mode,
        BrowserSensitiveActivityMode::NetworkSuppressed
    );
    assert!(!plan.same_user_capability_bound);
    assert!(!plan.replay_protection_bound);
    assert!(!plan.launchable);
    assert!(plan.reasons.iter().any(|reason| {
        reason.contains("sensitive_activity_mode") && reason.contains("adapter_id")
    }));

    let matching_issue = json!({
        "actor": "agent",
        "sensitivity": "sensitive",
        "sensitive_activity_mode": "network_suppressed",
        "adapter_id": "tempo-os-jail-v1"
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/capability")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(matching_issue.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let matching: BrowserAdapterCapabilityIssueResponse = serde_json::from_slice(&body)?;
    assert_eq!(
        matching.sensitive_activity_mode,
        Some(BrowserSensitiveActivityMode::NetworkSuppressed)
    );

    let launch_plan = complete_adapter_launch_plan(&matching.same_user_capability);
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/launch/plan")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(launch_plan.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let plan: BrowserAdapterLaunchPlanResponse = serde_json::from_slice(&body)?;
    assert!(plan.same_user_capability_bound);
    assert!(plan.replay_protection_bound);
    assert!(!plan.launchable);
    Ok(())
}

#[tokio::test]
async fn browser_adapter_launch_plan_requires_contract_complete_manifest_for_replay_binding()
-> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::required("secret")?;
    let app = router(config);

    let issue = json!({
        "actor": "agent",
        "sensitivity": "sensitive",
        "adapter_id": "tempo-os-jail-v1"
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/capability")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(issue.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let issued: BrowserAdapterCapabilityIssueResponse = serde_json::from_slice(&body)?;

    let mut launch_plan = complete_adapter_launch_plan(&issued.same_user_capability);
    launch_plan["manifest"]["guard_fields"] = json!([]);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/launch/plan")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(launch_plan.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let plan: BrowserAdapterLaunchPlanResponse = serde_json::from_slice(&body)?;
    assert!(plan.same_user_capability_bound);
    assert!(!plan.adapter_contract_fields_complete);
    assert!(!plan.replay_protection_bound);
    assert!(!plan.launchable);
    assert!(
        plan.manifest_validation
            .missing_guard_fields
            .iter()
            .any(|field| field == "guard_plan.network.allowed_origins")
    );
    assert!(
        plan.reasons
            .iter()
            .any(|reason| reason.contains("adapter field contract was incomplete"))
    );

    let claim_request = json!({ "launch_request": plan.launch_request });
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/launch/claim")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(claim_request.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let claim: BrowserAdapterLaunchClaimResponse = serde_json::from_slice(&body)?;
    assert_eq!(claim.decision, BrowserAdapterLaunchClaimDecision::Rejected);
    assert!(!claim.server_issued_launch_request);
    assert!(!claim.launch_request_claim_bound);

    Ok(())
}

#[tokio::test]
async fn browser_adapter_capability_binding_rejects_mismatch_and_expiry()
-> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::required("secret")?;
    let app = router(config);

    let issue = json!({
        "actor": "human",
        "sensitivity": "sensitive",
        "adapter_id": "tempo-os-jail-v1"
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/capability")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(issue.to_string()))?,
        )
        .await?;
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let issued: BrowserAdapterCapabilityIssueResponse = serde_json::from_slice(&body)?;
    let mut registration = complete_adapter_registration();
    registration["same_user_capability"] = json!(issued.same_user_capability);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/register")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(registration.to_string()))?,
        )
        .await?;
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let mismatch: BrowserAdapterRegistrationResponse = serde_json::from_slice(&body)?;
    assert!(!mismatch.same_user_capability_bound);
    assert!(mismatch.reasons.iter().any(|reason| {
        reason.contains("sensitive_activity_mode") && reason.contains("adapter_id")
    }));

    let issue = json!({
        "actor": "agent",
        "sensitivity": "sensitive",
        "ttl_seconds": 1
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/capability")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(issue.to_string()))?,
        )
        .await?;
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let issued: BrowserAdapterCapabilityIssueResponse = serde_json::from_slice(&body)?;
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let mut registration = complete_adapter_registration();
    registration["same_user_capability"] = json!(issued.same_user_capability);
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/register")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(registration.to_string()))?,
        )
        .await?;
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let expired: BrowserAdapterRegistrationResponse = serde_json::from_slice(&body)?;
    assert!(!expired.same_user_capability_bound);
    Ok(())
}

#[tokio::test]
async fn browser_adapter_capability_unbound_adapter_id_matches_any_manifest()
-> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::required("secret")?;
    let app = router(config);
    let issue = json!({"actor": "agent", "sensitivity": "sensitive"});
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/capability")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(issue.to_string()))?,
        )
        .await?;
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let issued: BrowserAdapterCapabilityIssueResponse = serde_json::from_slice(&body)?;
    assert_eq!(issued.adapter_id, None);
    let mut registration = complete_adapter_registration();
    registration["same_user_capability"] = json!(issued.same_user_capability);
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/register")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(registration.to_string()))?,
        )
        .await?;
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let registration: BrowserAdapterRegistrationResponse = serde_json::from_slice(&body)?;
    assert!(registration.same_user_capability_bound);
    Ok(())
}

#[tokio::test]
async fn browser_adapter_capability_quota_limits_live_tokens()
-> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::required("secret")?;
    let app = router(config);
    for index in 0..beatbox_server::MAX_BROWSER_ADAPTER_CAPABILITIES {
        let issue = json!({
            "actor": "agent",
            "sensitivity": "sensitive",
            "adapter_id": format!("tempo-os-jail-v1-{index}")
        });
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/browser/adapter/capability")
                    .header("content-type", "application/json")
                    .header("x-beatbox-api-key", "secret")
                    .body(Body::from(issue.to_string()))?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
    }
    let issue = json!({
        "actor": "agent",
        "sensitivity": "sensitive",
        "adapter_id": "tempo-over-quota"
    });
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/capability")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(issue.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let error: ErrorResponse = serde_json::from_slice(&body)?;
    assert_eq!(error.error.code, "browser_adapter_capability_quota");
    Ok(())
}

#[tokio::test]
async fn browser_adapter_capability_concurrent_register_binds_once()
-> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::required("secret")?;
    let app = router(config);
    let issue = json!({
        "actor": "agent",
        "sensitivity": "sensitive",
        "adapter_id": "tempo-os-jail-v1"
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/capability")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(issue.to_string()))?,
        )
        .await?;
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let issued: BrowserAdapterCapabilityIssueResponse = serde_json::from_slice(&body)?;
    let mut registration = complete_adapter_registration();
    registration["same_user_capability"] = json!(issued.same_user_capability);
    let request_a = Request::builder()
        .method(Method::POST)
        .uri("/v1/browser/adapter/register")
        .header("content-type", "application/json")
        .header("x-beatbox-api-key", "secret")
        .body(Body::from(registration.to_string()))?;
    let request_b = Request::builder()
        .method(Method::POST)
        .uri("/v1/browser/adapter/register")
        .header("content-type", "application/json")
        .header("x-beatbox-api-key", "secret")
        .body(Body::from(registration.to_string()))?;
    let (response_a, response_b) = tokio::join!(
        app.clone().oneshot(request_a),
        app.clone().oneshot(request_b)
    );
    let response_a = response_a?;
    let response_b = response_b?;
    assert_eq!(response_a.status(), StatusCode::OK);
    assert_eq!(response_b.status(), StatusCode::OK);
    let body_a = to_bytes(response_a.into_body(), usize::MAX).await?;
    let body_b = to_bytes(response_b.into_body(), usize::MAX).await?;
    let registration_a: BrowserAdapterRegistrationResponse = serde_json::from_slice(&body_a)?;
    let registration_b: BrowserAdapterRegistrationResponse = serde_json::from_slice(&body_b)?;
    let bound_count = usize::from(registration_a.same_user_capability_bound)
        + usize::from(registration_b.same_user_capability_bound);
    assert_eq!(bound_count, 1);
    Ok(())
}

#[tokio::test]
async fn browser_adapter_registration_is_authenticated_fail_closed_and_non_echoing()
-> Result<(), Box<dyn std::error::Error>> {
    let mut config = ServerConfig::new(BeatboxEngine::new()?);
    config.auth = AuthMode::required("secret")?;
    let app = router(config);
    let registration = complete_adapter_registration();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/register")
                .header("content-type", "application/json")
                .body(Body::from(registration.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/register")
                .header("content-type", "application/json")
                .header("x-beatbox-api-key", "secret")
                .body(Body::from(registration.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let raw = String::from_utf8(body.to_vec())?;
    assert!(!raw.contains("test-capability-fixture"));
    assert!(!raw.contains("same_user_capability\":"));
    let registration: BrowserAdapterRegistrationResponse = serde_json::from_str(&raw)?;
    assert_eq!(registration.adapter_id, "tempo-os-jail-v1");
    assert_eq!(registration.actor, BrowserSessionActor::Agent);
    assert_eq!(registration.sensitivity, BrowserSensitivity::Sensitive);
    assert!(!registration.registered);
    assert!(!registration.launchable);
    assert!(!registration.trusted_for_sensitive_work);
    assert!(!registration.endpoint_network_policy_bound);
    assert!(!registration.same_user_capability_bound);
    assert!(!registration.manifest_validation.manifest_complete);
    assert!(!registration.manifest_validation.launchable);
    assert!(
        registration
            .reasons
            .iter()
            .any(|reason| reason.contains("does not persist or trust adapters yet"))
    );
    assert!(
        registration
            .required_next_steps
            .iter()
            .any(|step| step.contains("same-user capability"))
    );
    Ok(())
}

#[tokio::test]
async fn browser_adapter_registration_rejects_invalid_capability()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let mut registration = complete_adapter_registration();
    registration["same_user_capability"] = json!(" test-capability-fixture ");
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/register")
                .header("content-type", "application/json")
                .body(Body::from(registration.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let raw = String::from_utf8(body.to_vec())?;
    assert!(!raw.contains("test-capability-fixture"));
    let error: ErrorResponse = serde_json::from_str(&raw)?;
    assert_eq!(error.error.code, "invalid_browser_adapter_registration");
    assert!(
        error
            .error
            .message
            .contains("same_user_capability must be non-empty")
    );
    Ok(())
}

#[tokio::test]
async fn browser_adapter_registration_errors_do_not_echo_capability_or_endpoint_secret()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let mut registration = complete_adapter_registration();
    registration["manifest"]["launch_endpoint"] =
        json!("https://adapter.example/launch?token=endpoint-fixture");
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/register")
                .header("content-type", "application/json")
                .body(Body::from(registration.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let raw = String::from_utf8(body.to_vec())?;
    assert!(!raw.contains("test-capability-fixture"));
    assert!(!raw.contains("endpoint-fixture"));
    assert!(!raw.contains("adapter.example/launch"));
    let error: ErrorResponse = serde_json::from_str(&raw)?;
    assert_eq!(error.error.code, "invalid_browser_adapter_registration");
    assert!(error.error.message.contains("query or fragment components"));
    Ok(())
}

#[tokio::test]
async fn browser_adapter_registration_rejects_unknown_request_fields()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let mut registration = complete_adapter_registration();
    registration["unexpected"] = json!("ignored");
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/register")
                .header("content-type", "application/json")
                .body(Body::from(registration.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let raw = String::from_utf8(body.to_vec())?;
    assert!(!raw.contains("test-capability-fixture"));
    let error: ErrorResponse = serde_json::from_str(&raw)?;
    assert_eq!(error.error.code, "invalid_json");
    assert!(error.error.message.contains("unexpected"));
    Ok(())
}

#[tokio::test]
async fn browser_adapter_conformance_profile_cases_match_rest_and_mcp()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/validate")
                .header("content-type", "application/json")
                .body(Body::from(complete_adapter_manifest().to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let validation: BrowserAdapterManifestResponse = serde_json::from_slice(&body)?;

    for case in &validation.conformance_profile.required_cases {
        let rest_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/browser/adapter/validate")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&case.manifest)?))?,
            )
            .await?;
        assert_eq!(
            rest_response.status().as_u16(),
            case.expected_rest_status,
            "REST conformance case {} returned unexpected status",
            case.name
        );
        let rest_body = to_bytes(rest_response.into_body(), usize::MAX).await?;
        if let Some(expected_error_code) = &case.expected_rest_error_code {
            let error: ErrorResponse = serde_json::from_slice(&rest_body)?;
            assert_eq!(
                error.error.code, *expected_error_code,
                "REST conformance case {} returned unexpected error code",
                case.name
            );
        } else {
            let rest_validation: BrowserAdapterManifestResponse =
                serde_json::from_slice(&rest_body)?;
            let expected = case
                .expected_validation
                .as_ref()
                .ok_or("successful REST case should publish expected_validation")?;
            assert_adapter_validation_matches_expectation(&rest_validation, expected);
        }

        let mcp_request = json!({
            "jsonrpc": "2.0",
            "id": case.name,
            "method": "tools/call",
            "params": {
                "name": "validate_browser_adapter",
                "arguments": case.manifest
            }
        });
        let mcp_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .body(Body::from(mcp_request.to_string()))?,
            )
            .await?;
        assert_eq!(mcp_response.status(), StatusCode::OK);
        let mcp_body = to_bytes(mcp_response.into_body(), usize::MAX).await?;
        let mcp_value: serde_json::Value = serde_json::from_slice(&mcp_body)?;
        if let Some(expected_mcp_error_code) = case.expected_mcp_error_code {
            assert_eq!(
                mcp_value["error"]["code"], expected_mcp_error_code,
                "MCP conformance case {} returned unexpected error code",
                case.name
            );
            for expected_message in &case.expected_mcp_error_message_contains {
                assert!(
                    mcp_value["error"]["message"]
                        .as_str()
                        .is_some_and(|message| message.contains(expected_message)),
                    "MCP conformance case {} error should contain {:?}",
                    case.name,
                    expected_message
                );
            }
        } else {
            let mcp_validation: BrowserAdapterManifestResponse =
                serde_json::from_value(mcp_value["result"]["structuredContent"].clone())?;
            let expected = case
                .expected_validation
                .as_ref()
                .ok_or("successful MCP case should publish expected_validation")?;
            assert_adapter_validation_matches_expectation(&mcp_validation, expected);
        }
    }
    Ok(())
}

#[tokio::test]
async fn browser_adapter_manifest_reports_missing_contract_parts()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/validate")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "adapter_id": "partial",
                        "contract_version": "browser-adapter-v1",
                        "launch_endpoint": null,
                        "supported_levels": ["network_suppressed"],
                        "supported_controls": ["fresh_profile", "egress_policy"],
                        "guard_fields": ["guard_plan.network.allowed_origins"],
                        "completion_proofs": []
                    })
                    .to_string(),
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let validation: BrowserAdapterManifestResponse = serde_json::from_slice(&body)?;
    assert!(!validation.manifest_complete);
    assert!(!validation.launchable);
    assert!(validation.launch_endpoint.is_none());
    assert!(
        validation
            .missing_levels
            .contains(&BrowserSandboxLevel::OsIsolated)
    );
    assert!(
        validation
            .missing_controls
            .contains(&BrowserSandboxControl::TeardownProof)
    );
    assert!(
        validation
            .missing_guard_fields
            .iter()
            .any(|field| field == "guard_plan.storage.teardown_proof_required")
    );
    assert!(
        validation
            .missing_completion_proofs
            .iter()
            .any(|proof| proof.contains("temporary profile directory"))
    );
    Ok(())
}

#[tokio::test]
async fn browser_adapter_manifest_rejects_unsafe_launch_endpoints()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    for endpoint in [
        "http://adapter.example/launch",
        "https://127.0.0.1/launch",
        "https://[::ffff:10.0.0.1]/launch",
        "https://adapter.example/launch?token=secret",
    ] {
        let mut manifest = complete_adapter_manifest();
        manifest["launch_endpoint"] = json!(endpoint);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/browser/adapter/validate")
                    .header("content-type", "application/json")
                    .body(Body::from(manifest.to_string()))?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await?;
        let raw = String::from_utf8(body.to_vec())?;
        assert!(!raw.contains(endpoint));
        assert!(!raw.contains("token=secret"));
        let error: ErrorResponse = serde_json::from_str(&raw)?;
        assert_eq!(error.error.code, "invalid_browser_adapter_manifest");
    }
    Ok(())
}

#[tokio::test]
async fn browser_adapter_manifest_does_not_complete_dns_unverified_endpoints()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let mut manifest = complete_adapter_manifest();
    manifest["launch_endpoint"] = json!("https://127.0.0.1.nip.io/launch");
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/browser/adapter/validate")
                .header("content-type", "application/json")
                .body(Body::from(manifest.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let validation: BrowserAdapterManifestResponse = serde_json::from_slice(&body)?;
    assert!(!validation.manifest_complete);
    assert!(!validation.launchable);
    assert!(!validation.endpoint_network_policy_bound);
    assert!(validation.missing_levels.is_empty());
    assert!(validation.missing_controls.is_empty());
    assert!(validation.missing_guard_fields.is_empty());
    assert!(validation.missing_completion_proofs.is_empty());
    assert!(
        validation
            .reasons
            .iter()
            .any(|reason| reason.contains("DNS, proxy, redirect, and retry"))
    );
    Ok(())
}

#[tokio::test]
async fn capabilities_embed_the_browser_profile_contract() -> Result<(), Box<dyn std::error::Error>>
{
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
    assert_eq!(value["browser_sandbox"]["runnable_browser_sessions"], false);
    assert_eq!(
        value["browser_sandbox"]["default_level"],
        serde_json::Value::Null
    );
    assert_eq!(value["browser_sandbox"]["integration"]["consumer"], "tempo");
    assert_eq!(
        value["browser_sandbox"]["integration"]["adapter"]["status"],
        "planned"
    );
    assert_eq!(
        value["browser_sandbox"]["integration"]["adapter"]["launch_endpoint"],
        serde_json::Value::Null
    );
    assert!(
        value["browser_sandbox"]["integration"]["adapter"]["handoff_fields"]
            .as_array()
            .is_some_and(|fields| fields.iter().any(|field| field == "guard_plan"))
    );
    assert!(
        value["browser_sandbox"]["integration"]["adapter"]["required_completion_proofs"]
            .as_array()
            .is_some_and(|proofs| proofs.iter().any(|proof| proof
                .as_str()
                .is_some_and(|proof| proof.contains("temporary profile directory"))))
    );
    assert!(
        value["browser_sandbox"]["profiles"]
            .as_array()
            .is_some_and(|profiles| profiles
                .iter()
                .all(|profile| profile["availability"] != "available"))
    );
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
            .is_some_and(|paths| paths.contains_key("/v1/jobs")
                && paths.contains_key("/v1/browser/profiles")
                && paths.contains_key("/v1/browser/admit")
                && paths.contains_key("/v1/browser/adapter/contract")
                && paths.contains_key("/v1/browser/adapter/capability")
                && paths.contains_key("/v1/browser/adapter/register")
                && paths.contains_key("/v1/browser/adapter/launch/plan")
                && paths.contains_key("/v1/browser/adapter/launch/claim")
                && paths.contains_key("/v1/browser/adapter/validate")
                && paths.contains_key("/v1/browser/adapter/completion/validate"))
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
        "BrowserProfilesResponse",
        "BrowserSandboxProfile",
        "BrowserIntegrationContract",
        "BrowserAdapterCapabilityIssueRequest",
        "BrowserAdapterCapabilityIssueResponse",
        "BrowserAdapterCompletionReport",
        "BrowserAdapterCompletionProofRequirement",
        "BrowserAdapterCompletionValidationDecision",
        "BrowserAdapterCompletionValidationResponse",
        "BrowserAdapterContract",
        "BrowserAdapterContractResponse",
        "BrowserAdapterConformanceCase",
        "BrowserAdapterConformanceExpectation",
        "BrowserAdapterConformanceProfile",
        "BrowserAdapterLaunchClaimDecision",
        "BrowserAdapterLaunchClaimRequest",
        "BrowserAdapterLaunchClaimResponse",
        "BrowserAdapterLaunchRequest",
        "BrowserAdapterLaunchPlanDecision",
        "BrowserAdapterLaunchPlanRequest",
        "BrowserAdapterLaunchPlanResponse",
        "BrowserAdapterManifestRequest",
        "BrowserAdapterManifestResponse",
        "BrowserAdapterRegistrationDecision",
        "BrowserAdapterRegistrationRequest",
        "BrowserAdapterRegistrationResponse",
        "BrowserAdapterValidationDecision",
        "BrowserSandboxLevel",
        "BrowserSandboxAvailability",
        "BrowserSandboxControl",
        "BrowserAdmissionRequest",
        "BrowserAdmissionResponse",
        "BrowserAdmissionDecision",
        "BrowserAdmissionGuardPlan",
        "BrowserSensitiveActivityMode",
        "BrowserSuppressionGuardPlan",
        "BrowserAdapterHandoff",
        "BrowserSessionActor",
        "BrowserSensitivity",
        "CapabilitiesResponse",
        "CapabilityLane",
        "CapabilityLimits",
    ] {
        assert!(schemas.contains_key(expected), "missing schema: {expected}");
    }
    let capabilities_200 = &value["paths"]["/v1/capabilities"]["get"]["responses"]["200"];
    let schema_ref = capabilities_200["content"]["application/json"]["schema"]["$ref"]
        .as_str()
        .ok_or("capabilities 200 should reference a schema")?;
    assert!(
        schema_ref.ends_with("/CapabilitiesResponse"),
        "got {schema_ref}"
    );
    assert!(
        schemas["CapabilitiesResponse"]["required"]
            .as_array()
            .is_some_and(|required| required.iter().any(|field| field == "browser_sandbox")),
        "capabilities schema should require browser_sandbox"
    );
    let manifest_properties = &schemas["BrowserAdapterManifestRequest"]["properties"];
    assert_eq!(manifest_properties["adapter_id"]["minLength"], 1);
    assert_eq!(manifest_properties["adapter_id"]["maxLength"], 128);
    assert_eq!(manifest_properties["supported_levels"]["maxItems"], 64);
    assert!(
        manifest_properties["guard_fields"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("without surrounding whitespace")),
        "guard_fields should describe runtime string-entry validation"
    );
    assert!(
        schemas["BrowserAdapterContractResponse"]["required"]
            .as_array()
            .is_some_and(
                |required| required.iter().any(|field| field == "conformance_profile")
                    && required.iter().any(|field| field == "required_levels")
                    && required.iter().any(|field| field == "launchable")
            ),
        "adapter contract response should require discovery metadata"
    );
    assert!(
        schemas["BrowserAdapterCapabilityIssueRequest"]["required"]
            .as_array()
            .is_some_and(|required| required.iter().any(|field| field == "actor")
                && required.iter().any(|field| field == "sensitivity")
                && !required.iter().any(|field| field == "adapter_id")
                && !required.iter().any(|field| field == "ttl_seconds")),
        "adapter capability request should require actor/sensitivity but keep adapter_id and ttl_seconds optional"
    );
    assert!(
        schemas["BrowserAdapterCapabilityIssueResponse"]["required"]
            .as_array()
            .is_some_and(
                |required| required.iter().any(|field| field == "same_user_capability")
                    && required.iter().any(|field| field == "expires_at")
                    && required.iter().any(|field| field == "ttl_seconds")
            ),
        "adapter capability response should require secret and expiry metadata"
    );
    assert_eq!(
        schemas["BrowserAdapterRegistrationRequest"]["properties"]["same_user_capability"]["maxLength"],
        256
    );
    assert_eq!(
        schemas["BrowserAdapterLaunchPlanRequest"]["properties"]["same_user_capability"]["maxLength"],
        256
    );
    assert!(
        schemas["BrowserAdapterRegistrationResponse"]["required"]
            .as_array()
            .is_some_and(
                |required| required.iter().any(|field| field == "registered")
                    && required
                        .iter()
                        .any(|field| field == "same_user_capability_bound")
                    && required.iter().any(|field| field == "manifest_validation")
            ),
        "adapter registration response should require fail-closed registration metadata"
    );
    assert!(
        schemas["BrowserAdapterLaunchPlanResponse"]["required"]
            .as_array()
            .is_some_and(
                |required| required.iter().any(|field| field == "launch_request")
                    && required
                        .iter()
                        .any(|field| field == "same_user_capability_bound")
                    && required
                        .iter()
                        .any(|field| field == "replay_protection_bound")
                    && required
                        .iter()
                        .any(|field| field == "adapter_contract_fields_complete")
                    && required
                        .iter()
                        .any(|field| field == "completion_validation_endpoint")
                    && required.iter().any(|field| field == "admission")
                    && required.iter().any(|field| field == "manifest_validation")
            ),
        "adapter launch plan response should require the fail-closed launch envelope"
    );
    assert!(
        schemas["BrowserAdmissionRequest"]["properties"]
            .as_object()
            .is_some_and(|properties| properties.contains_key("sensitive_activity_mode")),
        "browser admission request should expose sensitive_activity_mode"
    );
    assert!(
        schemas["BrowserAdmissionResponse"]["required"]
            .as_array()
            .is_some_and(|required| required
                .iter()
                .any(|field| field == "sensitive_activity_mode")),
        "browser admission response should require sensitive_activity_mode"
    );
    assert!(
        schemas["BrowserAdapterLaunchRequest"]["required"]
            .as_array()
            .is_some_and(|required| required
                .iter()
                .any(|field| field == "sensitive_activity_mode")),
        "launch request should require sensitive_activity_mode"
    );
    assert!(
        schemas["BrowserAdmissionGuardPlan"]["required"]
            .as_array()
            .is_some_and(|required| required.iter().any(|field| field == "suppression")),
        "guard plan should require the suppression sub-plan"
    );
    assert!(
        schemas["BrowserAdapterLaunchClaimResponse"]["required"]
            .as_array()
            .is_some_and(|required| required
                .iter()
                .any(|field| field == "launch_request_claim_bound")
                && required
                    .iter()
                    .any(|field| field == "launch_request_replay_detected")
                && required
                    .iter()
                    .any(|field| field == "canonical_request_matched")
                && required.iter().any(|field| field == "launchable")),
        "adapter launch claim response should expose replay claim state"
    );
    assert!(
        schemas["BrowserAdapterManifestResponse"]["required"]
            .as_array()
            .is_some_and(|required| required
                .iter()
                .any(|field| field == "endpoint_network_policy_bound")),
        "adapter manifest response should require endpoint_network_policy_bound"
    );
    assert!(
        schemas["BrowserAdapterManifestResponse"]["required"]
            .as_array()
            .is_some_and(|required| required.iter().any(|field| field == "conformance_profile")),
        "adapter manifest response should require conformance_profile"
    );
    assert!(
        schemas["BrowserAdapterHandoff"]["required"]
            .as_array()
            .is_some_and(|required| required
                .iter()
                .any(|field| field == "launch_request_template")
                && required
                    .iter()
                    .any(|field| field == "completion_proof_contract")),
        "adapter handoff should require the launch request and proof templates"
    );
    assert!(
        schemas["BrowserAdapterConformanceProfile"]["required"]
            .as_array()
            .is_some_and(|required| required
                .iter()
                .any(|field| field == "field_complete_launch_request")),
        "conformance profile should require a field-complete launch request fixture"
    );
    assert!(
        schemas["BrowserAdapterLaunchRequest"]["required"]
            .as_array()
            .is_some_and(
                |required| required.iter().any(|field| field == "request_id")
                    && required.iter().any(|field| field == "issued_at")
                    && required.iter().any(|field| field == "expires_at")
                    && required.iter().any(|field| field == "max_session_seconds")
                    && required.iter().any(|field| field == "adapter_id")
                    && required.iter().any(|field| field == "guard_plan")
                    && required
                        .iter()
                        .any(|field| field == "completion_proof_contract")
                    && required
                        .iter()
                        .any(|field| field == "completion_report_template")
                    && required
                        .iter()
                        .any(|field| field == "same_user_capability_required")
                    && required
                        .iter()
                        .any(|field| field == "endpoint_network_policy_binding_required")
                    && required
                        .iter()
                        .any(|field| field == "replay_protection_required")
            ),
        "launch request schema should expose the full adapter handoff envelope"
    );
    assert!(
        schemas["BrowserAdapterContract"]["required"]
            .as_array()
            .is_some_and(|required| required
                .iter()
                .any(|field| field == "completion_proof_contract")),
        "adapter contract should publish typed completion proof requirements"
    );
    assert!(
        schemas["BrowserAdapterCompletionReport"]["required"]
            .as_array()
            .is_some_and(
                |required| required.iter().any(|field| field == "request_id")
                    && required.iter().any(|field| field == "process_terminated")
                    && required.iter().any(|field| field == "proof_ids")
            ),
        "completion report schema should require teardown proof evidence"
    );
    assert_eq!(
        schemas["BrowserAdapterCompletionReport"]["properties"]["proof_ids"]["maxItems"],
        64
    );
    assert_eq!(
        schemas["BrowserAdapterCompletionReport"]["properties"]["contract_version"]["maxLength"],
        128
    );
    assert_eq!(
        schemas["BrowserAdapterCompletionReport"]["properties"]["notes"]["maxItems"],
        64
    );
    assert!(
        schemas["BrowserAdapterCompletionValidationResponse"]["required"]
            .as_array()
            .is_some_and(|required| required
                .iter()
                .any(|field| field == "report_shape_complete")
                && required
                    .iter()
                    .any(|field| field == "server_issued_launch_request")
                && required
                    .iter()
                    .any(|field| field == "launch_request_claimed")
                && required
                    .iter()
                    .any(|field| field == "launch_request_envelope_matched")
                && required
                    .iter()
                    .any(|field| field == "completion_report_template_matched")
                && required
                    .iter()
                    .any(|field| field == "completion_bound_to_claimed_launch")
                && required
                    .iter()
                    .any(|field| field == "verified_on_production_path")
                && required.iter().any(|field| field == "missing_proof_ids")
                && required
                    .iter()
                    .any(|field| field == "failed_evidence_fields")),
        "completion validation response should expose fail-closed proof state"
    );
    assert!(
        schemas["BrowserAdapterCompletionProofRequirement"]["required"]
            .as_array()
            .is_some_and(|required| required.iter().any(|field| field == "proof_id")
                && required.iter().any(|field| field == "evidence_field")
                && required.iter().any(|field| field == "required_invariant")),
        "completion proof requirement schema should be self-contained"
    );
    assert!(
        schemas["BrowserAdapterConformanceCase"]["required"]
            .as_array()
            .is_some_and(|required| required
                .iter()
                .any(|field| field == "expected_rest_error_code")
                && required
                    .iter()
                    .any(|field| field == "expected_mcp_error_code")
                && required.iter().any(|field| field == "expected_validation")),
        "conformance cases should preserve nullable expected fields"
    );
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
    assert!(tools.to_string().contains("get_browser_profiles"));
    assert!(tools.to_string().contains("admit_browser_session"));
    assert!(tools.to_string().contains("get_browser_adapter_contract"));
    assert!(tools.to_string().contains("register_browser_adapter"));
    assert!(tools.to_string().contains("validate_browser_adapter"));
    assert!(
        tools
            .to_string()
            .contains("validate_browser_adapter_completion")
    );
    assert!(
        !tools
            .to_string()
            .contains("issue_browser_adapter_capability")
    );
    assert!(!tools.to_string().contains("browser_adapter_launch_plan"));
    assert!(!tools.to_string().contains("plan_browser_adapter_launch"));
    assert!(!tools.to_string().contains("browser_adapter_launch_claim"));
    assert!(!tools.to_string().contains("claim_browser_adapter_launch"));
    assert!(!tools.to_string().contains("browser_adapter_capability"));
    let contract_tool = tools
        .as_array()
        .and_then(|tools| {
            tools
                .iter()
                .find(|tool| tool["name"] == "get_browser_adapter_contract")
        })
        .ok_or("get_browser_adapter_contract tool should be listed")?;
    assert_eq!(
        contract_tool["inputSchema"]["additionalProperties"],
        serde_json::json!(false)
    );
    let register_tool = tools
        .as_array()
        .and_then(|tools| {
            tools
                .iter()
                .find(|tool| tool["name"] == "register_browser_adapter")
        })
        .ok_or("register_browser_adapter tool should be listed")?;
    assert_eq!(
        register_tool["inputSchema"]["required"],
        serde_json::json!(["actor", "sensitivity", "manifest"])
    );
    assert!(
        register_tool["inputSchema"]["properties"]
            .as_object()
            .is_some_and(|properties| !properties.contains_key("same_user_capability"))
    );
    assert_eq!(
        register_tool["inputSchema"]["properties"]["manifest"]["additionalProperties"],
        serde_json::json!(false)
    );
    assert_eq!(
        register_tool["inputSchema"]["properties"]["manifest"]["properties"]["adapter_id"]["maxLength"],
        128
    );
    let validate_tool = tools
        .as_array()
        .and_then(|tools| {
            tools
                .iter()
                .find(|tool| tool["name"] == "validate_browser_adapter")
        })
        .ok_or("validate_browser_adapter tool should be listed")?;
    let schema = &validate_tool["inputSchema"]["properties"];
    assert_eq!(schema["adapter_id"]["minLength"], 1);
    assert_eq!(schema["adapter_id"]["maxLength"], 128);
    assert_eq!(schema["supported_controls"]["maxItems"], 64);
    assert_eq!(schema["guard_fields"]["items"]["minLength"], 1);
    assert!(
        schema["launch_endpoint"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("DNS, proxy, redirect, and retry")),
        "launch_endpoint schema should not overstate endpoint validation"
    );
    let completion_tool = tools
        .as_array()
        .and_then(|tools| {
            tools
                .iter()
                .find(|tool| tool["name"] == "validate_browser_adapter_completion")
        })
        .ok_or("validate_browser_adapter_completion tool should be listed")?;
    assert_eq!(
        completion_tool["inputSchema"]["additionalProperties"],
        serde_json::json!(false)
    );
    assert_eq!(
        completion_tool["inputSchema"]["required"],
        serde_json::json!([
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
        ])
    );
    let completion_schema = &completion_tool["inputSchema"]["properties"];
    assert_eq!(completion_schema["request_id"]["maxLength"], 128);
    assert_eq!(completion_schema["contract_version"]["maxLength"], 128);
    assert_eq!(completion_schema["proof_ids"]["maxItems"], 64);
    assert_eq!(completion_schema["proof_ids"]["items"]["minLength"], 1);
    assert_eq!(completion_schema["notes"]["maxItems"], 64);
    Ok(())
}

#[tokio::test]
async fn mcp_options_allows_aether_payment_headers_for_local_browser_origin()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/mcp")
                .header("origin", "http://localhost:3000")
                .header(
                    "access-control-request-headers",
                    format!("content-type, {AETHER_PAYMENT_HEADER}, {AETHER_PAYMENT_HASH_HEADER}"),
                )
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let headers = response.headers();
    assert_eq!(
        headers
            .get("access-control-allow-origin")
            .and_then(|value| value.to_str().ok()),
        Some("http://localhost:3000")
    );
    let allowed_headers = headers
        .get("access-control-allow-headers")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(allowed_headers.contains(AETHER_PAYMENT_HEADER));
    assert!(allowed_headers.contains(AETHER_PAYMENT_HASH_HEADER));
    let exposed_headers = headers
        .get("access-control-expose-headers")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(!exposed_headers.contains(AETHER_PAYMENT_HEADER));
    assert!(exposed_headers.contains(AETHER_PAYMENT_HASH_HEADER));
    assert!(
        headers
            .get("access-control-allow-methods")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|methods| methods.contains("OPTIONS") && methods.contains("POST"))
    );
    Ok(())
}

#[tokio::test]
async fn mcp_options_rejects_nonlocal_origin() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/mcp")
                .header("origin", "https://evil.example")
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
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
async fn mcp_get_capabilities_returns_structured_content() -> Result<(), Box<dyn std::error::Error>>
{
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": "get_capabilities", "arguments": {}}
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
    assert_eq!(result["isError"], serde_json::json!(false));
    assert_eq!(result["content"][0]["text"], "beatbox capabilities");
    assert_eq!(
        result["structuredContent"]["version"],
        env!("CARGO_PKG_VERSION")
    );
    assert!(result["structuredContent"]["lanes"].is_array());
    assert_eq!(
        result["structuredContent"]["browser_sandbox"]["runnable_browser_sessions"],
        false
    );
    assert_eq!(
        result["structuredContent"]["aether_payment"]["payment_header"],
        AETHER_PAYMENT_HEADER
    );
    assert_eq!(
        result["structuredContent"]["aether_payment"]["payment_hash_header"],
        AETHER_PAYMENT_HASH_HEADER
    );
    assert_eq!(
        result["structuredContent"]["aether_payment"]["require_hash_with_payment"],
        true
    );
    assert_eq!(
        result["structuredContent"]["aether_payment"]["echo_payment_payload"],
        false
    );
    Ok(())
}

#[tokio::test]
async fn mcp_tools_call_accepts_aether_payment_headers_without_echoing_payload()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": "get_capabilities", "arguments": {}}
    });
    let payment_payload = "sensitive-aether-payment-payload";
    let payment_hash = "0x1111111111111111111111111111111111111111111111111111111111111111";
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header("content-type", "application/json")
                .header(AETHER_PAYMENT_HEADER, payment_payload)
                .header(AETHER_PAYMENT_HASH_HEADER, payment_hash)
                .body(Body::from(request.to_string()))?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let raw = String::from_utf8(body.to_vec())?;
    assert!(!raw.contains(payment_payload));
    let value: serde_json::Value = serde_json::from_str(&raw)?;
    assert_eq!(
        value["result"]["_meta"]["aether_payment"]["payment_hash"],
        payment_hash
    );
    assert_eq!(
        value["result"]["_meta"]["aether_payment"]["payment_payload_echoed"],
        false
    );
    assert_eq!(value["result"]["isError"], serde_json::json!(false));
    Ok(())
}

#[tokio::test]
async fn mcp_tools_call_rejects_unhashed_aether_payment_header()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": "get_capabilities", "arguments": {}}
    });
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header("content-type", "application/json")
                .header(AETHER_PAYMENT_HEADER, "payment-without-hash")
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
            .is_some_and(|message| message.contains("must be supplied together"))
    );
    Ok(())
}

#[tokio::test]
async fn mcp_tools_call_rejects_duplicate_aether_payment_headers()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": "get_capabilities", "arguments": {}}
    });
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header("content-type", "application/json")
                .header(AETHER_PAYMENT_HEADER, "first-payment")
                .header(AETHER_PAYMENT_HEADER, "second-payment")
                .header(
                    AETHER_PAYMENT_HASH_HEADER,
                    "0x2222222222222222222222222222222222222222222222222222222222222222",
                )
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
            .is_some_and(|message| message.contains("must be supplied at most once"))
    );
    Ok(())
}

#[tokio::test]
async fn mcp_get_browser_profiles_returns_structured_content()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": "get_browser_profiles", "arguments": {}}
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
    assert_eq!(result["isError"], serde_json::json!(false));
    assert_eq!(
        result["content"][0]["text"],
        "beatbox browser sandbox profiles"
    );
    assert_eq!(
        result["structuredContent"]["runnable_browser_sessions"],
        false
    );
    assert_eq!(
        result["structuredContent"]["integration"]["selection_field"],
        "browser_sandbox_level"
    );
    assert_eq!(
        result["structuredContent"]["integration"]["adapter"]["launch_endpoint"],
        serde_json::Value::Null
    );
    assert_eq!(
        result["structuredContent"]["integration"]["adapter"]["status"],
        "planned"
    );
    assert!(
        result["structuredContent"]["integration"]["adapter"]["required_guard_fields"]
            .as_array()
            .is_some_and(|fields| fields
                .iter()
                .any(|field| field == "guard_plan.storage.teardown_proof_required"))
    );
    assert!(
        result["structuredContent"]["profiles"]
            .as_array()
            .is_some_and(|profiles| profiles
                .iter()
                .all(|profile| profile["availability"] != "available"))
    );
    Ok(())
}

#[tokio::test]
async fn mcp_get_browser_profiles_rejects_unknown_arguments()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "get_browser_profiles",
            "arguments": {"level": "os_isolated"}
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
            .is_some_and(|message| message.contains("level"))
    );
    Ok(())
}

#[tokio::test]
async fn mcp_admit_browser_session_returns_structured_rejection()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "admit_browser_session",
            "arguments": {
                "requested_level": "network_suppressed",
                "actor": "agent",
                "sensitivity": "sensitive",
                "sensitive_activity_mode": "network_suppressed",
                "target_origins": ["https://billing.example"],
                "credential_mode": "scoped_secrets",
                "artifact_mode": "explicit_downloads",
                "required_controls": ["egress_policy", "sealed_artifacts"],
                "allow_downgrade": true,
                "task_label": "review account"
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
    let result = &value["result"];
    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["content"][0]["text"],
        "beatbox browser admission decision"
    );
    assert_eq!(result["structuredContent"]["decision"], "rejected");
    assert_eq!(
        result["structuredContent"]["runnable_browser_sessions"],
        false
    );
    assert_eq!(
        result["structuredContent"]["selected_level"],
        serde_json::Value::Null
    );
    assert_eq!(
        result["structuredContent"]["target_origins"],
        serde_json::json!(["https://billing.example"])
    );
    assert_eq!(
        result["structuredContent"]["credential_mode"],
        serde_json::json!("scoped_secrets")
    );
    assert_eq!(
        result["structuredContent"]["artifact_mode"],
        serde_json::json!("explicit_downloads")
    );
    assert_eq!(
        result["structuredContent"]["sensitive_activity_mode"],
        serde_json::json!("network_suppressed")
    );
    assert_eq!(
        result["structuredContent"]["requested_controls"],
        serde_json::json!(["egress_policy", "sealed_artifacts"])
    );
    assert_eq!(
        result["structuredContent"]["missing_controls"],
        serde_json::json!(["sealed_artifacts"])
    );
    assert_eq!(
        result["structuredContent"]["level_satisfies_requested_controls"],
        false
    );
    assert_eq!(
        result["structuredContent"]["intent_warnings"],
        serde_json::json!([])
    );
    assert_eq!(
        result["structuredContent"]["guard_plan"]["network"]["allowed_origins"],
        serde_json::json!(["https://billing.example"])
    );
    assert_eq!(
        result["structuredContent"]["guard_plan"]["network"]["outbound_network_disabled_without_proxy"],
        true
    );
    assert_eq!(
        result["structuredContent"]["guard_plan"]["credentials"]["scoped_secret_channel_required"],
        true
    );
    assert_eq!(
        result["structuredContent"]["guard_plan"]["storage"]["explicit_artifact_allowlist_required"],
        true
    );
    assert_eq!(
        result["structuredContent"]["guard_plan"]["suppression"]["mode"],
        serde_json::json!("network_suppressed")
    );
    assert_eq!(
        result["structuredContent"]["guard_plan"]["suppression"]["suppress_unapproved_network"],
        true
    );
    assert_eq!(
        result["structuredContent"]["adapter_handoff"]["launchable"],
        false
    );
    assert_eq!(
        result["structuredContent"]["adapter_handoff"]["launch_endpoint"],
        serde_json::Value::Null
    );
    assert!(
        result["structuredContent"]["adapter_handoff"]["handoff_fields"]
            .as_array()
            .is_some_and(|fields| fields.iter().any(|field| field == "request_id")
                && fields.iter().any(|field| field == "adapter_id")
                && fields.iter().any(|field| field == "guard_plan"))
    );
    assert_eq!(
        result["structuredContent"]["adapter_handoff"]["launch_request_template"]["request_id"],
        "browser-admission-launch-template-v1"
    );
    assert_eq!(
        result["structuredContent"]["adapter_handoff"]["launch_request_template"]["adapter_id"],
        serde_json::Value::Null
    );
    assert_eq!(
        result["structuredContent"]["adapter_handoff"]["launch_request_template"]["target_origins"],
        serde_json::json!(["https://billing.example"])
    );
    assert_eq!(
        result["structuredContent"]["adapter_handoff"]["launch_request_template"]["sensitive_activity_mode"],
        serde_json::json!("network_suppressed")
    );
    assert_eq!(
        result["structuredContent"]["adapter_handoff"]["launch_request_template"]["guard_plan"]["network"]
            ["allowed_origins"],
        serde_json::json!(["https://billing.example"])
    );
    assert_eq!(
        result["structuredContent"]["adapter_handoff"]["launch_request_template"]["same_user_capability_required"],
        true
    );
    assert_eq!(
        result["structuredContent"]["adapter_handoff"]["launch_request_template"]["endpoint_network_policy_binding_required"],
        true
    );
    assert!(
        result["structuredContent"]["adapter_handoff"]["required_completion_proofs"]
            .as_array()
            .is_some_and(|proofs| proofs.iter().any(|proof| proof
                .as_str()
                .is_some_and(|proof| proof.contains("temporary profile directory"))))
    );
    assert!(
        result["structuredContent"]["adapter_handoff"]["completion_proof_contract"]
            .as_array()
            .is_some_and(|proofs| proofs
                .iter()
                .any(|proof| proof["proof_id"] == "temporary_profile_removed"
                    && proof["evidence_field"] == "temporary_profile_removed"))
    );
    assert_eq!(
        result["structuredContent"]["adapter_handoff"]["launch_request_template"]["completion_report_template"]
            ["request_id"],
        "browser-admission-launch-template-v1"
    );
    assert_eq!(
        result["structuredContent"]["adapter_handoff"]["launch_request_template"]["completion_report_template"]
            ["proof_ids"][0],
        "browser_process_terminated"
    );
    assert_eq!(result["structuredContent"]["downgrade_allowed"], true);
    assert!(
        result["structuredContent"]["reasons"]
            .as_array()
            .is_some_and(|reasons| reasons.iter().any(|reason| reason
                .as_str()
                .is_some_and(|reason| reason.contains("no weaker browser profile"))))
    );
    Ok(())
}

#[tokio::test]
async fn mcp_validate_browser_adapter_returns_structured_rejection()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "validate_browser_adapter",
            "arguments": complete_adapter_manifest()
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
    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["content"][0]["text"],
        "beatbox browser adapter validation"
    );
    assert_eq!(
        result["structuredContent"]["decision"],
        serde_json::json!("rejected")
    );
    assert_eq!(result["structuredContent"]["manifest_complete"], false);
    assert_eq!(result["structuredContent"]["launchable"], false);
    assert_eq!(
        result["structuredContent"]["trusted_for_sensitive_work"],
        false
    );
    assert_eq!(
        result["structuredContent"]["endpoint_network_policy_bound"],
        false
    );
    assert_eq!(
        result["structuredContent"]["launch_endpoint"],
        serde_json::json!("https://adapter.example/launch")
    );
    assert_eq!(
        result["structuredContent"]["conformance_profile"]["profile_version"],
        serde_json::json!("browser-adapter-conformance-v1")
    );
    assert_eq!(
        result["structuredContent"]["conformance_profile"]["field_complete_launch_request"]["adapter_id"],
        "tempo-conformance-adapter-v1"
    );
    assert_eq!(
        result["structuredContent"]["conformance_profile"]["field_complete_launch_request"]["same_user_capability_required"],
        true
    );
    assert_eq!(
        result["structuredContent"]["conformance_profile"]["field_complete_launch_request"]["endpoint_network_policy_binding_required"],
        true
    );
    assert!(
        result["structuredContent"]["conformance_profile"]["required_cases"]
            .as_array()
            .is_some_and(|cases| cases.iter().any(|case| case["name"]
                == "dns_rebinding_hostname_stays_incomplete"
                && case["expected_rest_status"] == StatusCode::OK.as_u16()
                && case["expected_mcp_error_code"].is_null()
                && case["expected_validation"]["endpoint_network_policy_bound"] == false))
    );
    assert!(
        result["structuredContent"]["conformance_profile"]["required_cases"]
            .as_array()
            .is_some_and(|cases| cases.iter().any(|case| case["name"]
                == "insecure_scheme_rejected_before_validation"
                && case["expected_rest_status"] == StatusCode::BAD_REQUEST.as_u16()
                && case["expected_rest_error_code"] == "invalid_browser_adapter_manifest"
                && case["expected_mcp_error_code"] == -32602
                && case["expected_validation"].is_null()))
    );
    assert!(
        result["structuredContent"]["conformance_profile"]["required_cases"]
            .as_array()
            .is_some_and(|cases| cases.iter().any(|case| case["name"]
                == "missing_required_level_reports_gap"
                && case["expected_validation"]["missing_levels"]
                    .as_array()
                    .is_some_and(|levels| levels.iter().any(|level| level == "os_isolated"))))
    );
    assert_eq!(
        result["structuredContent"]["missing_guard_fields"],
        serde_json::json!([])
    );
    assert!(
        result["structuredContent"]["adapter_contract"]["required_guard_fields"]
            .as_array()
            .is_some_and(|fields| fields
                .iter()
                .any(|field| field == "guard_plan.network.deny_metadata_endpoints"))
    );
    assert!(
        result["structuredContent"]["adapter_contract"]["completion_proof_contract"]
            .as_array()
            .is_some_and(|proofs| proofs.iter().any(|proof| proof["proof_id"]
                == "egress_log_sealed_or_discarded"
                && proof["evidence_field"] == "egress_log_sealed_or_discarded"))
    );
    assert_eq!(
        result["structuredContent"]["conformance_profile"]["field_complete_launch_request"]["completion_report_template"]
            ["adapter_id"],
        "tempo-conformance-adapter-v1"
    );
    Ok(())
}

#[tokio::test]
async fn mcp_validate_browser_adapter_completion_returns_structured_rejection()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "validate_browser_adapter_completion",
            "arguments": complete_adapter_completion_report()
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
    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["content"][0]["text"],
        "beatbox browser adapter completion validation"
    );
    assert_eq!(
        result["structuredContent"]["decision"],
        serde_json::json!("rejected")
    );
    assert_eq!(result["structuredContent"]["report_shape_complete"], true);
    assert_eq!(
        result["structuredContent"]["server_issued_launch_request"],
        false
    );
    assert_eq!(result["structuredContent"]["launch_request_claimed"], false);
    assert_eq!(
        result["structuredContent"]["launch_request_envelope_matched"],
        false
    );
    assert_eq!(
        result["structuredContent"]["completion_report_template_matched"],
        false
    );
    assert_eq!(
        result["structuredContent"]["completion_bound_to_claimed_launch"],
        false
    );
    assert_eq!(
        result["structuredContent"]["verified_on_production_path"],
        false
    );
    assert_eq!(
        result["structuredContent"]["trusted_for_sensitive_work"],
        false
    );
    assert_eq!(
        result["structuredContent"]["missing_proof_ids"],
        serde_json::json!([])
    );
    assert_eq!(
        result["structuredContent"]["failed_evidence_fields"],
        serde_json::json!([])
    );
    assert!(
        result["structuredContent"]["completion_proof_contract"]
            .as_array()
            .is_some_and(|proofs| proofs
                .iter()
                .any(|proof| proof["proof_id"] == "temporary_profile_removed"))
    );
    assert!(
        result["structuredContent"]["reasons"]
            .as_array()
            .is_some_and(|reasons| reasons.iter().any(|reason| reason
                .as_str()
                .is_some_and(|reason| reason.contains("not verified"))))
    );
    assert!(
        result["structuredContent"]["reasons"]
            .as_array()
            .is_some_and(|reasons| reasons.iter().any(|reason| reason
                .as_str()
                .is_some_and(|reason| reason.contains("shape-only"))))
    );
    Ok(())
}

#[tokio::test]
async fn mcp_get_browser_adapter_contract_returns_structured_content()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "get_browser_adapter_contract",
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
    assert_eq!(result["isError"], serde_json::json!(false));
    assert_eq!(
        result["content"][0]["text"],
        "beatbox browser adapter contract"
    );
    assert_eq!(result["structuredContent"]["launchable"], false);
    assert_eq!(
        result["structuredContent"]["trusted_for_sensitive_work"],
        false
    );
    assert_eq!(
        result["structuredContent"]["endpoint_network_policy_bound"],
        false
    );
    assert_eq!(
        result["structuredContent"]["adapter_contract"]["version"],
        serde_json::json!("browser-adapter-v1")
    );
    assert_eq!(
        result["structuredContent"]["adapter_contract"]["launch_endpoint"],
        serde_json::Value::Null
    );
    assert!(
        result["structuredContent"]["required_levels"]
            .as_array()
            .is_some_and(|levels| levels.iter().any(|level| level == "os_isolated")
                && levels.iter().any(|level| level == "remote_isolated"))
    );
    assert!(
        result["structuredContent"]["required_controls"]
            .as_array()
            .is_some_and(|controls| controls
                .iter()
                .any(|control| control == "local_network_block")
                && controls.iter().any(|control| control == "teardown_proof"))
    );
    assert_eq!(
        result["structuredContent"]["conformance_profile"]["profile_version"],
        serde_json::json!("browser-adapter-conformance-v1")
    );
    assert_eq!(
        result["structuredContent"]["conformance_profile"]["field_complete_launch_request"]["adapter_id"],
        "tempo-conformance-adapter-v1"
    );
    assert_eq!(
        result["structuredContent"]["conformance_profile"]["field_complete_launch_request"]["guard_plan"]
            ["network"]["allowed_origins"],
        serde_json::json!(["https://example.com"])
    );
    assert!(
        result["structuredContent"]["adapter_contract"]["completion_proof_contract"]
            .as_array()
            .is_some_and(|proofs| proofs.iter().any(|proof| proof["proof_id"]
                == "temporary_profile_removed"
                && proof["required_invariant"]
                    .as_str()
                    .is_some_and(|invariant| invariant.contains("profile directory"))))
    );
    assert_eq!(
        result["structuredContent"]["conformance_profile"]["field_complete_launch_request"]["completion_report_template"]
            ["proof_ids"][1],
        "temporary_profile_removed"
    );
    assert!(
        result["structuredContent"]["conformance_profile"]["required_cases"]
            .as_array()
            .is_some_and(|cases| cases
                .iter()
                .any(|case| case["name"] == "field_complete_manifest_stays_fail_closed"))
    );
    Ok(())
}

#[tokio::test]
async fn mcp_get_browser_adapter_contract_rejects_unknown_arguments()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "get_browser_adapter_contract",
            "arguments": {"adapter_id": "tempo-os-jail-v1"}
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
    assert_eq!(value["error"]["code"], serde_json::json!(-32602));
    assert!(
        value["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("does not accept argument"))
    );
    Ok(())
}

#[tokio::test]
async fn mcp_register_browser_adapter_returns_structured_rejection_without_capability_echo()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let mut registration = complete_adapter_registration();
    registration
        .as_object_mut()
        .ok_or("registration should be an object")?
        .remove("same_user_capability");
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "register_browser_adapter",
            "arguments": registration
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
    let raw = String::from_utf8(body.to_vec())?;
    assert!(!raw.contains("test-capability-fixture"));
    assert!(!raw.contains("same_user_capability\":"));
    let value: serde_json::Value = serde_json::from_str(&raw)?;
    let result = &value["result"];
    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["content"][0]["text"],
        "beatbox browser adapter registration preflight"
    );
    assert_eq!(result["structuredContent"]["decision"], "rejected");
    assert_eq!(
        result["structuredContent"]["adapter_id"],
        "tempo-os-jail-v1"
    );
    assert_eq!(result["structuredContent"]["actor"], "agent");
    assert_eq!(result["structuredContent"]["sensitivity"], "sensitive");
    assert_eq!(result["structuredContent"]["registered"], false);
    assert_eq!(result["structuredContent"]["launchable"], false);
    assert_eq!(
        result["structuredContent"]["same_user_capability_bound"],
        false
    );
    assert_eq!(
        result["structuredContent"]["manifest_validation"]["launchable"],
        false
    );
    assert_eq!(
        result["structuredContent"]["manifest_validation"]["endpoint_network_policy_bound"],
        false
    );
    assert!(
        result["structuredContent"]["required_next_steps"]
            .as_array()
            .is_some_and(|steps| steps
                .iter()
                .any(|step| step.as_str().is_some_and(|step| step
                    .contains("REST registration endpoint")
                    || step.contains("REST"))))
    );
    assert!(
        result["structuredContent"]["reasons"]
            .as_array()
            .is_some_and(|reasons| reasons.iter().any(|reason| reason
                .as_str()
                .is_some_and(|reason| reason.contains("model-visible transcripts"))))
    );
    Ok(())
}

#[tokio::test]
async fn mcp_register_browser_adapter_rejects_unknown_arguments()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let mut registration = complete_adapter_registration();
    registration["secret_note"] = json!("ignored");
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "register_browser_adapter",
            "arguments": registration
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
    assert_eq!(value["error"]["code"], serde_json::json!(-32602));
    assert!(
        value["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("does not accept argument"))
    );
    Ok(())
}

#[tokio::test]
async fn mcp_admit_browser_session_rejects_unsafe_target_origins()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "admit_browser_session",
            "arguments": {
                "requested_level": "network_suppressed",
                "actor": "agent",
                "sensitivity": "sensitive",
                "target_origins": ["https://example.com/path"]
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
            .is_some_and(|message| message.contains("scheme, host, and optional port"))
    );
    Ok(())
}

#[tokio::test]
async fn mcp_admit_browser_session_rejects_mistyped_arguments()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "admit_browser_session",
            "arguments": {
                "requested_level": "network_suppressed",
                "actor": "agent",
                "sensitivity": "sensitive",
                "allow_downgrade": "yes"
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
            .is_some_and(|message| message.contains("boolean"))
    );
    Ok(())
}

#[tokio::test]
async fn mcp_admit_browser_session_rejects_mistyped_controls()
-> Result<(), Box<dyn std::error::Error>> {
    let app = router(ServerConfig::new(BeatboxEngine::new()?));
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "admit_browser_session",
            "arguments": {
                "requested_level": "network_suppressed",
                "actor": "agent",
                "sensitivity": "sensitive",
                "required_controls": ["egress_policy", "ambient_cookies"]
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
            .is_some_and(|message| message.contains("ambient_cookies"))
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
