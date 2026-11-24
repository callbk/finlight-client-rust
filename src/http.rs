//! REST transport: headers, retries, burst wait, error bodies.

use std::sync::Arc;
use std::time::Duration;

use reqwest::Method;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value};

use crate::config::{Config, RetryOnBurst};
use crate::error::{ApiError, ApiErrorData, ApiErrorKind, Error, FieldIssue, Result};
use crate::params::encode_query;
use crate::version::{VERSION, user_agent};
use crate::vocab::{ProblemCode, RateLimitType};

pub(crate) struct Request {
    pub method: Method,
    pub path: String,
    pub query: Vec<(&'static str, String)>,
    pub body: Option<String>,
    pub if_none_match: Option<String>,
}

impl Request {
    pub fn get(path: impl Into<String>) -> Self {
        Request {
            method: Method::GET,
            path: path.into(),
            query: Vec::new(),
            body: None,
            if_none_match: None,
        }
    }

    pub fn post(path: impl Into<String>, body: String) -> Self {
        Request {
            method: Method::POST,
            path: path.into(),
            query: Vec::new(),
            body: Some(body),
            if_none_match: None,
        }
    }
}

pub(crate) struct Response {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: String,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    }
}

enum RetryClass {
    Never,
    Burst,
    Backoff,
}

pub(crate) struct Transport {
    cfg: Arc<Config>,
    http: reqwest::Client,
    owns_client: bool,
}

impl Transport {
    pub fn new(cfg: Arc<Config>) -> Result<Self> {
        let (http, owns_client) = match &cfg.http_client {
            Some(client) => (client.clone(), false),
            None => {
                let client = reqwest::Client::builder()
                    .connect_timeout(cfg.timeout)
                    // No hidden retries: every attempt is counted and logged by the SDK.
                    .retry(reqwest::retry::never())
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .map_err(|e| {
                        Error::network("could not build the HTTP client", Some(Box::new(e)))
                    })?;
                (client, true)
            }
        };
        Ok(Transport {
            cfg,
            http,
            owns_client,
        })
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// One logical call: one `X-Request-Id`, retries and the burst wait included.
    pub async fn call(&self, req: &Request) -> Result<Response> {
        let request_id = uuid::Uuid::new_v4().to_string();
        match self.cfg.total_timeout {
            Some(limit) => {
                match tokio::time::timeout(limit, self.call_with_retries(req, &request_id)).await {
                    Ok(result) => result,
                    Err(_) => Err(Error::timeout(
                        format!("the call did not finish within total_timeout ({limit:?})"),
                        None,
                    )),
                }
            }
            None => self.call_with_retries(req, &request_id).await,
        }
    }

    async fn call_with_retries(&self, req: &Request, request_id: &str) -> Result<Response> {
        let cfg = &self.cfg;
        let mut retries: u32 = 0;
        let mut burst_retried = false;
        loop {
            let error = match self.attempt(req, request_id).await {
                Ok(resp) if (200..300).contains(&resp.status) || resp.status == 304 => {
                    return Ok(resp);
                }
                Ok(resp) => {
                    let err =
                        api_error_from_response(resp.status, &resp.headers, &resp.body, request_id);
                    match retry_class(&err) {
                        RetryClass::Burst
                            if cfg.retry_on_burst == RetryOnBurst::Once && !burst_retried =>
                        {
                            let wait = err.retry_after().unwrap_or(cfg.burst_wait);
                            tracing::warn!(
                                target: "finlight",
                                request_id,
                                "burst rate limit reached; retrying once after {wait:?}"
                            );
                            burst_retried = true;
                            tokio::time::sleep(wait).await;
                            continue;
                        }
                        RetryClass::Backoff => Error::Api(err),
                        _ => return Err(Error::Api(err)),
                    }
                }
                Err(err @ (Error::Network(_) | Error::Timeout(_))) => err,
                Err(err) => return Err(err),
            };
            if retries >= cfg.max_retries {
                return Err(error);
            }
            let delay = backoff(cfg.retry_base_delay, cfg.retry_max_delay, retries);
            retries += 1;
            tracing::warn!(
                target: "finlight",
                request_id,
                retry = retries,
                "retrying after {delay:?}: {error}"
            );
            tokio::time::sleep(delay).await;
        }
    }

    async fn attempt(&self, req: &Request, request_id: &str) -> Result<Response> {
        let cfg = &self.cfg;
        let mut url = format!("{}{}", cfg.rest_base(), req.path);
        if !req.query.is_empty() {
            url.push('?');
            url.push_str(&encode_query(&req.query));
        }
        let mut headers = HeaderMap::new();
        let mut key = HeaderValue::from_str(cfg.api_key()).map_err(|_| {
            Error::validation("api_key is not a valid header value", Some("api_key"))
        })?;
        key.set_sensitive(true);
        headers.insert(HeaderName::from_static("x-api-key"), key);
        headers.insert(reqwest::header::USER_AGENT, header_value(&user_agent()));
        headers.insert(
            HeaderName::from_static("x-client-version"),
            header_value(VERSION),
        );
        headers.insert(
            reqwest::header::ACCEPT,
            HeaderValue::from_static("application/json"),
        );
        headers.insert(
            HeaderName::from_static("x-request-id"),
            header_value(request_id),
        );
        if let Some(etag) = &req.if_none_match {
            let value = HeaderValue::from_str(etag).map_err(|_| {
                Error::validation(
                    "if_none_match is not a valid header value",
                    Some("if_none_match"),
                )
            })?;
            headers.insert(reqwest::header::IF_NONE_MATCH, value);
        }
        let mut builder = self.http.request(req.method.clone(), &url).headers(headers);
        if let Some(body) = &req.body {
            builder = builder
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.clone());
        }
        if self.owns_client {
            builder = builder.timeout(cfg.timeout);
        }
        tracing::debug!(target: "finlight", method = %req.method, path = %req.path, request_id, "request");
        let mut resp = builder.send().await.map_err(transport_error)?;
        let status = resp.status().as_u16();
        let headers = resp.headers().clone();
        let mut bytes: Vec<u8> = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(transport_error)? {
            if bytes.len() + chunk.len() > cfg.max_response_bytes {
                return Err(Error::decode(
                    format!(
                        "the response is larger than max_response_bytes ({})",
                        cfg.max_response_bytes
                    ),
                    None,
                    None,
                    None,
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let body = String::from_utf8(bytes).map_err(|e| {
            Error::decode("the response is not UTF-8", None, None, Some(Box::new(e)))
        })?;
        tracing::debug!(target: "finlight", status, request_id, "response");
        Ok(Response {
            status,
            headers,
            body,
        })
    }
}

fn header_value(value: &str) -> HeaderValue {
    HeaderValue::from_str(value).unwrap_or_else(|_| HeaderValue::from_static("invalid"))
}

fn transport_error(e: reqwest::Error) -> Error {
    let message = if e.is_timeout() {
        "the request timed out".to_owned()
    } else if e.is_connect() {
        "could not connect to the API".to_owned()
    } else {
        "the request failed on the network".to_owned()
    };
    if e.is_timeout() {
        Error::timeout(message, Some(Box::new(e.without_url())))
    } else {
        Error::network(message, Some(Box::new(e.without_url())))
    }
}

/// Exponential backoff with jitter: `min(max, base * 2^n)` times 0.5 to 1.0.
pub(crate) fn backoff(base: Duration, max: Duration, n: u32) -> Duration {
    let factor = 2u32.saturating_pow(n.min(20));
    let capped = base.saturating_mul(factor).min(max);
    capped.mul_f64(0.5 + fastrand::f64() * 0.5)
}

fn retry_class(err: &ApiError) -> RetryClass {
    match err.status() {
        429 => match err.limit_type() {
            Some(RateLimitType::Burst) => RetryClass::Burst,
            _ => RetryClass::Never,
        },
        s if s >= 500 => RetryClass::Backoff,
        _ => RetryClass::Never,
    }
}

fn str_field(obj: &Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key).and_then(Value::as_str).map(str::to_owned)
}

pub(crate) fn field_issues(value: Option<&Value>) -> Vec<FieldIssue> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(Value::as_object)
        .map(|e| FieldIssue {
            field: str_field(e, "field"),
            issue: str_field(e, "issue"),
            allowed_values: e
                .get("allowedValues")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                }),
            constraints: None,
        })
        .collect()
}

/// Classifies a 429 exactly once: the `limitType` member first, then the detail text.
pub(crate) fn classify_429(limit_type: Option<String>, detail: Option<&str>) -> RateLimitType {
    if let Some(value) = limit_type {
        return RateLimitType::from(value);
    }
    match detail {
        Some(d) if d.contains("Exceeded token limit") => RateLimitType::Monthly,
        Some(d) if d.contains("Exceeded rate limit") => RateLimitType::Burst,
        _ => RateLimitType::Unknown,
    }
}

fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    let value = headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .to_owned();
    let secs: f64 = value.parse().ok()?;
    if secs.is_finite() && secs >= 0.0 {
        Some(Duration::from_secs_f64(secs))
    } else {
        None
    }
}

/// Reads an error body: problem+json, then the `{statusCode, message}` form, then text.
pub(crate) fn api_error_from_response(
    status: u16,
    headers: &HeaderMap,
    body: &str,
    request_id: &str,
) -> ApiError {
    let content_type = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let json = serde_json::from_str::<Value>(body).ok();
    let obj = json.as_ref().and_then(Value::as_object);

    let mut code = None;
    let mut r#type = None;
    let mut title = None;
    let mut detail = None;
    let mut errors = Vec::new();
    let mut server_request_id = None;
    let mut limit_member = None;
    let api_kind;
    match obj {
        Some(o)
            if content_type.contains("problem+json")
                || (!o.contains_key("statusCode")
                    && (o.contains_key("code") || o.contains_key("type"))) =>
        {
            code = str_field(o, "code").map(ProblemCode::from);
            r#type = str_field(o, "type");
            title = str_field(o, "title");
            detail = str_field(o, "detail");
            errors = field_issues(o.get("errors"));
            server_request_id = str_field(o, "requestId");
            limit_member = str_field(o, "limitType");
            api_kind = code
                .as_ref()
                .map(ApiErrorKind::from_code)
                .unwrap_or(ApiErrorKind::Other);
        }
        Some(o) if o.contains_key("statusCode") || o.contains_key("message") => {
            detail = match o.get("message") {
                Some(Value::String(s)) => Some(s.clone()),
                Some(Value::Array(items)) => Some(
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("; "),
                ),
                _ => None,
            };
            title = str_field(o, "error");
            server_request_id = str_field(o, "requestId");
            api_kind = ApiErrorKind::from_status(status);
        }
        _ => {
            api_kind = ApiErrorKind::Other;
        }
    }
    let (limit_type, retry_after) = if status == 429 {
        (
            Some(classify_429(limit_member, detail.as_deref())),
            parse_retry_after(headers),
        )
    } else {
        (None, None)
    };
    let request_id = server_request_id.unwrap_or_else(|| request_id.to_owned());
    let mut message = format!("finlight API error {status}");
    if let Some(c) = &code {
        message.push_str(&format!(" {c}"));
    }
    if let Some(text) = detail.as_ref().or(title.as_ref()) {
        message.push_str(&format!(": {text}"));
    }
    message.push_str(&format!(" (request {request_id})"));
    ApiError::new(ApiErrorData {
        status,
        code,
        r#type,
        title,
        detail,
        request_id: Some(request_id),
        errors,
        api_kind,
        limit_type,
        retry_after,
        raw: if body.is_empty() {
            None
        } else {
            Some(body.to_owned())
        },
        message,
    })
}
