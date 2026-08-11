use reqwest::StatusCode;

/// Errors returned by the finlight client.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The API key is missing or empty.
    #[error("finlight: Config.api_key is required")]
    MissingApiKey,

    /// A non-2xx REST response (after retries).
    #[error("finlight: API error: {status}")]
    Api {
        /// HTTP status code of the failed response.
        status: StatusCode,
        /// Raw response body, useful for diagnosing the failure.
        body: String,
    },

    /// The HTTP request itself failed (connection, TLS, timeout, ...).
    #[error("finlight: request failed: {0}")]
    Http(#[from] reqwest::Error),

    /// A 2xx response body could not be decoded.
    #[error("finlight: decode response: {0}")]
    Decode(#[from] serde_json::Error),

    /// The server permanently rejected the WebSocket connection (close code
    /// 1008). Reconnecting will not help; contact finlight support.
    #[error("finlight: connection rejected by server (blocked)")]
    Blocked,

    /// A WebSocket protocol failure that ended the stream.
    #[error("finlight: websocket error: {0}")]
    WebSocket(String),
}

/// Returned by [`construct_webhook_event`](crate::construct_webhook_event)
/// when a webhook fails signature, timestamp, or payload validation.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct WebhookVerificationError(pub(crate) String);
