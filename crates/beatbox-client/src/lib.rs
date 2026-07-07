pub use beatbox_core::*;

use std::net::IpAddr;
use std::time::Duration;

use reqwest::{StatusCode, Url};
use thiserror::Error;

pub const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(65);

#[derive(Clone)]
pub struct Client {
    base_url: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

impl Client {
    /// Construct a client, returning an error if the base URL is not safe for
    /// secret-bearing requests or if the underlying HTTP client cannot be
    /// built. Prefer this over [`new`](Self::new) in library code that must not
    /// panic.
    ///
    /// HTTPS base URLs are accepted. Plain HTTP is accepted only for
    /// `localhost` or loopback addresses so local development does not train
    /// production callers to send `x-beatbox-api-key` over public plaintext
    /// origins. Credentials, query strings, and fragments are rejected.
    pub fn try_new(base_url: impl Into<String>) -> Result<Self, ClientError> {
        Ok(Self {
            base_url: validate_base_url(base_url.into())?,
            api_key: None,
            http: http_client_builder()
                .timeout(DEFAULT_HTTP_TIMEOUT)
                .build()?,
        })
    }

    /// Construct a client with the default configuration.
    ///
    /// Panics if the base URL is invalid, rejected by the client URL policy, or
    /// if the HTTP client cannot be built. Use [`try_new`](Self::try_new) for a
    /// non-panicking constructor.
    pub fn new(base_url: impl Into<String>) -> Self {
        match Self::try_new(base_url) {
            Ok(client) => client,
            Err(error) => panic!("beatbox client must construct: {error}"),
        }
    }

    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Result<Self, ClientError> {
        self.http = http_client_builder().timeout(timeout).build()?;
        Ok(self)
    }

    pub async fn health(&self) -> Result<serde_json::Value, ClientError> {
        let response = self
            .http
            .get(format!("{}/v1/health", self.base_url))
            .send()
            .await?;
        decode_response(response).await
    }

    pub async fn capabilities(&self) -> Result<serde_json::Value, ClientError> {
        let request = self.http.get(format!("{}/v1/capabilities", self.base_url));
        let response = self.authorize(request).send().await?;
        decode_response(response).await
    }

    pub async fn browser_profiles(&self) -> Result<BrowserProfilesResponse, ClientError> {
        let request = self
            .http
            .get(format!("{}/v1/browser/profiles", self.base_url));
        let response = self.authorize(request).send().await?;
        decode_response(response).await
    }

    pub async fn browser_admit(
        &self,
        request: &BrowserAdmissionRequest,
    ) -> Result<BrowserAdmissionResponse, ClientError> {
        let request_builder = self
            .http
            .post(format!("{}/v1/browser/admit", self.base_url))
            .json(request);
        let response = self.authorize(request_builder).send().await?;
        decode_response(response).await
    }

    pub async fn browser_adapter_contract(
        &self,
    ) -> Result<BrowserAdapterContractResponse, ClientError> {
        let request = self
            .http
            .get(format!("{}/v1/browser/adapter/contract", self.base_url));
        let response = self.authorize(request).send().await?;
        decode_response(response).await
    }

    pub async fn browser_adapter_capability(
        &self,
        request: &BrowserAdapterCapabilityIssueRequest,
    ) -> Result<BrowserAdapterCapabilityIssueResponse, ClientError> {
        let request_builder = self
            .http
            .post(format!("{}/v1/browser/adapter/capability", self.base_url))
            .json(request);
        let response = self.authorize(request_builder).send().await?;
        decode_response(response).await
    }

    pub async fn browser_adapter_register(
        &self,
        request: &BrowserAdapterRegistrationRequest,
    ) -> Result<BrowserAdapterRegistrationResponse, ClientError> {
        let request_builder = self
            .http
            .post(format!("{}/v1/browser/adapter/register", self.base_url))
            .json(request);
        let response = self.authorize(request_builder).send().await?;
        decode_response(response).await
    }

    pub async fn browser_adapter_launch_plan(
        &self,
        request: &BrowserAdapterLaunchPlanRequest,
    ) -> Result<BrowserAdapterLaunchPlanResponse, ClientError> {
        let request_builder = self
            .http
            .post(format!("{}/v1/browser/adapter/launch/plan", self.base_url))
            .json(request);
        let response = self.authorize(request_builder).send().await?;
        decode_response(response).await
    }

    pub async fn browser_adapter_launch_claim(
        &self,
        request: &BrowserAdapterLaunchClaimRequest,
    ) -> Result<BrowserAdapterLaunchClaimResponse, ClientError> {
        let request_builder = self
            .http
            .post(format!("{}/v1/browser/adapter/launch/claim", self.base_url))
            .json(request);
        let response = self.authorize(request_builder).send().await?;
        decode_response(response).await
    }

    pub async fn browser_adapter_validate(
        &self,
        request: &BrowserAdapterManifestRequest,
    ) -> Result<BrowserAdapterManifestResponse, ClientError> {
        let request_builder = self
            .http
            .post(format!("{}/v1/browser/adapter/validate", self.base_url))
            .json(request);
        let response = self.authorize(request_builder).send().await?;
        decode_response(response).await
    }

    pub async fn browser_adapter_completion_validate(
        &self,
        request: &BrowserAdapterCompletionReport,
    ) -> Result<BrowserAdapterCompletionValidationResponse, ClientError> {
        let request_builder = self
            .http
            .post(format!(
                "{}/v1/browser/adapter/completion/validate",
                self.base_url
            ))
            .json(request);
        let response = self.authorize(request_builder).send().await?;
        decode_response(response).await
    }

    pub async fn execute(&self, request: &ExecuteRequest) -> Result<ExecutionResult, ClientError> {
        let request_builder = self
            .http
            .post(format!("{}/v1/execute", self.base_url))
            .json(request);
        let response = self.authorize(request_builder).send().await?;
        decode_response(response).await
    }

    pub async fn create_job(
        &self,
        request: &ExecuteRequest,
    ) -> Result<CreateJobResponse, ClientError> {
        let request_builder = self
            .http
            .post(format!("{}/v1/jobs", self.base_url))
            .json(request);
        let response = self.authorize(request_builder).send().await?;
        decode_response(response).await
    }

    pub async fn get_job(&self, job_id: &str) -> Result<JobRecord, ClientError> {
        let request = self.http.get(self.job_url(job_id)?);
        let response = self.authorize(request).send().await?;
        decode_response(response).await
    }

    pub async fn cancel_job(&self, job_id: &str) -> Result<(), ClientError> {
        let request = self.http.delete(self.job_url(job_id)?);
        let response = self.authorize(request).send().await?;
        decode_empty_response(response).await
    }

    /// Build the `/v1/jobs/{id}` URL with `job_id` percent-encoded as a single
    /// path segment. Interpolating the id directly would let an id containing
    /// `/`, `?`, or `#` retarget the request (e.g. `../execute`, `x?k=v`).
    /// Empty and dot-segment ids (`""`, `.`, `..`) are rejected outright: the URL
    /// crate treats `.`/`..` as relative navigation rather than a literal
    /// segment, so encoding alone would not keep them under `/v1/jobs/`.
    fn job_url(&self, job_id: &str) -> Result<Url, ClientError> {
        if job_id.is_empty() || job_id == "." || job_id == ".." {
            return Err(ClientError::InvalidUrl(format!(
                "invalid job id: {job_id:?}"
            )));
        }
        let mut url = Url::parse(&format!("{}/v1/jobs/", self.base_url))
            .map_err(|error| ClientError::InvalidUrl(error.to_string()))?;
        url.path_segments_mut()
            .map_err(|()| ClientError::InvalidUrl("base URL cannot be a base".to_string()))?
            .pop_if_empty()
            .push(job_id);
        Ok(url)
    }

    pub async fn openapi(&self) -> Result<serde_json::Value, ClientError> {
        let response = self
            .http
            .get(format!("{}/openapi.json", self.base_url))
            .send()
            .await?;
        decode_response(response).await
    }

    fn authorize(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.api_key {
            Some(api_key) => request.header("x-beatbox-api-key", api_key),
            None => request,
        }
    }
}

fn http_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().redirect(reqwest::redirect::Policy::none())
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    #[error("beatbox API returned {status}: {message}")]
    Api {
        status: StatusCode,
        code: String,
        message: String,
    },
}

async fn decode_response<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, ClientError> {
    let status = response.status();
    if status.is_success() {
        return response.json::<T>().await.map_err(ClientError::from);
    }
    let error = response.json::<ErrorResponse>().await;
    match error {
        Ok(error) => Err(ClientError::Api {
            status,
            code: error.error.code,
            message: error.error.message,
        }),
        Err(source) => Err(ClientError::Api {
            status,
            code: "http_error".to_string(),
            message: source.to_string(),
        }),
    }
}

async fn decode_empty_response(response: reqwest::Response) -> Result<(), ClientError> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let error = response.json::<ErrorResponse>().await;
    match error {
        Ok(error) => Err(ClientError::Api {
            status,
            code: error.error.code,
            message: error.error.message,
        }),
        Err(source) => Err(ClientError::Api {
            status,
            code: "http_error".to_string(),
            message: source.to_string(),
        }),
    }
}

fn trim_base_url(mut value: String) -> String {
    while value.ends_with('/') {
        value.pop();
    }
    value
}

fn validate_base_url(value: String) -> Result<String, ClientError> {
    let value = trim_base_url(value);
    let url = Url::parse(&value).map_err(|error| ClientError::InvalidUrl(error.to_string()))?;
    if url.cannot_be_a_base() {
        return Err(ClientError::InvalidUrl(
            "base URL must be an absolute HTTP(S) origin".to_string(),
        ));
    }
    if url.username() != "" || url.password().is_some() || base_url_authority_contains_at(&value) {
        return Err(ClientError::InvalidUrl(
            "base URL must not contain credentials".to_string(),
        ));
    }
    if url.query().is_some() {
        return Err(ClientError::InvalidUrl(
            "base URL must not contain a query".to_string(),
        ));
    }
    if url.fragment().is_some() {
        return Err(ClientError::InvalidUrl(
            "base URL must not contain a fragment".to_string(),
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| ClientError::InvalidUrl("base URL must include a host".to_string()))?;
    match url.scheme() {
        "https" => {}
        "http" if is_loopback_base_url_host(host) => {}
        "http" => {
            return Err(ClientError::InvalidUrl(
                "http base URL is allowed only for localhost or loopback addresses".to_string(),
            ));
        }
        _ => {
            return Err(ClientError::InvalidUrl(
                "base URL must use http or https".to_string(),
            ));
        }
    }
    Ok(trim_base_url(url.to_string()))
}

fn is_loopback_base_url_host(host: &str) -> bool {
    let host = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn base_url_authority_contains_at(value: &str) -> bool {
    let Some((_, rest)) = value.split_once("://") else {
        return false;
    };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    rest[..authority_end].contains('@')
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn try_new_builds_without_panicking() -> Result<(), Box<dyn std::error::Error>> {
        let client = Client::try_new("http://localhost:7300")?;
        assert_eq!(client.base_url, "http://localhost:7300");
        Ok(())
    }

    #[test]
    fn try_new_rejects_secret_unsafe_base_urls() -> Result<(), Box<dyn std::error::Error>> {
        let https = Client::try_new("https://beatbox.example/api/")?;
        assert_eq!(https.base_url, "https://beatbox.example/api");

        let ipv4_loopback = Client::try_new("http://127.0.0.1:7300/")?;
        assert_eq!(ipv4_loopback.base_url, "http://127.0.0.1:7300");

        let ipv6_loopback = Client::try_new("http://[::1]:7300/")?;
        assert_eq!(ipv6_loopback.base_url, "http://[::1]:7300");

        let uppercase_localhost = Client::try_new("http://LOCALHOST:7300/")?;
        assert_eq!(uppercase_localhost.base_url, "http://localhost:7300");

        let shorthand_loopback = Client::try_new("http://127.1:7300/")?;
        assert_eq!(shorthand_loopback.base_url, "http://127.0.0.1:7300");

        for base_url in [
            "http://beatbox.example:7300",
            "https://user:pass@beatbox.example",
            "https://user@beatbox.example",
            "https://@beatbox.example",
            "https://beatbox.example?api_key=hidden",
            "https://beatbox.example/#fragment",
            "file:///tmp/beatbox.sock",
            "/v1",
        ] {
            assert!(
                matches!(Client::try_new(base_url), Err(ClientError::InvalidUrl(_))),
                "expected {base_url:?} to be rejected"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn base_url_path_prefix_is_preserved_on_secret_bearing_requests()
    -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let (request_tx, request_rx) = mpsc::channel();
        let server = std::thread::spawn(move || -> std::io::Result<()> {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(1)))?;
            let mut buffer = [0_u8; 4096];
            let bytes = stream.read(&mut buffer)?;
            let request = String::from_utf8_lossy(&buffer[..bytes]).to_string();
            request_tx
                .send(request)
                .map_err(|_| std::io::Error::other("request receiver dropped"))?;
            let body = "{}";
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes())?;
            Ok(())
        });

        let client = Client::new(format!("http://{addr}/api/")).with_api_key("secret");
        let _capabilities = client.capabilities().await?;

        match server.join() {
            Ok(result) => result?,
            Err(_) => return Err("capabilities prefix test server panicked".into()),
        }
        let request = request_rx.recv_timeout(Duration::from_secs(1))?;
        assert!(request.starts_with("GET /api/v1/capabilities "));
        assert!(request.contains("x-beatbox-api-key: secret"));
        Ok(())
    }

    #[test]
    fn job_url_percent_encodes_untrusted_ids() -> Result<(), Box<dyn std::error::Error>> {
        let client = Client::new("http://localhost:7300");

        // A normal server-issued UUID passes through unchanged.
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        let url = client.job_url(uuid)?;
        assert_eq!(
            url.as_str(),
            format!("http://localhost:7300/v1/jobs/{uuid}")
        );

        // `../execute` must not climb out of /v1/jobs/ — the `/` is encoded so it
        // stays a single segment (three literal slashes: /v1 /jobs /<id>).
        let url = client.job_url("../execute")?;
        assert_eq!(url.query(), None);
        assert!(url.path().starts_with("/v1/jobs/"));
        assert_eq!(url.path().matches('/').count(), 3);

        // `x?k=v` must not smuggle a query string.
        let url = client.job_url("x?k=v")?;
        assert_eq!(url.query(), None);
        assert_eq!(url.path().matches('/').count(), 3);

        // Bare dot-segments and empty ids are rejected (url treats `.`/`..` as
        // relative navigation, so they could otherwise reach /v1/jobs).
        assert!(client.job_url("").is_err());
        assert!(client.job_url(".").is_err());
        assert!(client.job_url("..").is_err());

        // A slash-bearing id stays one encoded segment even with dots present.
        let url = client.job_url("a/b/..")?;
        assert_eq!(url.path().matches('/').count(), 3);
        Ok(())
    }

    #[tokio::test]
    async fn browser_admit_posts_authenticated_json_preflight()
    -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let (request_tx, request_rx) = mpsc::channel();
        let server = std::thread::spawn(move || -> std::io::Result<()> {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(1)))?;
            let mut buffer = [0_u8; 4096];
            let bytes = stream.read(&mut buffer)?;
            let request = String::from_utf8_lossy(&buffer[..bytes]).to_string();
            request_tx
                .send(request)
                .map_err(|_| std::io::Error::other("request receiver dropped"))?;
            let body = r#"{"decision":"rejected","runnable_browser_sessions":false,"requested_level":"os_isolated","selected_level":null,"actor":"agent","sensitivity":"sensitive","target_origins":["https://example.com"],"credential_mode":"no_credentials","artifact_mode":"discard","requested_controls":["egress_policy","remote_worker_isolation"],"requested_profile_controls":["fresh_profile","no_ambient_credentials","egress_policy","local_network_block","os_process_isolation","teardown_proof"],"missing_controls":["remote_worker_isolation"],"level_satisfies_requested_controls":false,"intent_warnings":[],"guard_plan":{"network":{"allowed_origins":["https://example.com"],"deny_private_networks":true,"deny_localhost":true,"deny_metadata_endpoints":true,"require_dns_rebinding_protection":true,"require_redirect_revalidation":true,"require_proxy_enforcement":true,"outbound_network_disabled_without_proxy":true},"credentials":{"mode":"no_credentials","ambient_credentials_allowed":false,"user_mediation_required":false,"scoped_secret_channel_required":false},"storage":{"mode":"discard","plaintext_persistence_allowed":false,"explicit_artifact_allowlist_required":false,"encryption_required_for_persistence":false,"teardown_proof_required":true},"required_runtime_guards":["browser launcher bound to the selected sandbox profile","production-path admission check before launch","teardown proof before reporting session completion","fresh profile directory with no host browser state","deny-by-default egress proxy that revalidates DNS, redirects, and final socket targets","loopback, LAN, shared, link-local, and metadata address block","OS jail or microVM boundary around the browser process"]},"adapter_handoff":{"contract_version":"browser-adapter-v1","launch_endpoint":null,"launchable":false,"handoff_fields":["requested_level","actor","sensitivity","target_origins","credential_mode","artifact_mode","requested_controls","guard_plan"],"required_completion_proofs":["browser process exited or was killed","temporary profile directory removed","plaintext artifacts outside the explicit allowlist removed","egress proxy log sealed or discarded according to artifact_mode"],"unavailable_reason":"no browser adapter launch endpoint is implemented by this daemon"},"downgrade_allowed":false,"reasons":["no runnable browser sandbox"],"required_next_steps":["implement a browser launcher"],"profiles_endpoint":"/v1/browser/profiles"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes())?;
            Ok(())
        });

        let client = Client::new(format!("http://{addr}")).with_api_key("secret");
        let decision = client
            .browser_admit(&BrowserAdmissionRequest {
                requested_level: BrowserSandboxLevel::OsIsolated,
                actor: BrowserSessionActor::Agent,
                sensitivity: BrowserSensitivity::Sensitive,
                sensitive_activity_mode:
                    beatbox_core::BrowserSensitiveActivityMode::NetworkSuppressed,
                target_origins: vec!["https://example.com".to_string()],
                credential_mode: BrowserCredentialMode::NoCredentials,
                artifact_mode: BrowserArtifactMode::Discard,
                required_controls: vec![
                    BrowserSandboxControl::EgressPolicy,
                    BrowserSandboxControl::RemoteWorkerIsolation,
                ],
                allow_downgrade: false,
                task_label: None,
            })
            .await?;

        match server.join() {
            Ok(result) => result?,
            Err(_) => return Err("admission test server panicked".into()),
        }
        let request = request_rx.recv_timeout(Duration::from_secs(1))?;
        assert!(request.starts_with("POST /v1/browser/admit "));
        assert!(request.contains("x-beatbox-api-key: secret"));
        assert!(request.contains("content-type: application/json"));
        assert!(request.contains(r#""requested_level":"os_isolated""#));
        assert!(request.contains(r#""actor":"agent""#));
        assert!(request.contains(r#""sensitivity":"sensitive""#));
        assert!(request.contains(r#""sensitive_activity_mode":"network_suppressed""#));
        assert!(request.contains(r#""target_origins":["https://example.com"]"#));
        assert!(request.contains(r#""credential_mode":"no_credentials""#));
        assert!(request.contains(r#""artifact_mode":"discard""#));
        assert!(
            request.contains(r#""required_controls":["egress_policy","remote_worker_isolation"]"#)
        );
        assert_eq!(decision.decision, BrowserAdmissionDecision::Rejected);
        assert_eq!(decision.selected_level, None);
        assert_eq!(
            decision.missing_controls,
            vec![BrowserSandboxControl::RemoteWorkerIsolation]
        );
        assert_eq!(decision.target_origins, vec!["https://example.com"]);
        assert_eq!(
            decision.credential_mode,
            BrowserCredentialMode::NoCredentials
        );
        assert_eq!(decision.artifact_mode, BrowserArtifactMode::Discard);
        assert!(decision.intent_warnings.is_empty());
        assert_eq!(
            decision.guard_plan.network.allowed_origins,
            vec!["https://example.com"]
        );
        assert!(decision.guard_plan.network.require_proxy_enforcement);
        assert!(
            decision
                .guard_plan
                .required_runtime_guards
                .iter()
                .any(|guard| guard.contains("final socket targets"))
        );
        assert!(
            decision
                .guard_plan
                .required_runtime_guards
                .iter()
                .any(|guard| guard.contains("OS jail"))
        );
        assert!(!decision.adapter_handoff.launchable);
        assert_eq!(decision.adapter_handoff.launch_endpoint, None);
        assert!(
            decision
                .adapter_handoff
                .handoff_fields
                .iter()
                .any(|field| field == "guard_plan")
        );
        Ok(())
    }

    #[tokio::test]
    async fn browser_adapter_contract_gets_authenticated_json()
    -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let (request_tx, request_rx) = mpsc::channel();
        let server = std::thread::spawn(move || -> std::io::Result<()> {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(1)))?;
            let mut buffer = [0_u8; 4096];
            let bytes = stream.read(&mut buffer)?;
            let request = String::from_utf8_lossy(&buffer[..bytes]).to_string();
            request_tx
                .send(request)
                .map_err(|_| std::io::Error::other("request receiver dropped"))?;
            let body = r#"{"adapter_contract":{"version":"browser-adapter-v1","status":"planned","launch_endpoint":null,"handoff_fields":["guard_plan"],"required_guard_fields":["guard_plan.network.deny_metadata_endpoints"],"required_completion_proofs":["temporary profile directory removed"],"unavailable_reason":"no browser adapter launch endpoint is implemented by this daemon"},"conformance_profile":{"profile_version":"browser-adapter-conformance-v1","field_complete_manifest":{"adapter_id":"tempo-conformance-adapter-v1","contract_version":"browser-adapter-v1","launch_endpoint":"https://adapter.example/launch","supported_levels":["os_isolated"],"supported_controls":["os_process_isolation"],"guard_fields":["guard_plan.network.deny_metadata_endpoints"],"completion_proofs":["temporary profile directory removed"]},"field_complete_expectation":{"decision":"rejected","manifest_complete":false,"launchable":false,"trusted_for_sensitive_work":false,"endpoint_network_policy_bound":false,"missing_levels":[],"missing_controls":[],"missing_guard_fields":[],"missing_completion_proofs":[]},"required_cases":[],"notes":["not a launch grant"]},"required_levels":["os_isolated"],"required_controls":["os_process_isolation"],"launchable":false,"trusted_for_sensitive_work":false,"endpoint_network_policy_bound":false,"notes":["not adapter registration"]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes())?;
            Ok(())
        });

        let client = Client::new(format!("http://{addr}")).with_api_key("secret");
        let contract = client.browser_adapter_contract().await?;

        match server.join() {
            Ok(result) => result?,
            Err(_) => return Err("adapter contract test server panicked".into()),
        }
        let request = request_rx.recv_timeout(Duration::from_secs(1))?;
        assert!(request.starts_with("GET /v1/browser/adapter/contract "));
        assert!(request.contains("x-beatbox-api-key: secret"));
        assert!(!request.contains("content-type: application/json"));
        assert_eq!(contract.adapter_contract.version, "browser-adapter-v1");
        assert!(!contract.launchable);
        assert!(!contract.trusted_for_sensitive_work);
        assert!(!contract.endpoint_network_policy_bound);
        assert_eq!(
            contract.conformance_profile.profile_version,
            "browser-adapter-conformance-v1"
        );
        Ok(())
    }

    #[tokio::test]
    async fn browser_adapter_capability_posts_authenticated_json()
    -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let (request_tx, request_rx) = mpsc::channel();
        let server = std::thread::spawn(move || -> std::io::Result<()> {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(1)))?;
            let mut buffer = [0_u8; 4096];
            let bytes = stream.read(&mut buffer)?;
            let request = String::from_utf8_lossy(&buffer[..bytes]).to_string();
            request_tx
                .send(request)
                .map_err(|_| std::io::Error::other("request receiver dropped"))?;
            let body = r#"{"same_user_capability":"bbx-browser-adapter-cap-v1.test.fixture","expires_at":"2026-07-06T20:00:00Z","ttl_seconds":60,"actor":"agent","sensitivity":"sensitive","sensitive_activity_mode":null,"adapter_id":"tempo-os-jail-v1","registration_endpoint":"/v1/browser/adapter/register","notes":["keep it out of logs"]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes())?;
            Ok(())
        });

        let client = Client::new(format!("http://{addr}")).with_api_key("secret");
        let issued = client
            .browser_adapter_capability(&BrowserAdapterCapabilityIssueRequest {
                actor: BrowserSessionActor::Agent,
                sensitivity: BrowserSensitivity::Sensitive,
                sensitive_activity_mode: None,
                adapter_id: Some("tempo-os-jail-v1".to_string()),
                ttl_seconds: Some(60),
            })
            .await?;

        match server.join() {
            Ok(result) => result?,
            Err(_) => return Err("adapter capability test server panicked".into()),
        }
        let request = request_rx.recv_timeout(Duration::from_secs(1))?;
        assert!(request.starts_with("POST /v1/browser/adapter/capability "));
        assert!(request.contains("x-beatbox-api-key: secret"));
        assert!(request.contains("content-type: application/json"));
        assert!(request.contains(r#""adapter_id":"tempo-os-jail-v1""#));
        assert!(request.contains(r#""sensitive_activity_mode":null"#));
        assert!(request.contains(r#""ttl_seconds":60"#));
        assert_eq!(
            issued.same_user_capability,
            "bbx-browser-adapter-cap-v1.test.fixture"
        );
        assert_eq!(issued.ttl_seconds, 60);
        assert_eq!(issued.sensitive_activity_mode, None);
        assert_eq!(issued.adapter_id.as_deref(), Some("tempo-os-jail-v1"));
        Ok(())
    }

    #[tokio::test]
    async fn browser_adapter_register_posts_authenticated_json()
    -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let (request_tx, request_rx) = mpsc::channel();
        let server = std::thread::spawn(move || -> std::io::Result<()> {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(1)))?;
            let mut buffer = [0_u8; 8192];
            let bytes = stream.read(&mut buffer)?;
            let request = String::from_utf8_lossy(&buffer[..bytes]).to_string();
            request_tx
                .send(request)
                .map_err(|_| std::io::Error::other("request receiver dropped"))?;
            let body = r#"{"decision":"rejected","adapter_id":"tempo-os-jail-v1","actor":"agent","sensitivity":"sensitive","registered":false,"launchable":false,"trusted_for_sensitive_work":false,"endpoint_network_policy_bound":false,"same_user_capability_bound":false,"manifest_validation":{"decision":"rejected","manifest_complete":false,"launchable":false,"trusted_for_sensitive_work":false,"adapter_id":"tempo-os-jail-v1","launch_endpoint":"https://adapter.example/launch","endpoint_network_policy_bound":false,"missing_levels":[],"missing_controls":[],"missing_guard_fields":[],"missing_completion_proofs":[],"reasons":["validation metadata only"],"required_next_steps":["implement registration"],"adapter_contract":{"version":"browser-adapter-v1","status":"planned","launch_endpoint":null,"handoff_fields":["guard_plan"],"required_guard_fields":["guard_plan.network.deny_metadata_endpoints"],"required_completion_proofs":["temporary profile directory removed"],"unavailable_reason":"no browser adapter launch endpoint is implemented by this daemon"},"conformance_profile":{"profile_version":"browser-adapter-conformance-v1","field_complete_manifest":{"adapter_id":"tempo-conformance-adapter-v1","contract_version":"browser-adapter-v1","launch_endpoint":"https://adapter.example/launch","supported_levels":["os_isolated"],"supported_controls":["os_process_isolation"],"guard_fields":["guard_plan.network.deny_metadata_endpoints"],"completion_proofs":["temporary profile directory removed"]},"field_complete_expectation":{"decision":"rejected","manifest_complete":false,"launchable":false,"trusted_for_sensitive_work":false,"endpoint_network_policy_bound":false,"missing_levels":[],"missing_controls":[],"missing_guard_fields":[],"missing_completion_proofs":[]},"required_cases":[],"notes":["not a launch grant"]}},"reasons":["does not persist or trust adapters yet"],"required_next_steps":["issue a same-user capability"]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes())?;
            Ok(())
        });

        let client = Client::new(format!("http://{addr}")).with_api_key("secret");
        let registration = client
            .browser_adapter_register(&BrowserAdapterRegistrationRequest {
                actor: BrowserSessionActor::Agent,
                sensitivity: BrowserSensitivity::Sensitive,
                same_user_capability: "test-capability-fixture".to_string(),
                manifest: BrowserAdapterManifestRequest {
                    adapter_id: "tempo-os-jail-v1".to_string(),
                    contract_version: "browser-adapter-v1".to_string(),
                    launch_endpoint: Some("https://adapter.example/launch".to_string()),
                    supported_levels: vec![BrowserSandboxLevel::OsIsolated],
                    supported_controls: vec![BrowserSandboxControl::OsProcessIsolation],
                    guard_fields: vec!["guard_plan.network.deny_metadata_endpoints".to_string()],
                    completion_proofs: vec!["temporary profile directory removed".to_string()],
                },
            })
            .await?;

        match server.join() {
            Ok(result) => result?,
            Err(_) => return Err("adapter registration test server panicked".into()),
        }
        let request = request_rx.recv_timeout(Duration::from_secs(1))?;
        assert!(request.starts_with("POST /v1/browser/adapter/register "));
        assert!(request.contains("x-beatbox-api-key: secret"));
        assert!(request.contains("content-type: application/json"));
        assert!(request.contains(r#""same_user_capability":"test-capability-fixture""#));
        assert!(request.contains(r#""manifest":{"adapter_id":"tempo-os-jail-v1""#));
        assert!(!registration.registered);
        assert!(!registration.launchable);
        assert!(!registration.same_user_capability_bound);
        assert!(!registration.manifest_validation.launchable);
        Ok(())
    }

    #[tokio::test]
    async fn browser_adapter_validate_posts_authenticated_json()
    -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let (request_tx, request_rx) = mpsc::channel();
        let server = std::thread::spawn(move || -> std::io::Result<()> {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(1)))?;
            let mut buffer = [0_u8; 4096];
            let bytes = stream.read(&mut buffer)?;
            let request = String::from_utf8_lossy(&buffer[..bytes]).to_string();
            request_tx
                .send(request)
                .map_err(|_| std::io::Error::other("request receiver dropped"))?;
            let body = r#"{"decision":"rejected","manifest_complete":false,"launchable":false,"trusted_for_sensitive_work":false,"adapter_id":"tempo-os-jail-v1","launch_endpoint":"https://adapter.example/launch","endpoint_network_policy_bound":false,"missing_levels":[],"missing_controls":[],"missing_guard_fields":[],"missing_completion_proofs":[],"reasons":["no trusted adapter registration, endpoint binding, or launch path is implemented"],"required_next_steps":["implement authenticated adapter registration"],"adapter_contract":{"version":"browser-adapter-v1","status":"planned","launch_endpoint":null,"handoff_fields":["guard_plan"],"required_guard_fields":["guard_plan.network.deny_metadata_endpoints"],"required_completion_proofs":["temporary profile directory removed"],"unavailable_reason":"no browser adapter launch endpoint is implemented by this daemon"},"conformance_profile":{"profile_version":"browser-adapter-conformance-v1","field_complete_manifest":{"adapter_id":"tempo-conformance-adapter-v1","contract_version":"browser-adapter-v1","launch_endpoint":"https://adapter.example/launch","supported_levels":["os_isolated"],"supported_controls":["os_process_isolation"],"guard_fields":["guard_plan.network.deny_metadata_endpoints"],"completion_proofs":["temporary profile directory removed"]},"field_complete_expectation":{"decision":"rejected","manifest_complete":false,"launchable":false,"trusted_for_sensitive_work":false,"endpoint_network_policy_bound":false,"missing_levels":[],"missing_controls":[],"missing_guard_fields":[],"missing_completion_proofs":[]},"required_cases":[{"name":"insecure_scheme_rejected_before_validation","manifest":{"adapter_id":"tempo-conformance-adapter-v1","contract_version":"browser-adapter-v1","launch_endpoint":"http://adapter.example/launch","supported_levels":["os_isolated"],"supported_controls":["os_process_isolation"],"guard_fields":["guard_plan.network.deny_metadata_endpoints"],"completion_proofs":["temporary profile directory removed"]},"expected_rest_status":400,"expected_rest_error_code":"invalid_browser_adapter_manifest","expected_mcp_error_code":-32602,"expected_mcp_error_message_contains":["must use https"],"expected_validation":null,"notes":["parser failure"]}],"notes":["not a launch grant"]}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes())?;
            Ok(())
        });

        let client = Client::new(format!("http://{addr}")).with_api_key("secret");
        let validation = client
            .browser_adapter_validate(&BrowserAdapterManifestRequest {
                adapter_id: "tempo-os-jail-v1".to_string(),
                contract_version: "browser-adapter-v1".to_string(),
                launch_endpoint: Some("https://adapter.example/launch".to_string()),
                supported_levels: vec![BrowserSandboxLevel::OsIsolated],
                supported_controls: vec![BrowserSandboxControl::OsProcessIsolation],
                guard_fields: vec!["guard_plan.network.deny_metadata_endpoints".to_string()],
                completion_proofs: vec!["temporary profile directory removed".to_string()],
            })
            .await?;

        match server.join() {
            Ok(result) => result?,
            Err(_) => return Err("adapter validation test server panicked".into()),
        }
        let request = request_rx.recv_timeout(Duration::from_secs(1))?;
        assert!(request.starts_with("POST /v1/browser/adapter/validate "));
        assert!(request.contains("x-beatbox-api-key: secret"));
        assert!(request.contains("content-type: application/json"));
        assert!(request.contains(r#""adapter_id":"tempo-os-jail-v1""#));
        assert!(request.contains(r#""launch_endpoint":"https://adapter.example/launch""#));
        assert_eq!(
            validation.decision,
            BrowserAdapterValidationDecision::Rejected
        );
        assert!(!validation.manifest_complete);
        assert!(!validation.launchable);
        assert!(!validation.trusted_for_sensitive_work);
        assert!(!validation.endpoint_network_policy_bound);
        assert_eq!(
            validation.conformance_profile.profile_version,
            "browser-adapter-conformance-v1"
        );
        Ok(())
    }

    #[tokio::test]
    async fn browser_adapter_completion_validate_posts_authenticated_json()
    -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let (request_tx, request_rx) = mpsc::channel();
        let server = std::thread::spawn(move || -> std::io::Result<()> {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(1)))?;
            let mut buffer = [0_u8; 4096];
            let bytes = stream.read(&mut buffer)?;
            let request = String::from_utf8_lossy(&buffer[..bytes]).to_string();
            request_tx
                .send(request)
                .map_err(|_| std::io::Error::other("request receiver dropped"))?;
            let body = r#"{"decision":"rejected","report_shape_complete":true,"verified_on_production_path":false,"trusted_for_sensitive_work":false,"request_id":"browser-adapter-conformance-launch-v1","adapter_id":"tempo-conformance-adapter-v1","contract_version":"browser-adapter-v1","missing_proof_ids":[],"unexpected_proof_ids":[],"failed_evidence_fields":[],"required_completion_proofs":["temporary profile directory removed"],"completion_proof_contract":[{"proof_id":"temporary_profile_removed","label":"temporary profile directory removed","evidence_field":"temporary_profile_removed","required_invariant":"fresh profile directory is removed before completion is trusted"}],"reasons":["shape only"],"required_next_steps":["verify production teardown"],"adapter_contract":{"version":"browser-adapter-v1","status":"planned","launch_endpoint":null,"handoff_fields":["completion_report_template"],"required_guard_fields":[],"required_completion_proofs":["temporary profile directory removed"],"completion_proof_contract":[{"proof_id":"temporary_profile_removed","label":"temporary profile directory removed","evidence_field":"temporary_profile_removed","required_invariant":"fresh profile directory is removed before completion is trusted"}],"unavailable_reason":"no browser adapter launch endpoint is implemented by this daemon"}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes())?;
            Ok(())
        });

        let client = Client::new(format!("http://{addr}")).with_api_key("secret");
        let validation = client
            .browser_adapter_completion_validate(&BrowserAdapterCompletionReport {
                request_id: "browser-adapter-conformance-launch-v1".to_string(),
                adapter_id: "tempo-conformance-adapter-v1".to_string(),
                contract_version: "browser-adapter-v1".to_string(),
                process_terminated: true,
                temporary_profile_removed: true,
                plaintext_artifacts_removed: true,
                egress_log_sealed_or_discarded: true,
                sealed_artifact_handles: Vec::new(),
                proof_ids: vec!["temporary_profile_removed".to_string()],
                notes: Vec::new(),
            })
            .await?;

        match server.join() {
            Ok(result) => result?,
            Err(_) => return Err("adapter completion validation test server panicked".into()),
        }
        let request = request_rx.recv_timeout(Duration::from_secs(1))?;
        assert!(request.starts_with("POST /v1/browser/adapter/completion/validate "));
        assert!(request.contains("x-beatbox-api-key: secret"));
        assert!(request.contains("content-type: application/json"));
        assert!(request.contains(r#""request_id":"browser-adapter-conformance-launch-v1""#));
        assert!(request.contains(r#""proof_ids":["temporary_profile_removed"]"#));
        assert_eq!(
            validation.decision,
            BrowserAdapterCompletionValidationDecision::Rejected
        );
        assert!(validation.report_shape_complete);
        assert!(!validation.verified_on_production_path);
        assert!(!validation.trusted_for_sensitive_work);
        Ok(())
    }

    #[tokio::test]
    async fn api_key_header_is_not_forwarded_across_redirects()
    -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let saw_redirect_target = Arc::new(AtomicBool::new(false));
        let saw_key_on_redirect_target = Arc::new(AtomicBool::new(false));

        let server_stop = Arc::clone(&stop);
        let server_saw_redirect_target = Arc::clone(&saw_redirect_target);
        let server_saw_key_on_redirect_target = Arc::clone(&saw_key_on_redirect_target);
        let server = std::thread::spawn(move || -> std::io::Result<()> {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut handled_initial_request = false;
            while Instant::now() < deadline
                && (!server_stop.load(Ordering::SeqCst) || !handled_initial_request)
            {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_read_timeout(Some(Duration::from_secs(1)))?;
                        let mut buffer = [0_u8; 4096];
                        let bytes = stream.read(&mut buffer)?;
                        let request = String::from_utf8_lossy(&buffer[..bytes]);
                        let lower = request.to_ascii_lowercase();
                        if request.starts_with("POST /v1/execute ") {
                            handled_initial_request = true;
                            if !lower.contains("x-beatbox-api-key: secret") {
                                return Err(std::io::Error::other(
                                    "initial request did not include API key header",
                                ));
                            }
                            let response = format!(
                                "HTTP/1.1 307 Temporary Redirect\r\nlocation: http://{addr}/leak\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                            );
                            stream.write_all(response.as_bytes())?;
                        } else if request.contains(" /leak ") {
                            server_saw_redirect_target.store(true, Ordering::SeqCst);
                            if lower.contains("x-beatbox-api-key") {
                                server_saw_key_on_redirect_target.store(true, Ordering::SeqCst);
                            }
                            stream.write_all(
                                b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
                            )?;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => return Err(error),
                }
            }
            Ok(())
        });

        let client = Client::new(format!("http://{addr}")).with_api_key("secret");
        let request = ExecuteRequest {
            lane: Lane::Wasm,
            source: Source::WasmWat {
                text: "(module)".to_string(),
            },
            entrypoint: None,
            input: serde_json::Value::Null,
            stdin: String::new(),
            policy: Policy::default(),
            idempotency_key: None,
        };

        let error = match client.execute(&request).await {
            Ok(_) => return Err("redirect response should not decode as execution result".into()),
            Err(error) => error,
        };
        match error {
            ClientError::Api { status, .. } => {
                assert_eq!(status, StatusCode::TEMPORARY_REDIRECT);
            }
            ClientError::Http(error) => return Err(error.into()),
            ClientError::InvalidUrl(message) => return Err(message.into()),
        }

        stop.store(true, Ordering::SeqCst);
        match server.join() {
            Ok(result) => result?,
            Err(_) => return Err("redirect test server panicked".into()),
        }
        assert!(!saw_redirect_target.load(Ordering::SeqCst));
        assert!(!saw_key_on_redirect_target.load(Ordering::SeqCst));
        Ok(())
    }
}
