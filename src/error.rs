//! Errors. Every error the SDK returns is one [`Error`] variant; [`Error::kind`] names it as text.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use crate::vocab::{PreemptReason, ProblemCode, RateLimitType};

/// Result type of this crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

type Source = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Every error of this SDK. Cancelling a call is not an error: drop its future.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The API answered with an error: a REST error, a refused WebSocket dial or a
    /// terminal `connection.error` frame.
    Api(ApiError),
    /// A local check failed before any request or dial.
    Validation(ValidationError),
    /// The network failed (connect, TLS, reset, body read).
    Network(NetworkError),
    /// An attempt or the whole call ran out of time.
    Timeout(TimeoutError),
    /// A response could not be decoded.
    Decode(DecodeError),
    /// A webhook did not pass verification.
    WebhookVerification(WebhookVerificationError),
    /// The stream was preempted by the server.
    Preempted(PreemptedError),
}

impl Error {
    /// The kind as text: `api`, `validation`, `network`, `timeout`, `decode`,
    /// `webhook_verification` or `preempted`.
    pub fn kind(&self) -> &'static str {
        match self {
            Error::Api(_) => "api",
            Error::Validation(_) => "validation",
            Error::Network(_) => "network",
            Error::Timeout(_) => "timeout",
            Error::Decode(_) => "decode",
            Error::WebhookVerification(_) => "webhook_verification",
            Error::Preempted(_) => "preempted",
        }
    }

    /// The API error, when this is one.
    pub fn as_api(&self) -> Option<&ApiError> {
        match self {
            Error::Api(e) => Some(e),
            _ => None,
        }
    }

    pub(crate) fn validation(message: impl Into<String>, param: Option<&str>) -> Self {
        Error::Validation(ValidationError {
            message: message.into(),
            param: param.map(str::to_owned),
        })
    }

    pub(crate) fn network(message: impl Into<String>, source: Option<Source>) -> Self {
        Error::Network(NetworkError {
            message: message.into(),
            source,
        })
    }

    pub(crate) fn timeout(message: impl Into<String>, source: Option<Source>) -> Self {
        Error::Timeout(TimeoutError {
            message: message.into(),
            source,
        })
    }

    pub(crate) fn decode(
        message: impl Into<String>,
        path: Option<String>,
        raw: Option<String>,
        source: Option<Source>,
    ) -> Self {
        Error::Decode(DecodeError {
            message: message.into(),
            path,
            raw,
            source,
        })
    }

    pub(crate) fn webhook(reason: WebhookVerificationReason, message: impl Into<String>) -> Self {
        Error::WebhookVerification(WebhookVerificationError {
            reason,
            message: message.into(),
        })
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Api(e) => e.fmt(f),
            Error::Validation(e) => e.fmt(f),
            Error::Network(e) => e.fmt(f),
            Error::Timeout(e) => e.fmt(f),
            Error::Decode(e) => e.fmt(f),
            Error::WebhookVerification(e) => e.fmt(f),
            Error::Preempted(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Network(e) => e.source(),
            Error::Timeout(e) => e.source(),
            Error::Decode(e) => e.source(),
            _ => None,
        }
    }
}

/// Kind of an API error, from the problem `code` or, without one, from the status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ApiErrorKind {
    /// `VALIDATION_FAILED`, or a 400 without a code.
    ValidationFailed,
    /// `CURSOR_INVALID`.
    CursorInvalid,
    /// `CURSOR_MISMATCH`.
    CursorMismatch,
    /// `UNAUTHORIZED`, or a refused dial with 401.
    Unauthorized,
    /// `FORBIDDEN`, or a refused dial with 403.
    Forbidden,
    /// `NOT_FOUND`, or a 404 without a code.
    NotFound,
    /// `RATE_LIMITED`, or a refused dial with 429.
    RateLimited,
    /// `INTERNAL_ERROR`, or a refused dial with 5xx.
    InternalError,
    /// An unknown code, or no code at all.
    Other,
}

impl ApiErrorKind {
    /// The kind as text, e.g. `not_found`.
    pub fn as_str(&self) -> &'static str {
        match self {
            ApiErrorKind::ValidationFailed => "validation_failed",
            ApiErrorKind::CursorInvalid => "cursor_invalid",
            ApiErrorKind::CursorMismatch => "cursor_mismatch",
            ApiErrorKind::Unauthorized => "unauthorized",
            ApiErrorKind::Forbidden => "forbidden",
            ApiErrorKind::NotFound => "not_found",
            ApiErrorKind::RateLimited => "rate_limited",
            ApiErrorKind::InternalError => "internal_error",
            ApiErrorKind::Other => "other",
        }
    }

    pub(crate) fn from_code(code: &ProblemCode) -> Self {
        match code {
            ProblemCode::ValidationFailed => ApiErrorKind::ValidationFailed,
            ProblemCode::CursorInvalid => ApiErrorKind::CursorInvalid,
            ProblemCode::CursorMismatch => ApiErrorKind::CursorMismatch,
            ProblemCode::Unauthorized => ApiErrorKind::Unauthorized,
            ProblemCode::Forbidden => ApiErrorKind::Forbidden,
            ProblemCode::NotFound => ApiErrorKind::NotFound,
            ProblemCode::RateLimited => ApiErrorKind::RateLimited,
            ProblemCode::InternalError => ApiErrorKind::InternalError,
            _ => ApiErrorKind::Other,
        }
    }

    /// Kind of a refused WebSocket dial or a code-less body, from the HTTP status.
    pub(crate) fn from_status(status: u16) -> Self {
        match status {
            400 => ApiErrorKind::ValidationFailed,
            401 => ApiErrorKind::Unauthorized,
            403 => ApiErrorKind::Forbidden,
            404 => ApiErrorKind::NotFound,
            429 => ApiErrorKind::RateLimited,
            500..=599 => ApiErrorKind::InternalError,
            _ => ApiErrorKind::Other,
        }
    }
}

impl fmt::Display for ApiErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One entry of [`ApiError::errors`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct FieldIssue {
    /// The request property, e.g. `eventTypes`.
    pub field: Option<String>,
    /// What is wrong with it.
    pub issue: Option<String>,
    /// The values the server accepts, for closed vocabularies.
    pub allowed_values: Option<Vec<String>>,
    /// Rule name to text, only on stream errors (`connection.error` details).
    pub constraints: Option<BTreeMap<String, String>>,
}

pub(crate) struct ApiErrorData {
    pub status: u16,
    pub code: Option<ProblemCode>,
    pub r#type: Option<String>,
    pub title: Option<String>,
    pub detail: Option<String>,
    pub request_id: Option<String>,
    pub errors: Vec<FieldIssue>,
    pub api_kind: ApiErrorKind,
    pub limit_type: Option<RateLimitType>,
    pub retry_after: Option<Duration>,
    pub raw: Option<String>,
    pub message: String,
}

/// An error answered by the API.
///
/// For REST it carries the HTTP status and the problem body. A refused WebSocket dial
/// carries the dial status. A `connection.error` frame carries status 0 and the frame's code.
pub struct ApiError(Box<ApiErrorData>);

impl ApiError {
    pub(crate) fn new(data: ApiErrorData) -> Self {
        ApiError(Box::new(data))
    }

    /// HTTP status (0 for a `connection.error` frame, 1008 for a policy close).
    pub fn status(&self) -> u16 {
        self.0.status
    }

    /// Problem code (open), `None` when the body had none.
    pub fn code(&self) -> Option<&ProblemCode> {
        self.0.code.as_ref()
    }

    /// Problem `type` URL, e.g. `https://docs.finlight.me/errors/not-found`.
    pub fn r#type(&self) -> Option<&str> {
        self.0.r#type.as_deref()
    }

    /// Problem title.
    pub fn title(&self) -> Option<&str> {
        self.0.title.as_deref()
    }

    /// Problem detail, or the `message` of the fallback form.
    pub fn detail(&self) -> Option<&str> {
        self.0.detail.as_deref()
    }

    /// Request id: the server's value, else the `X-Request-Id` the SDK sent.
    pub fn request_id(&self) -> Option<&str> {
        self.0.request_id.as_deref()
    }

    /// Field problems of validation and cursor errors.
    pub fn errors(&self) -> &[FieldIssue] {
        &self.0.errors
    }

    /// Kind of the error.
    pub fn api_kind(&self) -> ApiErrorKind {
        self.0.api_kind
    }

    /// For 429: which limit was hit (`monthly`, `burst`, `unknown` or the value the server sent).
    pub fn limit_type(&self) -> Option<&RateLimitType> {
        self.0.limit_type.as_ref()
    }

    /// For 429: the `Retry-After` header, when the server sent one.
    pub fn retry_after(&self) -> Option<Duration> {
        self.0.retry_after
    }

    /// The body as received (text), when there was one.
    pub fn raw(&self) -> Option<&str> {
        self.0.raw.as_deref()
    }

    /// The body as JSON text (same as [`ApiError::raw`]).
    pub fn raw_json(&self) -> Option<&str> {
        self.0.raw.as_deref()
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.message)
    }
}

impl fmt::Debug for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The body is left out on purpose: it is never printed in full.
        f.debug_struct("ApiError")
            .field("status", &self.0.status)
            .field("code", &self.0.code)
            .field("api_kind", &self.0.api_kind)
            .field("title", &self.0.title)
            .field("detail", &self.0.detail)
            .field("request_id", &self.0.request_id)
            .field("errors", &self.0.errors)
            .field("limit_type", &self.0.limit_type)
            .field("retry_after", &self.0.retry_after)
            .field("raw_len", &self.0.raw.as_ref().map(String::len))
            .finish()
    }
}

impl std::error::Error for ApiError {}

/// A local check failed before any request or dial.
#[derive(Debug, Clone)]
pub struct ValidationError {
    message: String,
    param: Option<String>,
}

impl ValidationError {
    /// The rejected parameter, when there is one.
    pub fn param(&self) -> Option<&str> {
        self.param.as_deref()
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ValidationError {}

/// The network failed.
#[derive(Debug)]
pub struct NetworkError {
    message: String,
    source: Option<Source>,
}

impl fmt::Display for NetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for NetworkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|e| e.as_ref() as _)
    }
}

/// An attempt (`timeout`, `connect_timeout`) or the whole call (`total_timeout`) ran out of time.
#[derive(Debug)]
pub struct TimeoutError {
    message: String,
    source: Option<Source>,
}

impl fmt::Display for TimeoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TimeoutError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|e| e.as_ref() as _)
    }
}

/// A response could not be decoded.
pub struct DecodeError {
    message: String,
    path: Option<String>,
    raw: Option<String>,
    source: Option<Source>,
}

impl DecodeError {
    /// JSON path of the value that did not fit, when known (e.g. `article.source.domain`).
    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// The JSON text that did not fit, when available.
    pub fn raw(&self) -> Option<&str> {
        self.raw.as_deref()
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl fmt::Debug for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DecodeError")
            .field("message", &self.message)
            .field("path", &self.path)
            .field("raw_len", &self.raw.as_ref().map(String::len))
            .finish()
    }
}

impl std::error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|e| e.as_ref() as _)
    }
}

/// Why webhook verification failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WebhookVerificationReason {
    /// The body passed is not usable as the raw body.
    InvalidRawBody,
    /// The secret is empty.
    InvalidSecret,
    /// No `X-Webhook-Signature` header.
    MissingSignature,
    /// No `X-Webhook-Timestamp` header.
    MissingTimestamp,
    /// A webhook header appears more than once.
    DuplicateHeader,
    /// A webhook header contains characters outside printable ASCII.
    NonAsciiHeader,
    /// The signature does not start with `sha256=`.
    UnsupportedSignatureScheme,
    /// The timestamp is not RFC 3339 (Unix seconds are rejected too).
    UnparseableTimestamp,
    /// The signature does not match.
    SignatureMismatch,
    /// The timestamp is further away from now than the tolerance.
    TimestampOutsideTolerance,
}

impl WebhookVerificationReason {
    /// The reason as text, e.g. `signature_mismatch`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::InvalidRawBody => "invalid_raw_body",
            Self::InvalidSecret => "invalid_secret",
            Self::MissingSignature => "missing_signature",
            Self::MissingTimestamp => "missing_timestamp",
            Self::DuplicateHeader => "duplicate_header",
            Self::NonAsciiHeader => "non_ascii_header",
            Self::UnsupportedSignatureScheme => "unsupported_signature_scheme",
            Self::UnparseableTimestamp => "unparseable_timestamp",
            Self::SignatureMismatch => "signature_mismatch",
            Self::TimestampOutsideTolerance => "timestamp_outside_tolerance",
        }
    }
}

impl fmt::Display for WebhookVerificationReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A webhook did not pass verification. Answer the request with 400 (never 410).
#[derive(Debug, Clone)]
pub struct WebhookVerificationError {
    reason: WebhookVerificationReason,
    message: String,
}

impl WebhookVerificationError {
    /// Why verification failed.
    pub fn reason(&self) -> WebhookVerificationReason {
        self.reason
    }
}

impl fmt::Display for WebhookVerificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "webhook verification failed ({}): {}",
            self.reason, self.message
        )
    }
}

impl std::error::Error for WebhookVerificationError {}

/// The server preempted the stream (`connection.preempted`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreemptedError {
    reason: PreemptReason,
    new_lease_id: Option<String>,
    timestamp: Option<i64>,
}

impl PreemptedError {
    pub(crate) fn new(
        reason: PreemptReason,
        new_lease_id: Option<String>,
        timestamp: Option<i64>,
    ) -> Self {
        PreemptedError {
            reason,
            new_lease_id,
            timestamp,
        }
    }

    /// `connection_limit` (another connection took over) or `quota_exceeded`.
    pub fn reason(&self) -> &PreemptReason {
        &self.reason
    }

    /// Lease id of the connection that took over, for `connection_limit`.
    pub fn new_lease_id(&self) -> Option<&str> {
        self.new_lease_id.as_deref()
    }

    /// Server time of the preemption in epoch milliseconds.
    pub fn timestamp(&self) -> Option<i64> {
        self.timestamp
    }
}

impl fmt::Display for PreemptedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.reason {
            PreemptReason::ConnectionLimit => write!(
                f,
                "stream preempted (connection_limit): another connection of this API key took over"
            ),
            PreemptReason::QuotaExceeded => write!(
                f,
                "stream preempted (quota_exceeded): the WebSocket quota of the current period is used up"
            ),
            _ => write!(f, "stream preempted ({})", self.reason),
        }
    }
}

impl std::error::Error for PreemptedError {}

impl From<ApiError> for Error {
    fn from(e: ApiError) -> Self {
        Error::Api(e)
    }
}

impl From<PreemptedError> for Error {
    fn from(e: PreemptedError) -> Self {
        Error::Preempted(e)
    }
}
