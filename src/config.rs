use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::ws::WsConnector;

/// What the SDK does on a 429 `burst`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum RetryOnBurst {
    /// Wait exactly `Retry-After` (`burst_wait` when the header is missing), then retry once.
    /// This retry does not count against `max_retries`.
    #[default]
    Once,
    /// Return the error at once.
    Off,
}

/// Configuration of a [`Client`](crate::Client). Build it with [`Config::builder`].
///
/// Durations are `std::time::Duration`. Every value can be overridden.
#[derive(Clone)]
#[non_exhaustive]
pub struct Config {
    api_key: String,
    /// REST base URL. Default `https://api.finlight.me`.
    pub base_url: String,
    /// WebSocket URL. Default `wss://wss.finlight.me`; the raw stream uses `<wss_url>/raw`.
    pub wss_url: String,
    /// Timeout of one REST attempt. Default 30 s. Ignored with your own `http_client`.
    pub timeout: Duration,
    /// Upper bound of one REST call, retries and burst wait included. Default none.
    pub total_timeout: Option<Duration>,
    /// Retries after the first attempt for 5xx, network errors and attempt timeouts. Default 3.
    pub max_retries: u32,
    /// First backoff delay. Default 0.5 s.
    pub retry_base_delay: Duration,
    /// Largest backoff delay. Default 8 s.
    pub retry_max_delay: Duration,
    /// Behaviour on a 429 `burst`. Default [`RetryOnBurst::Once`].
    pub retry_on_burst: RetryOnBurst,
    /// Wait on a 429 `burst` without `Retry-After`. Default 10 s.
    pub burst_wait: Duration,
    /// Page limit of `search_all`. Default 10.
    pub max_pages: u32,
    /// Largest REST response body read. Default 32 MiB.
    pub max_response_bytes: usize,
    /// Interval of the application ping on streams. Default 25 s.
    pub ping_interval: Duration,
    /// A stream connection without a pong for this long is replaced. Default 60 s.
    pub watchdog_timeout: Duration,
    /// A stream connection is renewed after this long (the gateway ends it after 2 h). Default 115 min.
    pub rotation_interval: Duration,
    /// First reconnect delay of a stream. Default 1 s.
    pub reconnect_base_delay: Duration,
    /// Largest reconnect delay. Default 30 s.
    pub reconnect_max_delay: Duration,
    /// Wait after a refused dial because of the connection limit. Default 60 s.
    pub long_backoff: Duration,
    /// When the WebSocket quota is used up, keep the stream and redial every
    /// `quota_reconnect_interval`. `false` ends the stream with the error. Default `true`.
    pub quota_reconnect: bool,
    /// Interval of quota redials. Default 1 h.
    pub quota_reconnect_interval: Duration,
    /// After this many `SEARCH_FAILED` in a row, the error goes to `on_error`. Default 5.
    pub search_failed_threshold: u32,
    /// Entries of the duplicate filter of each stream. Default 1000.
    pub dedupe_cache_size: usize,
    /// Largest incoming stream message. Default 32 MiB.
    pub max_message_bytes: usize,
    /// Timeout of the WebSocket dial and handshake. Default 30 s.
    pub connect_timeout: Duration,
    /// Your own `reqwest::Client` for REST. Its own timeouts apply; the SDK never closes it.
    pub http_client: Option<reqwest::Client>,
    /// Your own transport for WebSocket dials (e.g. through a proxy).
    pub ws_connector: Option<Arc<dyn WsConnector>>,
}

impl Config {
    /// A configuration with the defaults and this API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Config {
            api_key: api_key.into(),
            base_url: "https://api.finlight.me".to_owned(),
            wss_url: "wss://wss.finlight.me".to_owned(),
            timeout: Duration::from_secs(30),
            total_timeout: None,
            max_retries: 3,
            retry_base_delay: Duration::from_millis(500),
            retry_max_delay: Duration::from_secs(8),
            retry_on_burst: RetryOnBurst::Once,
            burst_wait: Duration::from_secs(10),
            max_pages: 10,
            max_response_bytes: 32 * 1024 * 1024,
            ping_interval: Duration::from_secs(25),
            watchdog_timeout: Duration::from_secs(60),
            rotation_interval: Duration::from_secs(115 * 60),
            reconnect_base_delay: Duration::from_secs(1),
            reconnect_max_delay: Duration::from_secs(30),
            long_backoff: Duration::from_secs(60),
            quota_reconnect: true,
            quota_reconnect_interval: Duration::from_secs(3600),
            search_failed_threshold: 5,
            dedupe_cache_size: 1000,
            max_message_bytes: 32 * 1024 * 1024,
            connect_timeout: Duration::from_secs(30),
            http_client: None,
            ws_connector: None,
        }
    }

    /// A builder with the defaults and this API key.
    pub fn builder(api_key: impl Into<String>) -> ConfigBuilder {
        ConfigBuilder(Config::new(api_key))
    }

    pub(crate) fn api_key(&self) -> &str {
        &self.api_key
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.api_key.trim().is_empty() {
            return Err(Error::validation(
                "api_key must not be empty",
                Some("api_key"),
            ));
        }
        if self.api_key.chars().any(|c| !c.is_ascii_graphic()) {
            return Err(Error::validation(
                "api_key may only contain printable ASCII characters",
                Some("api_key"),
            ));
        }
        for (name, url, schemes) in [
            ("base_url", &self.base_url, &["http://", "https://"]),
            ("wss_url", &self.wss_url, &["ws://", "wss://"]),
        ] {
            if !schemes.iter().any(|s| url.starts_with(s)) || reqwest::Url::parse(url).is_err() {
                return Err(Error::validation(
                    format!("{name} must be an absolute {} URL", schemes.join(" or ")),
                    Some(name),
                ));
            }
        }
        if self.max_pages == 0 {
            return Err(Error::validation(
                "max_pages must be at least 1",
                Some("max_pages"),
            ));
        }
        if self.max_message_bytes < 16 * 1024 * 1024 {
            return Err(Error::validation(
                "max_message_bytes must be at least 16 MiB (the initial push has no size limit)",
                Some("max_message_bytes"),
            ));
        }
        for (name, d) in [
            ("ping_interval", self.ping_interval),
            ("watchdog_timeout", self.watchdog_timeout),
            ("rotation_interval", self.rotation_interval),
            ("quota_reconnect_interval", self.quota_reconnect_interval),
            ("connect_timeout", self.connect_timeout),
        ] {
            if d.is_zero() {
                return Err(Error::validation(
                    format!("{name} must be greater than zero"),
                    Some(name),
                ));
            }
        }
        Ok(())
    }

    /// `base_url` without a trailing slash.
    pub(crate) fn rest_base(&self) -> &str {
        self.base_url.trim_end_matches('/')
    }

    /// The enriched or raw stream URL.
    pub(crate) fn stream_url(&self, raw: bool) -> String {
        let base = self.wss_url.trim_end_matches('/');
        if raw {
            format!("{base}/raw")
        } else {
            format!("{base}/")
        }
    }
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The API key is never printed.
        f.debug_struct("Config")
            .field("api_key", &"[redacted]")
            .field("base_url", &self.base_url)
            .field("wss_url", &self.wss_url)
            .field("timeout", &self.timeout)
            .field("total_timeout", &self.total_timeout)
            .field("max_retries", &self.max_retries)
            .field("retry_base_delay", &self.retry_base_delay)
            .field("retry_max_delay", &self.retry_max_delay)
            .field("retry_on_burst", &self.retry_on_burst)
            .field("burst_wait", &self.burst_wait)
            .field("max_pages", &self.max_pages)
            .field("max_response_bytes", &self.max_response_bytes)
            .field("ping_interval", &self.ping_interval)
            .field("watchdog_timeout", &self.watchdog_timeout)
            .field("rotation_interval", &self.rotation_interval)
            .field("reconnect_base_delay", &self.reconnect_base_delay)
            .field("reconnect_max_delay", &self.reconnect_max_delay)
            .field("long_backoff", &self.long_backoff)
            .field("quota_reconnect", &self.quota_reconnect)
            .field("quota_reconnect_interval", &self.quota_reconnect_interval)
            .field("search_failed_threshold", &self.search_failed_threshold)
            .field("dedupe_cache_size", &self.dedupe_cache_size)
            .field("max_message_bytes", &self.max_message_bytes)
            .field("connect_timeout", &self.connect_timeout)
            .field("http_client", &self.http_client.as_ref().map(|_| "custom"))
            .field(
                "ws_connector",
                &self.ws_connector.as_ref().map(|_| "custom"),
            )
            .finish()
    }
}

/// Builder of [`Config`].
#[derive(Debug, Clone)]
pub struct ConfigBuilder(Config);

macro_rules! setters {
    ($( $(#[$m:meta])* $name:ident : $ty:ty ),* $(,)?) => {
        $(
            $(#[$m])*
            pub fn $name(mut self, value: $ty) -> Self {
                self.0.$name = value;
                self
            }
        )*
    };
}

impl ConfigBuilder {
    /// REST base URL.
    pub fn base_url(mut self, value: impl Into<String>) -> Self {
        self.0.base_url = value.into();
        self
    }

    /// WebSocket URL.
    pub fn wss_url(mut self, value: impl Into<String>) -> Self {
        self.0.wss_url = value.into();
        self
    }

    /// Upper bound of one REST call.
    pub fn total_timeout(mut self, value: Duration) -> Self {
        self.0.total_timeout = Some(value);
        self
    }

    /// Your own `reqwest::Client` for REST (its own timeouts apply).
    pub fn http_client(mut self, value: reqwest::Client) -> Self {
        self.0.http_client = Some(value);
        self
    }

    /// Your own transport for WebSocket dials.
    pub fn ws_connector(mut self, value: impl WsConnector) -> Self {
        self.0.ws_connector = Some(Arc::new(value));
        self
    }

    setters! {
        /// Timeout of one REST attempt.
        timeout: Duration,
        /// Retries for 5xx, network errors and attempt timeouts.
        max_retries: u32,
        /// First backoff delay.
        retry_base_delay: Duration,
        /// Largest backoff delay.
        retry_max_delay: Duration,
        /// Behaviour on a 429 `burst`.
        retry_on_burst: RetryOnBurst,
        /// Wait on a 429 `burst` without `Retry-After`.
        burst_wait: Duration,
        /// Page limit of `search_all`.
        max_pages: u32,
        /// Largest REST response body read.
        max_response_bytes: usize,
        /// Interval of the stream ping.
        ping_interval: Duration,
        /// Pong watchdog of streams.
        watchdog_timeout: Duration,
        /// Connection renewal of streams.
        rotation_interval: Duration,
        /// First reconnect delay.
        reconnect_base_delay: Duration,
        /// Largest reconnect delay.
        reconnect_max_delay: Duration,
        /// Wait after a dial refused for the connection limit.
        long_backoff: Duration,
        /// Redial while the WebSocket quota is used up (default `true`).
        quota_reconnect: bool,
        /// Interval of quota redials.
        quota_reconnect_interval: Duration,
        /// `SEARCH_FAILED` in a row before `on_error`.
        search_failed_threshold: u32,
        /// Entries of the duplicate filter of each stream.
        dedupe_cache_size: usize,
        /// Largest incoming stream message (at least 16 MiB).
        max_message_bytes: usize,
        /// Timeout of the WebSocket dial.
        connect_timeout: Duration,
    }

    /// Checks the configuration and returns it.
    pub fn build(self) -> Result<Config> {
        self.0.validate()?;
        Ok(self.0)
    }
}
