pub use beatbox_core::*;

use std::time::Duration;

use reqwest::StatusCode;
use thiserror::Error;

pub const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(65);

#[derive(Clone)]
pub struct Client {
    base_url: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

impl Client {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: trim_base_url(base_url.into()),
            api_key: None,
            http: default_http_client(),
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
        let request = self.http.get(format!("{}/v1/jobs/{job_id}", self.base_url));
        let response = self.authorize(request).send().await?;
        decode_response(response).await
    }

    pub async fn cancel_job(&self, job_id: &str) -> Result<(), ClientError> {
        let request = self
            .http
            .delete(format!("{}/v1/jobs/{job_id}", self.base_url));
        let response = self.authorize(request).send().await?;
        decode_empty_response(response).await
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

fn default_http_client() -> reqwest::Client {
    match http_client_builder().timeout(DEFAULT_HTTP_TIMEOUT).build() {
        Ok(client) => client,
        Err(error) => panic!("default beatbox HTTP client must construct: {error}"),
    }
}

fn http_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().redirect(reqwest::redirect::Policy::none())
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error(transparent)]
    Http(#[from] reqwest::Error),
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
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use super::*;

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
