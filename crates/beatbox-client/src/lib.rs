pub use beatbox_core::*;

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
    /// Construct a client, returning an error if the underlying HTTP client
    /// cannot be built. Prefer this over [`new`](Self::new) in library code that
    /// must not panic.
    pub fn try_new(base_url: impl Into<String>) -> Result<Self, ClientError> {
        Ok(Self {
            base_url: trim_base_url(base_url.into()),
            api_key: None,
            http: http_client_builder()
                .timeout(DEFAULT_HTTP_TIMEOUT)
                .build()?,
        })
    }

    /// Construct a client with the default configuration.
    ///
    /// Panics only if the HTTP client cannot be built, which does not happen
    /// with the pinned rustls/bundled-roots configuration. Use
    /// [`try_new`](Self::try_new) for a non-panicking constructor.
    pub fn new(base_url: impl Into<String>) -> Self {
        match Self::try_new(base_url) {
            Ok(client) => client,
            Err(error) => panic!("default beatbox HTTP client must construct: {error}"),
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
            let body = r#"{"decision":"rejected","runnable_browser_sessions":false,"requested_level":"os_isolated","selected_level":null,"actor":"agent","sensitivity":"sensitive","target_origins":["https://example.com"],"credential_mode":"no_credentials","artifact_mode":"discard","requested_controls":["egress_policy","remote_worker_isolation"],"requested_profile_controls":["fresh_profile","no_ambient_credentials","egress_policy","local_network_block","os_process_isolation","teardown_proof"],"missing_controls":["remote_worker_isolation"],"level_satisfies_requested_controls":false,"intent_warnings":[],"guard_plan":{"network":{"allowed_origins":["https://example.com"],"deny_private_networks":true,"deny_localhost":true,"deny_metadata_endpoints":true,"require_dns_rebinding_protection":true,"require_redirect_revalidation":true,"require_proxy_enforcement":true,"outbound_network_disabled_without_proxy":true},"credentials":{"mode":"no_credentials","ambient_credentials_allowed":false,"user_mediation_required":false,"scoped_secret_channel_required":false},"storage":{"mode":"discard","plaintext_persistence_allowed":false,"explicit_artifact_allowlist_required":false,"encryption_required_for_persistence":false,"teardown_proof_required":true},"required_runtime_guards":["browser launcher bound to the selected sandbox profile","production-path admission check before launch","teardown proof before reporting session completion","fresh profile directory with no host browser state","deny-by-default egress proxy that revalidates DNS, redirects, and final socket targets","loopback, LAN, shared, link-local, and metadata address block","OS jail or microVM boundary around the browser process"]},"downgrade_allowed":false,"reasons":["no runnable browser sandbox"],"required_next_steps":["implement a browser launcher"],"profiles_endpoint":"/v1/browser/profiles"}"#;
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
