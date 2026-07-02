pub use beatbox_core::*;

use reqwest::StatusCode;
use thiserror::Error;

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
            http: reqwest::Client::new(),
        }
    }

    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
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
            Some(api_key) => request.bearer_auth(api_key),
            None => request,
        }
    }
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
