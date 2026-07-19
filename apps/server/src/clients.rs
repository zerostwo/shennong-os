use axum::body::Bytes;
use futures_util::StreamExt;
use reqwest::{Client, Method, StatusCode, redirect::Policy};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use thiserror::Error;
use url::Url;

const MAX_CONTROL_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const DEFAULT_SERVICE_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const RUNTIME_REQUEST_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Error)]
pub enum ClientConfigError {
    #[error("invalid URL for {0}")]
    Invalid(&'static str),
    #[error("HTTP client construction failed")]
    Build(#[from] reqwest::Error),
}

#[derive(Debug, Error)]
pub enum UpstreamError {
    #[error("upstream request failed")]
    Request(#[from] reqwest::Error),
    #[error("upstream response exceeded the control-plane limit")]
    TooLarge,
    #[error("upstream returned invalid JSON")]
    InvalidJson,
}

#[derive(Clone)]
pub struct ServiceClient {
    service: &'static str,
    base: Url,
    client: Client,
}

#[derive(Clone)]
pub struct AgentRuntimeClient {
    target: Url,
    secret: String,
    client: Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceHealth {
    pub service: String,
    pub reachable: bool,
    pub status: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug)]
pub struct JsonResponse {
    pub status: StatusCode,
    pub body: Value,
}

impl ServiceClient {
    pub fn new(service: &'static str, raw: &str) -> Result<Self, ClientConfigError> {
        let mut base = checked_url(service, raw)?;
        if !base.path().ends_with('/') {
            let path = format!("{}/", base.path());
            base.set_path(&path);
        }
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .timeout(service_request_timeout(service))
            .redirect(Policy::none())
            .build()?;
        Ok(Self {
            service,
            base,
            client,
        })
    }

    pub async fn health(&self) -> ServiceHealth {
        let segments: &[&str] = if self.service == "shennong-runtime" {
            &["v1", "health"]
        } else {
            &["healthz"]
        };
        match self
            .request_json(Method::GET, segments, &[], None, None, None)
            .await
        {
            Ok(response) if response.status.is_success() => ServiceHealth {
                service: self.service.into(),
                reachable: true,
                status: response
                    .body
                    .get("status")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or(Some("ok".into())),
                version: response
                    .body
                    .get("version")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            },
            _ => self.unreachable(),
        }
    }

    pub async fn info(&self) -> ServiceHealth {
        let mut health = self.health().await;
        if !health.reachable {
            return health;
        }
        let segments: &[&str] = if self.service == "shennong-runtime" {
            &["v1", "info"]
        } else {
            &["version"]
        };
        if let Ok(response) = self
            .request_json(Method::GET, segments, &[], None, None, None)
            .await
            && response.status.is_success()
        {
            health.version = response
                .body
                .get("version")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or(health.version);
        }
        health
    }

    pub async fn request_json(
        &self,
        method: Method,
        segments: &[&str],
        query: &[(&str, String)],
        body: Option<&Value>,
        authorization: Option<(&str, &str)>,
        idempotency_key: Option<&str>,
    ) -> Result<JsonResponse, UpstreamError> {
        self.request_json_with_headers(
            method,
            segments,
            query,
            body,
            authorization,
            idempotency_key,
            &[],
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn request_json_with_headers(
        &self,
        method: Method,
        segments: &[&str],
        query: &[(&str, String)],
        body: Option<&Value>,
        authorization: Option<(&str, &str)>,
        idempotency_key: Option<&str>,
        headers: &[(&str, &str)],
    ) -> Result<JsonResponse, UpstreamError> {
        let mut url = self.streaming_url(segments, None)?;
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (name, value) in query {
                pairs.append_pair(name, value);
            }
        }
        let mut request = self
            .client
            .request(method, url)
            .header("accept", "application/json");
        if let Some((name, value)) = authorization {
            request = request.header(name, value);
        }
        if let Some(key) = idempotency_key {
            request = request.header("idempotency-key", key);
        }
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await?;
        let status = response.status();
        let bytes = bounded_bytes(response, MAX_CONTROL_RESPONSE_BYTES).await?;
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).map_err(|_| UpstreamError::InvalidJson)?
        };
        Ok(JsonResponse { status, body })
    }

    pub async fn request_streaming_json(
        &self,
        method: Method,
        segments: &[&str],
        body: reqwest::Body,
        authorization: Option<(&str, &str)>,
        headers: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<JsonResponse, UpstreamError> {
        let url = self.streaming_url(segments, None)?;
        let mut request = self
            .client
            .request(method, url)
            .header("accept", "application/json")
            .timeout(timeout);
        if let Some((name, value)) = authorization {
            request = request.header(name, value);
        }
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = request.body(body).send().await?;
        let status = response.status();
        let bytes = bounded_bytes(response, MAX_CONTROL_RESPONSE_BYTES).await?;
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).map_err(|_| UpstreamError::InvalidJson)?
        };
        Ok(JsonResponse { status, body })
    }

    pub(crate) fn streaming_url(
        &self,
        segments: &[&str],
        raw_query: Option<&str>,
    ) -> Result<Url, UpstreamError> {
        let mut url = self.base.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| UpstreamError::InvalidJson)?;
            path.pop_if_empty();
            for segment in segments {
                path.push(segment);
            }
        }
        url.set_query(raw_query);
        Ok(url)
    }

    pub(crate) fn streaming_client(&self) -> &Client {
        &self.client
    }

    fn unreachable(&self) -> ServiceHealth {
        ServiceHealth {
            service: self.service.into(),
            reachable: false,
            status: None,
            version: None,
        }
    }
}

fn service_request_timeout(service: &str) -> Duration {
    if service == "shennong-runtime" {
        RUNTIME_REQUEST_TIMEOUT
    } else {
        DEFAULT_SERVICE_REQUEST_TIMEOUT
    }
}

impl AgentRuntimeClient {
    pub fn new(raw: &str, secret: String) -> Result<Self, ClientConfigError> {
        if secret.len() < 32 {
            return Err(ClientConfigError::Invalid("shennong-agent-runtime secret"));
        }
        let mut target = checked_url("shennong-agent-runtime", raw)?;
        match target.path().trim_end_matches('/') {
            "" => target.set_path("/v1/agent"),
            "/v1/agent" | "/api/agent" => {}
            _ => return Err(ClientConfigError::Invalid("shennong-agent-runtime")),
        }
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .redirect(Policy::none())
            .build()?;
        Ok(Self {
            target,
            secret,
            client,
        })
    }

    pub async fn run(&self, body: Bytes) -> Result<reqwest::Response, reqwest::Error> {
        self.client
            .post(self.target.clone())
            .bearer_auth(&self.secret)
            .header("accept", "text/event-stream")
            .header("content-type", "application/json")
            .header("x-shennong-os", "agent-gateway")
            .body(body)
            .send()
            .await
    }

    pub async fn healthy(&self) -> bool {
        let mut target = self.target.clone();
        target.set_path("/health");
        self.client
            .get(target)
            .timeout(Duration::from_secs(1))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
    }
}

async fn bounded_bytes(
    response: reqwest::Response,
    maximum: usize,
) -> Result<Vec<u8>, UpstreamError> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum as u64)
    {
        return Err(UpstreamError::TooLarge);
    }
    let mut stream = response.bytes_stream();
    let mut output = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if output.len().saturating_add(chunk.len()) > maximum {
            return Err(UpstreamError::TooLarge);
        }
        output.extend_from_slice(&chunk);
    }
    Ok(output)
}

fn checked_url(service: &'static str, raw: &str) -> Result<Url, ClientConfigError> {
    let url = Url::parse(raw).map_err(|_| ClientConfigError::Invalid(service))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ClientConfigError::Invalid(service));
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_base_rejects_credentials_and_non_http() {
        assert!(ServiceClient::new("db", "file:///tmp/db").is_err());
        assert!(ServiceClient::new("db", "https://user:pass@example.org").is_err());
        assert!(ServiceClient::new("db", "http://shennong-db:8000/").is_ok());
    }

    #[test]
    fn runtime_request_timeout_covers_ide_readiness_without_relaxing_db() {
        assert_eq!(
            service_request_timeout("shennong-db"),
            Duration::from_secs(30)
        );
        assert_eq!(
            service_request_timeout("shennong-runtime"),
            Duration::from_secs(90)
        );
    }

    #[test]
    fn agent_target_is_fixed_to_the_single_runtime_endpoint() {
        let secret = "s".repeat(32);
        assert!(AgentRuntimeClient::new("http://agent:8010", secret.clone()).is_ok());
        assert!(AgentRuntimeClient::new("http://agent:8010/v1/agent", secret.clone()).is_ok());
        assert!(AgentRuntimeClient::new("http://agent:8010/arbitrary", secret).is_err());
    }
}
