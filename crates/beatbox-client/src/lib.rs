pub use beatbox_core::*;

use std::net::IpAddr;
use std::time::Duration;

use reqwest::header::CONTENT_TYPE;
use reqwest::{StatusCode, Url};
use thiserror::Error;

pub const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(65);
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
pub struct Client {
    base_url: String,
    api_key: Option<ApiKey>,
    http: reqwest::Client,
    max_response_bytes: usize,
}

#[derive(Clone)]
struct ApiKey {
    value: String,
    allow_loopback_http: bool,
}

impl Client {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: trim_base_url(base_url.into()),
            api_key: None,
            http: default_http_client(),
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
        }
    }

    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(ApiKey {
            value: api_key.into(),
            allow_loopback_http: false,
        });
        self
    }

    pub fn with_api_key_allowing_loopback_http(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(ApiKey {
            value: api_key.into(),
            allow_loopback_http: true,
        });
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Result<Self, ClientError> {
        self.http = http_client_builder().timeout(timeout).build()?;
        Ok(self)
    }

    pub fn with_max_response_bytes(mut self, max_response_bytes: usize) -> Self {
        self.max_response_bytes = max_response_bytes;
        self
    }

    pub async fn health(&self) -> Result<serde_json::Value, ClientError> {
        let response = self
            .http
            .get(self.endpoint_url("/v1/health")?)
            .send()
            .await?;
        decode_response(response, self.max_response_bytes).await
    }

    pub async fn capabilities(&self) -> Result<serde_json::Value, ClientError> {
        let request = self.http.get(self.endpoint_url("/v1/capabilities")?);
        let response = self.authorize(request)?.send().await?;
        decode_response(response, self.max_response_bytes).await
    }

    pub async fn execute(&self, request: &ExecuteRequest) -> Result<ExecutionResult, ClientError> {
        let request_builder = self
            .http
            .post(self.endpoint_url("/v1/execute")?)
            .json(request);
        let response = self.authorize(request_builder)?.send().await?;
        decode_response(response, self.max_response_bytes).await
    }

    pub async fn create_job(
        &self,
        request: &ExecuteRequest,
    ) -> Result<CreateJobResponse, ClientError> {
        let request_builder = self.http.post(self.endpoint_url("/v1/jobs")?).json(request);
        let response = self.authorize(request_builder)?.send().await?;
        decode_response(response, self.max_response_bytes).await
    }

    pub async fn get_job(&self, job_id: &str) -> Result<JobRecord, ClientError> {
        let request = self.http.get(self.job_url(job_id)?);
        let response = self.authorize(request)?.send().await?;
        decode_response(response, self.max_response_bytes).await
    }

    pub async fn cancel_job(&self, job_id: &str) -> Result<(), ClientError> {
        let request = self.http.delete(self.job_url(job_id)?);
        let response = self.authorize(request)?.send().await?;
        decode_empty_response(response, self.max_response_bytes).await
    }

    pub async fn openapi(&self) -> Result<serde_json::Value, ClientError> {
        let request = self.http.get(self.endpoint_url("/openapi.json")?);
        let response = self.authorize(request)?.send().await?;
        decode_response(response, self.max_response_bytes).await
    }

    fn endpoint_url(&self, path: &str) -> Result<Url, ClientError> {
        endpoint_url(&self.base_url, path)
    }

    fn job_url(&self, job_id: &str) -> Result<Url, ClientError> {
        let mut url = self.endpoint_url("/v1/jobs")?;
        url.path_segments_mut()
            .map_err(|_| ClientError::InvalidBaseUrl {
                base_url: self.base_url.clone(),
                reason: "base URL cannot be used as a path base".to_string(),
            })?
            .push(job_id);
        Ok(url)
    }

    fn authorize(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, ClientError> {
        match &self.api_key {
            Some(api_key) => {
                validate_api_key(&api_key.value)?;
                validate_api_key_base_url(&self.base_url, api_key.allow_loopback_http)?;
                Ok(request.header("x-beatbox-api-key", &api_key.value))
            }
            None => Ok(request),
        }
    }
}

fn default_http_client() -> reqwest::Client {
    match http_client_builder().timeout(DEFAULT_HTTP_TIMEOUT).build() {
        Ok(client) => client,
        Err(error) => panic!("default beatbox HTTP client must construct: {error}"),
    }
}

fn http_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error("invalid beatbox base URL {base_url}: {reason}")]
    InvalidBaseUrl { base_url: String, reason: String },
    #[error("invalid beatbox API key: {reason}")]
    InvalidApiKey { reason: String },
    #[error("refusing to send beatbox API key to {base_url}: {reason}")]
    UnsafeApiKeyBaseUrl { base_url: String, reason: String },
    #[error("beatbox API response exceeded configured limit of {max_bytes} bytes")]
    ResponseTooLarge { max_bytes: usize },
    #[error("beatbox API returned unexpected content type `{actual}`; expected application/json")]
    UnexpectedContentType { actual: String },
    #[error("failed to decode beatbox API response JSON: {0}")]
    DecodeJson(#[from] serde_json::Error),
    #[error("beatbox API returned {status}: {message}")]
    Api {
        status: StatusCode,
        code: String,
        message: String,
    },
}

async fn decode_response<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    max_response_bytes: usize,
) -> Result<T, ClientError> {
    let status = response.status();
    if status.is_success() {
        validate_json_response_content_type(&response)?;
    }
    let bytes = read_limited_response(response, max_response_bytes).await?;
    if status.is_success() {
        return serde_json::from_slice(&bytes).map_err(ClientError::from);
    }
    match serde_json::from_slice::<ErrorResponse>(&bytes) {
        Ok(error) => Err(ClientError::Api {
            status,
            code: error.error.code,
            message: error.error.message,
        }),
        Err(error) => Err(ClientError::Api {
            status,
            code: "http_error".to_string(),
            message: error.to_string(),
        }),
    }
}

async fn read_limited_response(
    mut response: reqwest::Response,
    max_response_bytes: usize,
) -> Result<Vec<u8>, ClientError> {
    if response
        .content_length()
        .is_some_and(|length| length > max_response_bytes as u64)
    {
        return Err(ClientError::ResponseTooLarge {
            max_bytes: max_response_bytes,
        });
    }

    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        let Some(new_len) = body.len().checked_add(chunk.len()) else {
            return Err(ClientError::ResponseTooLarge {
                max_bytes: max_response_bytes,
            });
        };
        if new_len > max_response_bytes {
            return Err(ClientError::ResponseTooLarge {
                max_bytes: max_response_bytes,
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn validate_json_response_content_type(response: &reqwest::Response) -> Result<(), ClientError> {
    let Some(value) = response.headers().get(CONTENT_TYPE) else {
        return Err(ClientError::UnexpectedContentType {
            actual: "<missing>".to_string(),
        });
    };
    let Ok(value) = value.to_str() else {
        return Err(ClientError::UnexpectedContentType {
            actual: "<non-utf8>".to_string(),
        });
    };
    let media_type = value.split(';').next().unwrap_or_default().trim();
    if media_type.eq_ignore_ascii_case("application/json")
        || media_type
            .rsplit_once('+')
            .is_some_and(|(_, suffix)| suffix.eq_ignore_ascii_case("json"))
    {
        Ok(())
    } else {
        Err(ClientError::UnexpectedContentType {
            actual: value.to_string(),
        })
    }
}

fn validate_api_key(api_key: &str) -> Result<(), ClientError> {
    if api_key.trim().is_empty() {
        Err(ClientError::InvalidApiKey {
            reason: "API key must not be empty".to_string(),
        })
    } else {
        Ok(())
    }
}

fn validate_api_key_base_url(base_url: &str, allow_loopback_http: bool) -> Result<(), ClientError> {
    let url = Url::parse(base_url).map_err(|error| ClientError::UnsafeApiKeyBaseUrl {
        base_url: base_url.to_string(),
        reason: format!("invalid base URL: {error}"),
    })?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ClientError::UnsafeApiKeyBaseUrl {
            base_url: base_url.to_string(),
            reason: "base URL must not include credentials".to_string(),
        });
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(ClientError::UnsafeApiKeyBaseUrl {
            base_url: base_url.to_string(),
            reason: "base URL must not include query or fragment".to_string(),
        });
    }
    match url.scheme() {
        "https" => Ok(()),
        "http" if allow_loopback_http && url.host_str().is_some_and(is_loopback_ip_literal) => {
            Ok(())
        }
        "http" => Err(ClientError::UnsafeApiKeyBaseUrl {
            base_url: base_url.to_string(),
            reason:
                "HTTP API-key transport is allowed only through the explicit loopback IP opt-in"
                    .to_string(),
        }),
        scheme => Err(ClientError::UnsafeApiKeyBaseUrl {
            base_url: base_url.to_string(),
            reason: format!("scheme `{scheme}` is not allowed for API-key transport"),
        }),
    }
}

fn endpoint_url(base_url: &str, path: &str) -> Result<Url, ClientError> {
    let mut url = Url::parse(base_url).map_err(|error| ClientError::InvalidBaseUrl {
        base_url: base_url.to_string(),
        reason: error.to_string(),
    })?;
    if url.query().is_some() || url.fragment().is_some() {
        return Err(ClientError::InvalidBaseUrl {
            base_url: base_url.to_string(),
            reason: "base URL must not include query or fragment".to_string(),
        });
    }
    let base_path = url.path().trim_end_matches('/');
    let path = path.trim_start_matches('/');
    let joined = if base_path.is_empty() {
        format!("/{path}")
    } else {
        format!("{base_path}/{path}")
    };
    url.set_path(&joined);
    Ok(url)
}

fn is_loopback_ip_literal(host: &str) -> bool {
    let host = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    host.parse::<IpAddr>()
        .is_ok_and(|address| address.is_loopback())
}

async fn decode_empty_response(
    response: reqwest::Response,
    max_response_bytes: usize,
) -> Result<(), ClientError> {
    let status = response.status();
    if status == StatusCode::NO_CONTENT {
        return Ok(());
    }
    let bytes = read_limited_response(response, max_response_bytes).await?;
    if status.is_success() {
        return Err(ClientError::Api {
            status,
            code: "unexpected_status".to_string(),
            message: format!("expected 204 No Content, got {status}"),
        });
    }
    match serde_json::from_slice::<ErrorResponse>(&bytes) {
        Ok(error) => Err(ClientError::Api {
            status,
            code: error.error.code,
            message: error.error.message,
        }),
        Err(error) => Err(ClientError::Api {
            status,
            code: "http_error".to_string(),
            message: error.to_string(),
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use super::*;
    use axum::extract::State;
    use axum::http::header::{CONTENT_TYPE as AXUM_CONTENT_TYPE, LOCATION};
    use axum::http::{HeaderMap, HeaderValue};
    use axum::response::{IntoResponse, Response};
    use axum::routing::{delete, get};
    use axum::Router;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[derive(Clone)]
    struct RedirectState {
        location: String,
        saw_initial_key: Arc<AtomicBool>,
        saw_redirect_target: Arc<AtomicBool>,
        saw_key_on_redirect_target: Arc<AtomicBool>,
    }

    #[tokio::test]
    async fn api_key_header_rejects_http_without_loopback_opt_in(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let client = Client::new("http://127.0.0.1:1").with_api_key("secret");
        let error = match client.capabilities().await {
            Ok(_) => return Err("unsafe API-key base URL unexpectedly succeeded".into()),
            Err(error) => error,
        };
        assert_unsafe_api_key_base_url(error, "explicit loopback IP opt-in")?;
        Ok(())
    }

    #[tokio::test]
    async fn api_key_header_rejects_non_loopback_http_with_loopback_opt_in(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let client =
            Client::new("http://example.com").with_api_key_allowing_loopback_http("secret");
        let error = match client.capabilities().await {
            Ok(_) => return Err("unsafe API-key base URL unexpectedly succeeded".into()),
            Err(error) => error,
        };
        assert_unsafe_api_key_base_url(error, "explicit loopback IP opt-in")?;
        Ok(())
    }

    #[tokio::test]
    async fn api_key_header_rejects_localhost_http_with_loopback_opt_in(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let client =
            Client::new("http://localhost:7300").with_api_key_allowing_loopback_http("secret");
        let error = match client.capabilities().await {
            Ok(_) => return Err("localhost API-key base URL unexpectedly succeeded".into()),
            Err(error) => error,
        };
        assert_unsafe_api_key_base_url(error, "loopback IP")?;
        Ok(())
    }

    #[test]
    fn api_key_base_url_validation_allows_https_without_loopback_opt_in(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        validate_api_key_base_url("https://beatbox.example", false)?;
        Ok(())
    }

    #[test]
    fn api_key_base_url_validation_allows_literal_loopback_ips_with_opt_in(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        validate_api_key_base_url("http://127.0.0.1:7300", true)?;
        validate_api_key_base_url("http://[::1]:7300", true)?;
        Ok(())
    }

    #[test]
    fn api_key_header_rejects_empty_values_before_attach(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        for key in ["", " \t "] {
            let client = Client::new("https://beatbox.example").with_api_key(key);
            let request = client.http.get(client.endpoint_url("/v1/capabilities")?);
            match client.authorize(request) {
                Err(ClientError::InvalidApiKey { reason }) => {
                    assert!(reason.contains("empty"), "{reason}");
                }
                Ok(_) => return Err("empty API key was unexpectedly accepted".into()),
                Err(error) => return Err(format!("unexpected error: {error}").into()),
            }
        }
        Ok(())
    }

    #[test]
    fn endpoint_urls_preserve_base_path_and_encode_job_ids(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let client = Client::new("https://beatbox.example/api/");
        assert_eq!(
            client.endpoint_url("/v1/capabilities")?.as_str(),
            "https://beatbox.example/api/v1/capabilities"
        );
        assert_eq!(
            client.job_url("job/with?reserved#chars")?.as_str(),
            "https://beatbox.example/api/v1/jobs/job%2Fwith%3Freserved%23chars"
        );
        Ok(())
    }

    #[test]
    fn endpoint_urls_reject_query_and_fragment_base_urls(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        for base_url in [
            "https://beatbox.example?target=/v1/capabilities",
            "https://beatbox.example/#/v1/capabilities",
        ] {
            let client = Client::new(base_url);
            match client.endpoint_url("/v1/capabilities") {
                Err(ClientError::InvalidBaseUrl { reason, .. }) => {
                    assert!(reason.contains("query or fragment"));
                }
                Ok(url) => return Err(format!("unexpected URL construction: {url}").into()),
                Err(error) => return Err(format!("unexpected error: {error}").into()),
            }
        }
        Ok(())
    }

    #[test]
    fn api_key_base_url_validation_rejects_credentials(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let error = match validate_api_key_base_url("https://user:secret@beatbox.example", false) {
            Ok(()) => return Err("base URL credentials should be rejected".into()),
            Err(error) => error,
        };
        match error {
            ClientError::UnsafeApiKeyBaseUrl { reason, .. } => {
                assert!(reason.contains("credentials"));
            }
            other => {
                return Err(std::io::Error::other(format!(
                    "expected unsafe base URL error, got {other:?}"
                ))
                .into());
            }
        }
        Ok(())
    }

    #[test]
    fn api_key_base_url_validation_rejects_query_and_fragment(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        for base_url in [
            "https://beatbox.example?token-target=/v1/capabilities",
            "http://127.0.0.1:7300/#fragment",
        ] {
            let error = match validate_api_key_base_url(base_url, true) {
                Ok(()) => return Err("base URL query or fragment should be rejected".into()),
                Err(error) => error,
            };
            match error {
                ClientError::UnsafeApiKeyBaseUrl { reason, .. } => {
                    assert!(reason.contains("query or fragment"));
                }
                other => {
                    return Err(std::io::Error::other(format!(
                        "expected unsafe base URL error, got {other:?}"
                    ))
                    .into());
                }
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn api_key_header_is_not_forwarded_across_redirects(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let saw_initial_key = Arc::new(AtomicBool::new(false));
        let saw_redirect_target = Arc::new(AtomicBool::new(false));
        let saw_key_on_redirect_target = Arc::new(AtomicBool::new(false));
        let state = RedirectState {
            location: format!("http://{addr}/leak"),
            saw_initial_key: Arc::clone(&saw_initial_key),
            saw_redirect_target: Arc::clone(&saw_redirect_target),
            saw_key_on_redirect_target: Arc::clone(&saw_key_on_redirect_target),
        };
        let app = Router::new()
            .route("/v1/capabilities", get(redirect_once))
            .route("/leak", get(redirect_target))
            .with_state(state);
        let server = tokio::spawn(async move { axum::serve(listener, app).await });

        let client =
            Client::new(format!("http://{addr}")).with_api_key_allowing_loopback_http("secret");
        let error = match client.capabilities().await {
            Ok(_) => return Err("redirect response should not decode as capabilities".into()),
            Err(error) => error,
        };
        match error {
            ClientError::Api { status, .. } => {
                assert_eq!(status, StatusCode::TEMPORARY_REDIRECT);
            }
            ClientError::Http(error) => return Err(error.into()),
            ClientError::UnsafeApiKeyBaseUrl { base_url, reason } => {
                return Err(format!("loopback test URL {base_url} was rejected: {reason}").into());
            }
            ClientError::InvalidApiKey { reason } => {
                return Err(format!("test API key was rejected: {reason}").into());
            }
            ClientError::InvalidBaseUrl { base_url, reason } => {
                return Err(format!("loopback test URL {base_url} was invalid: {reason}").into());
            }
            ClientError::ResponseTooLarge { max_bytes } => {
                return Err(format!("redirect response exceeded {max_bytes} bytes").into());
            }
            ClientError::UnexpectedContentType { actual } => {
                return Err(
                    format!("redirect response had unexpected content type {actual}").into(),
                );
            }
            ClientError::DecodeJson(error) => return Err(error.into()),
        }

        server.abort();
        assert!(saw_initial_key.load(Ordering::SeqCst));
        assert!(!saw_redirect_target.load(Ordering::SeqCst));
        assert!(!saw_key_on_redirect_target.load(Ordering::SeqCst));
        Ok(())
    }

    #[tokio::test]
    async fn response_body_limit_rejects_chunked_success_without_content_length(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await?;
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await?;
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n",
                )
                .await?;
            stream.write_all(br#"{"padding":""#).await?;
            stream.write_all(&[b'a'; 128]).await?;
            stream.write_all(br#""}"#).await?;
            Ok::<(), std::io::Error>(())
        });

        let client = Client::new(format!("http://{addr}")).with_max_response_bytes(64);
        let error = match client.health().await {
            Ok(value) => {
                return Err(format!("oversized response unexpectedly decoded: {value}").into())
            }
            Err(error) => error,
        };
        match error {
            ClientError::ResponseTooLarge { max_bytes } => assert_eq!(max_bytes, 64),
            other => return Err(format!("expected response size error, got {other:?}").into()),
        }
        server.await??;
        Ok(())
    }

    #[tokio::test]
    async fn response_body_limit_allows_configured_larger_response(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let app = Router::new().route(
            "/v1/health",
            get(|| async {
                (
                    [(AXUM_CONTENT_TYPE, "application/json; charset=utf-8")],
                    r#"{"status":"ok","padding":"aaaaaaaaaaaaaaaa"}"#,
                )
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await });

        let value = Client::new(format!("http://{addr}"))
            .with_max_response_bytes(128)
            .health()
            .await?;
        assert_eq!(value["status"], "ok");
        server.abort();
        Ok(())
    }

    #[tokio::test]
    async fn successful_response_rejects_non_json_content_type(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let app = Router::new().route("/v1/health", get(|| async { r#"{"status":"ok"}"# }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await });

        let error = match Client::new(format!("http://{addr}")).health().await {
            Ok(value) => {
                return Err(format!("text/plain JSON was unexpectedly decoded: {value}").into());
            }
            Err(error) => error,
        };
        match error {
            ClientError::UnexpectedContentType { actual } => {
                assert!(actual.starts_with("text/plain"), "{actual}");
            }
            other => return Err(format!("expected content-type error, got {other:?}").into()),
        }

        server.abort();
        Ok(())
    }

    #[tokio::test]
    async fn cancel_job_accepts_only_204_no_content_success(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let app = Router::new().route("/v1/jobs/{id}", delete(|| async { StatusCode::NO_CONTENT }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await });

        Client::new(format!("http://{addr}"))
            .cancel_job("job-1")
            .await?;

        server.abort();
        Ok(())
    }

    #[tokio::test]
    async fn cancel_job_rejects_other_2xx_statuses(
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let app = Router::new().route(
            "/v1/jobs/{id}",
            delete(|| async {
                (
                    StatusCode::OK,
                    [(AXUM_CONTENT_TYPE, "application/json")],
                    r#"{"ignored":true}"#,
                )
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await });

        let error = match Client::new(format!("http://{addr}"))
            .cancel_job("job-1")
            .await
        {
            Ok(()) => return Err("non-204 cancel response was unexpectedly accepted".into()),
            Err(error) => error,
        };
        match error {
            ClientError::Api { status, code, .. } => {
                assert_eq!(status, StatusCode::OK);
                assert_eq!(code, "unexpected_status");
            }
            other => {
                return Err(format!("expected unexpected_status API error, got {other:?}").into())
            }
        }

        server.abort();
        Ok(())
    }

    async fn redirect_once(State(state): State<RedirectState>, headers: HeaderMap) -> Response {
        if api_key_header_matches(&headers, "secret") {
            state.saw_initial_key.store(true, Ordering::SeqCst);
        }
        let mut response = StatusCode::TEMPORARY_REDIRECT.into_response();
        match HeaderValue::from_str(&state.location) {
            Ok(location) => {
                response.headers_mut().insert(LOCATION, location);
                response
            }
            Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
        }
    }

    async fn redirect_target(State(state): State<RedirectState>, headers: HeaderMap) -> Response {
        state.saw_redirect_target.store(true, Ordering::SeqCst);
        if headers.get("x-beatbox-api-key").is_some() {
            state
                .saw_key_on_redirect_target
                .store(true, Ordering::SeqCst);
        }
        (StatusCode::OK, "{}").into_response()
    }

    fn api_key_header_matches(headers: &HeaderMap, expected: &str) -> bool {
        headers
            .get("x-beatbox-api-key")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|actual| actual == expected)
    }

    fn assert_unsafe_api_key_base_url(
        error: ClientError,
        expected_reason: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match error {
            ClientError::UnsafeApiKeyBaseUrl { reason, .. } => {
                assert!(reason.contains(expected_reason), "{reason}");
                Ok(())
            }
            other => Err(std::io::Error::other(format!(
                "expected unsafe base URL error, got {other:?}"
            ))
            .into()),
        }
    }
}
