use std::time::Duration;

use reqwest::{Method, StatusCode, header};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::config::Config;
use crate::error::Error;
use crate::version::CLIENT_VERSION;

/// Statuses retried with backoff, mirroring the sibling clients: 429 and
/// transient 5xx.
const RETRYABLE: [StatusCode; 5] = [
    StatusCode::TOO_MANY_REQUESTS,
    StatusCode::INTERNAL_SERVER_ERROR,
    StatusCode::BAD_GATEWAY,
    StatusCode::SERVICE_UNAVAILABLE,
    StatusCode::GATEWAY_TIMEOUT,
];

const BASE_RETRY_DELAY: Duration = Duration::from_millis(500);

/// Performs authenticated REST requests with retry and backoff.
pub(crate) struct ApiClient {
    cfg: Config,
    http: reqwest::Client,
}

impl ApiClient {
    pub(crate) fn new(cfg: Config) -> Result<Self, Error> {
        let mut headers = header::HeaderMap::new();
        let mut api_key =
            header::HeaderValue::from_str(&cfg.api_key).map_err(|_| Error::MissingApiKey)?;
        api_key.set_sensitive(true);
        headers.insert("x-api-key", api_key);
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .default_headers(headers)
            .user_agent(CLIENT_VERSION)
            .build()?;
        Ok(Self { cfg, http })
    }

    pub(crate) async fn get<R: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<R, Error> {
        self.request(Method::GET, path, query, None::<&()>).await
    }

    pub(crate) async fn post<B: Serialize + ?Sized, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<R, Error> {
        self.request(Method::POST, path, &[], Some(body)).await
    }

    /// Sends one API request, retrying retryable statuses with exponential
    /// backoff (500ms · 2^(attempt−1)), and decodes the 2xx response.
    async fn request<B: Serialize + ?Sized, R: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&B>,
    ) -> Result<R, Error> {
        let url = format!("{}{}", self.cfg.base_url, path);
        let mut attempt: u32 = 1;
        loop {
            let mut req = self.http.request(method.clone(), &url);
            if !query.is_empty() {
                req = req.query(query);
            }
            if let Some(body) = body {
                req = req.json(body);
            }
            let resp = req.send().await?;
            let status = resp.status();
            if status.is_success() {
                let text = resp.text().await?;
                return serde_json::from_str(&text).map_err(Error::from);
            }
            let body_text = resp.text().await.unwrap_or_default();
            if RETRYABLE.contains(&status) && attempt < self.cfg.retry_count {
                let delay = BASE_RETRY_DELAY * 2u32.pow(attempt - 1);
                tracing::warn!(status = %status, attempt, ?delay, "finlight: retrying request");
                tokio::time::sleep(delay).await;
                attempt += 1;
                continue;
            }
            return Err(Error::Api {
                status,
                body: body_text,
            });
        }
    }
}
