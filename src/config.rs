use std::time::Duration;

/// Default REST endpoint.
pub const DEFAULT_BASE_URL: &str = "https://api.finlight.me";
/// Default WebSocket endpoint.
pub const DEFAULT_WSS_URL: &str = "wss://wss.finlight.me";
/// Default per-request timeout.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
/// Default total request attempts.
pub const DEFAULT_RETRY_COUNT: u32 = 3;

/// Configures the finlight client. Only `api_key` is required; the remaining
/// fields default to the documented values shared by all sibling clients.
#[derive(Clone, Debug)]
pub struct Config {
    /// Your finlight API key (required).
    pub api_key: String,
    /// REST endpoint, default `https://api.finlight.me`.
    pub base_url: String,
    /// WebSocket endpoint, default `wss://wss.finlight.me`.
    pub wss_url: String,
    /// Per-request timeout (also the WebSocket dial timeout), default 5s.
    pub timeout: Duration,
    /// Total request attempts (initial try + retries), default 3.
    pub retry_count: u32,
}

impl Config {
    /// Returns a config with the given API key and default endpoints,
    /// timeout, and retry count.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_owned(),
            wss_url: DEFAULT_WSS_URL.to_owned(),
            timeout: DEFAULT_TIMEOUT,
            retry_count: DEFAULT_RETRY_COUNT,
        }
    }
}
