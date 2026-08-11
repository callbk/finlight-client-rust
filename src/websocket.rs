use std::collections::{HashSet, VecDeque};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Instant, interval_at, sleep, timeout};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::protocol::frame::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async_with_config};

use crate::config::Config;
use crate::error::Error;
use crate::models::{Article, RawArticle};
use crate::params::{GetArticlesWebSocketParams, GetRawArticlesWebSocketParams};
use crate::version::CLIENT_VERSION;

/// Called when a WebSocket connection closes, with the close code and reason.
pub type OnClose = Arc<dyn Fn(u16, &str) + Send + Sync>;

/// Tunes the streaming clients. The defaults match the sibling clients.
#[derive(Clone)]
pub struct WebSocketOptions {
    /// Application-level ping cadence, default 25s.
    pub ping_interval: Duration,
    /// Force reconnect when no pong arrives within this window, default 60s.
    pub pong_timeout: Duration,
    /// First reconnect backoff, default 500ms.
    pub base_reconnect_delay: Duration,
    /// Backoff cap, default 10s.
    pub max_reconnect_delay: Duration,
    /// Proactive connection rotation, default 115min (under the 2h server cap).
    pub connection_lifetime: Duration,
    /// Take over an existing connection for the same key.
    pub takeover: bool,
    /// Called with (code, reason) whenever a connection closes.
    pub on_close: Option<OnClose>,
}

impl Default for WebSocketOptions {
    fn default() -> Self {
        Self {
            ping_interval: Duration::from_secs(25),
            pong_timeout: Duration::from_secs(60),
            base_reconnect_delay: Duration::from_millis(500),
            max_reconnect_delay: Duration::from_secs(10),
            connection_lifetime: Duration::from_secs(115 * 60),
            takeover: false,
            on_close: None,
        }
    }
}

impl std::fmt::Debug for WebSocketOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSocketOptions")
            .field("ping_interval", &self.ping_interval)
            .field("pong_timeout", &self.pong_timeout)
            .field("base_reconnect_delay", &self.base_reconnect_delay)
            .field("max_reconnect_delay", &self.max_reconnect_delay)
            .field("connection_lifetime", &self.connection_lifetime)
            .field("takeover", &self.takeover)
            .field("on_close", &self.on_close.as_ref().map(|_| "Fn"))
            .finish()
    }
}

// Close codes used by the finlight WebSocket protocol.
const CLOSE_PROACTIVE_ROTATION: u16 = 4000;
const CLOSE_RATE_LIMITED: u16 = 4001;
const CLOSE_USER_BLOCKED: u16 = 4002;
const CLOSE_ADMIN_KICK: u16 = 4003;
/// 1008: the server permanently rejected the connection.
const CLOSE_POLICY_VIOLATION: u16 = 1008;

const RECENT_ARTICLE_CACHE_SIZE: usize = 10;
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(5);
const DIAL_RATE_LIMIT_BACKOFF: Duration = Duration::from_secs(60);
const ERROR_RATE_LIMIT_BACKOFF: Duration = Duration::from_secs(60);
const ERROR_BLOCKED_BACKOFF: Duration = Duration::from_secs(60 * 60);
const DEFAULT_ADMIN_KICK_RETRY: Duration = Duration::from_secs(15 * 60);
const MAX_ARTICLE_MESSAGE_SIZE: usize = 16 << 20;

/// Streams enriched articles in real time. Duplicate articles (same link
/// within the last 10 deliveries) are suppressed.
pub struct WebSocketClient {
    cfg: Config,
    opts: WebSocketOptions,
}

impl WebSocketClient {
    /// Returns a streaming client with custom options. Client instances
    /// created by [`Client::new`](crate::Client::new) use default options.
    pub fn new(cfg: Config, opts: WebSocketOptions) -> Self {
        Self { cfg, opts }
    }

    /// Connects to the finlight WebSocket and yields articles matching
    /// `params`. Reconnects (exponential backoff, proactive rotation,
    /// rate-limit waits) are handled internally. `Err` items are terminal
    /// (e.g. [`Error::Blocked`]); the stream ends after yielding one. End the
    /// stream by dropping it.
    ///
    /// Must be called within a tokio runtime.
    pub fn stream(&self, params: GetArticlesWebSocketParams) -> ArticleStream<Article> {
        spawn_stream(
            self.cfg.clone(),
            self.opts.clone(),
            self.cfg.wss_url.clone(),
            &params,
            Some(|a: &Article| a.link.clone()),
        )
    }
}

/// Streams unenriched articles in real time (no sentiment, entities, or
/// content — lower latency). No duplicate suppression.
pub struct RawWebSocketClient {
    cfg: Config,
    opts: WebSocketOptions,
}

impl RawWebSocketClient {
    /// Returns a raw streaming client with custom options. Client instances
    /// created by [`Client::new`](crate::Client::new) use default options.
    pub fn new(cfg: Config, opts: WebSocketOptions) -> Self {
        Self { cfg, opts }
    }

    /// Connects to the raw finlight WebSocket and yields articles matching
    /// `params`. See [`WebSocketClient::stream`] for the streaming semantics.
    ///
    /// Must be called within a tokio runtime.
    pub fn stream(&self, params: GetRawArticlesWebSocketParams) -> ArticleStream<RawArticle> {
        spawn_stream(
            self.cfg.clone(),
            self.opts.clone(),
            format!("{}/raw", self.cfg.wss_url),
            &params,
            None,
        )
    }
}

/// A stream of articles delivered over the finlight WebSocket. Dropping the
/// stream disconnects and stops the background task.
pub struct ArticleStream<T> {
    rx: mpsc::Receiver<Result<T, Error>>,
    handle: JoinHandle<()>,
}

impl<T> futures_core::Stream for ArticleStream<T> {
    type Item = Result<T, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().rx.poll_recv(cx)
    }
}

impl<T> Drop for ArticleStream<T> {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

fn spawn_stream<T>(
    cfg: Config,
    opts: WebSocketOptions,
    url: String,
    params: &impl serde::Serialize,
    identify: Option<fn(&T) -> String>,
) -> ArticleStream<T>
where
    T: DeserializeOwned + Send + 'static,
{
    let payload = match serde_json::to_value(params) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    };
    let (tx, rx) = mpsc::channel(256);
    let handle = tokio::spawn(run_stream(cfg, opts, url, payload, identify, tx));
    ArticleStream { rx, handle }
}

/// FIFO cache of recently seen article keys for duplicate suppression.
struct Dedup {
    order: VecDeque<String>,
    seen: HashSet<String>,
}

impl Dedup {
    fn new() -> Self {
        Self {
            order: VecDeque::new(),
            seen: HashSet::new(),
        }
    }

    /// Records `id`; returns true when it was already tracked.
    fn check_and_track(&mut self, id: String) -> bool {
        if self.seen.contains(&id) {
            return true;
        }
        self.order.push_back(id.clone());
        self.seen.insert(id);
        if self.order.len() > RECENT_ARTICLE_CACHE_SIZE {
            if let Some(old) = self.order.pop_front() {
                self.seen.remove(&old);
            }
        }
        false
    }
}

/// How one connection ended.
enum ConnEnd {
    /// Reconnect; `connected` reports whether the connection was established
    /// (resets the backoff).
    Reconnect { connected: bool },
    /// Stop the reconnect loop, optionally yielding a terminal error.
    Terminal(Option<Error>),
}

/// The reconnect loop: runs connections until the stream ends, waiting
/// between attempts with exponential backoff or until a server-mandated
/// reconnect time.
async fn run_stream<T>(
    cfg: Config,
    opts: WebSocketOptions,
    url: String,
    payload: Map<String, Value>,
    identify: Option<fn(&T) -> String>,
    tx: mpsc::Sender<Result<T, Error>>,
) where
    T: DeserializeOwned + Send + 'static,
{
    let mut delay = opts.base_reconnect_delay;
    let mut reconnect_at: Option<Instant> = None;
    let mut dedup = identify.map(|_| Dedup::new());

    loop {
        if tx.is_closed() {
            return;
        }
        tracing::info!(url = %url, "finlight ws: connecting");
        let end = run_connection(
            &cfg,
            &opts,
            &url,
            &payload,
            identify,
            dedup.as_mut(),
            &mut reconnect_at,
            &tx,
        )
        .await;
        let connected = match end {
            ConnEnd::Terminal(Some(err)) => {
                let _ = tx.send(Err(err)).await;
                return;
            }
            ConnEnd::Terminal(None) => return,
            ConnEnd::Reconnect { connected } => connected,
        };
        if connected {
            delay = opts.base_reconnect_delay;
        }

        let now = Instant::now();
        let wait = match reconnect_at {
            Some(at) if at > now => {
                let wait = at - now;
                tracing::info!(?wait, "finlight ws: waiting until reconnect_at");
                wait
            }
            _ => {
                let wait = delay;
                tracing::info!(delay = ?wait, "finlight ws: reconnecting");
                delay = (delay * 2).min(opts.max_reconnect_delay);
                wait
            }
        };
        tokio::select! {
            _ = sleep(wait) => {}
            _ = tx.closed() => return,
        }
    }
}

type WsSink = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

/// Envelope for every server message.
#[derive(Deserialize)]
struct WsMessage {
    #[serde(default)]
    action: String,
    #[serde(default)]
    t: Option<i64>,
    #[serde(default, rename = "leaseId")]
    lease_id: Option<String>,
    #[serde(default, rename = "clientNonce")]
    client_nonce: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default, rename = "newLeaseId")]
    new_lease_id: Option<String>,
    /// Milliseconds.
    #[serde(default, rename = "retryAfter")]
    retry_after: Option<i64>,
    #[serde(default)]
    data: Option<Value>,
    #[serde(default)]
    error: Option<Value>,
}

#[allow(clippy::too_many_arguments)]
async fn run_connection<T>(
    cfg: &Config,
    opts: &WebSocketOptions,
    url: &str,
    payload: &Map<String, Value>,
    identify: Option<fn(&T) -> String>,
    mut dedup: Option<&mut Dedup>,
    reconnect_at: &mut Option<Instant>,
    tx: &mpsc::Sender<Result<T, Error>>,
) -> ConnEnd
where
    T: DeserializeOwned,
{
    // The server reads these headers case-sensitively in exact lowercase.
    // The http crate always serializes header names in lowercase, so plain
    // inserts are safe here (unlike Go's net/http, which canonicalizes).
    let mut request = match url.into_client_request() {
        Ok(r) => r,
        Err(e) => {
            return ConnEnd::Terminal(Some(Error::WebSocket(format!("invalid URL: {e}"))));
        }
    };
    let api_key = match HeaderValue::from_str(&cfg.api_key) {
        Ok(v) => v,
        Err(_) => return ConnEnd::Terminal(Some(Error::MissingApiKey)),
    };
    let headers = request.headers_mut();
    headers.insert("x-api-key", api_key);
    headers.insert("x-client-version", HeaderValue::from_static(CLIENT_VERSION));
    if opts.takeover {
        headers.insert("x-takeover", HeaderValue::from_static("true"));
    }

    let ws_config = WebSocketConfig::default()
        .max_message_size(Some(MAX_ARTICLE_MESSAGE_SIZE))
        .max_frame_size(Some(MAX_ARTICLE_MESSAGE_SIZE));
    let (ws, _) = match timeout(
        cfg.timeout,
        connect_async_with_config(request, Some(ws_config), false),
    )
    .await
    {
        Err(_) => {
            tracing::error!("finlight ws: connection timed out");
            return ConnEnd::Reconnect { connected: false };
        }
        Ok(Err(WsError::Http(resp))) if resp.status().as_u16() == 429 => {
            *reconnect_at = Some(Instant::now() + DIAL_RATE_LIMIT_BACKOFF);
            tracing::warn!(
                backoff = ?DIAL_RATE_LIMIT_BACKOFF,
                "finlight ws: server rejected connection (429)"
            );
            return ConnEnd::Reconnect { connected: false };
        }
        Ok(Err(e)) => {
            tracing::error!(error = %e, "finlight ws: connection failed");
            return ConnEnd::Reconnect { connected: false };
        }
        Ok(Ok(ok)) => ok,
    };

    tracing::info!("finlight ws: connected");
    *reconnect_at = None;

    let (mut write, mut read) = ws.split();
    let nonce = uuid::Uuid::new_v4().to_string();

    let mut handshake = payload.clone();
    handshake.insert("clientNonce".to_owned(), Value::String(nonce.clone()));
    let handshake = serde_json::to_string(&Value::Object(handshake)).expect("valid JSON");
    if let Err(e) = write.send(Message::text(handshake)).await {
        tracing::error!(error = %e, "finlight ws: handshake write failed");
        return ConnEnd::Reconnect { connected: true };
    }

    let mut last_pong = Instant::now();
    let start = Instant::now();
    let mut ping = interval_at(start + opts.ping_interval, opts.ping_interval);
    let mut watchdog = interval_at(start + WATCHDOG_INTERVAL, WATCHDOG_INTERVAL);
    let rotation = sleep(opts.connection_lifetime);
    tokio::pin!(rotation);

    loop {
        tokio::select! {
            msg = read.next() => match msg {
                Some(Ok(Message::Text(text))) => {
                    if let Some(end) = handle_message(
                        text.as_str(), opts, &nonce, identify, dedup.as_deref_mut(),
                        reconnect_at, tx, &mut write, &mut last_pong,
                    ).await {
                        return end;
                    }
                }
                Some(Ok(Message::Close(frame))) => {
                    let (code, reason) = match &frame {
                        Some(f) => (u16::from(f.code), f.reason.to_string()),
                        None => (1005, String::new()),
                    };
                    tracing::info!(code, reason = %reason, "finlight ws: connection closed");
                    notify_close(opts, code, &reason);
                    if code == CLOSE_POLICY_VIOLATION {
                        tracing::warn!("finlight ws: connection rejected by server (blocked)");
                        return ConnEnd::Terminal(Some(Error::Blocked));
                    }
                    return ConnEnd::Reconnect { connected: true };
                }
                Some(Ok(_)) => {} // binary/ping/pong frames: not part of the protocol
                Some(Err(e)) => {
                    tracing::info!(error = %e, "finlight ws: connection closed");
                    notify_close(opts, 1006, "");
                    return ConnEnd::Reconnect { connected: true };
                }
                None => {
                    tracing::info!("finlight ws: connection closed");
                    notify_close(opts, 1006, "");
                    return ConnEnd::Reconnect { connected: true };
                }
            },
            _ = ping.tick() => {
                let msg = json!({"action": "ping", "t": chrono::Utc::now().timestamp_millis()});
                if let Err(e) = write.send(Message::text(msg.to_string())).await {
                    tracing::debug!(error = %e, "finlight ws: ping failed");
                }
            }
            _ = watchdog.tick() => {
                if last_pong.elapsed() > opts.pong_timeout {
                    tracing::warn!("finlight ws: no pong received in time, forcing reconnect");
                    close(&mut write, 1000, "pong timeout").await;
                    notify_close(opts, 1000, "pong timeout");
                    return ConnEnd::Reconnect { connected: true };
                }
            }
            _ = &mut rotation => {
                tracing::info!("finlight ws: proactive rotation before server connection cap");
                close(&mut write, CLOSE_PROACTIVE_ROTATION, "Proactive rotation").await;
                notify_close(opts, CLOSE_PROACTIVE_ROTATION, "Proactive rotation");
                return ConnEnd::Reconnect { connected: true };
            }
            _ = tx.closed() => {
                close(&mut write, 1000, "client stopped").await;
                notify_close(opts, 1000, "client stopped");
                return ConnEnd::Terminal(None);
            }
        }
    }
}

/// Dispatches one server message. Returns `Some` when the connection is done.
#[allow(clippy::too_many_arguments)]
async fn handle_message<T>(
    text: &str,
    opts: &WebSocketOptions,
    nonce: &str,
    identify: Option<fn(&T) -> String>,
    dedup: Option<&mut Dedup>,
    reconnect_at: &mut Option<Instant>,
    tx: &mpsc::Sender<Result<T, Error>>,
    write: &mut WsSink,
    last_pong: &mut Instant,
) -> Option<ConnEnd>
where
    T: DeserializeOwned,
{
    let msg: WsMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %e, "finlight ws: cannot parse message");
            return None;
        }
    };

    match msg.action.as_str() {
        "pong" => {
            match msg.t {
                Some(t) if t > 0 => {
                    let rtt = chrono::Utc::now().timestamp_millis() - t;
                    tracing::debug!(rtt_ms = rtt, "finlight ws: pong received");
                }
                _ => tracing::debug!("finlight ws: pong received"),
            }
            *last_pong = Instant::now();
        }

        "admit" => {
            tracing::info!(lease_id = ?msg.lease_id, "finlight ws: admitted");
            match &msg.client_nonce {
                Some(got) if got != nonce => {
                    tracing::warn!(expected = nonce, got = %got, "finlight ws: nonce mismatch");
                }
                _ => {}
            }
        }

        "preempted" => {
            tracing::warn!(
                reason = ?msg.reason,
                new_lease_id = ?msg.new_lease_id,
                "finlight ws: connection preempted"
            );
            close(write, 1000, "Preempted by server").await;
            notify_close(opts, 1000, "client stopped");
            return Some(ConnEnd::Terminal(None));
        }

        "sendArticle" => {
            let article: T = match serde_json::from_value(msg.data.unwrap_or(Value::Null)) {
                Ok(a) => a,
                Err(e) => {
                    tracing::error!(error = %e, "finlight ws: cannot parse article");
                    return None;
                }
            };
            if let (Some(identify), Some(dedup)) = (identify, dedup) {
                let id = identify(&article);
                if dedup.check_and_track(id.clone()) {
                    tracing::debug!(id = %id, "finlight ws: skipping duplicate article");
                    return None;
                }
            }
            if tx.send(Ok(article)).await.is_err() {
                // Consumer dropped the stream.
                close(write, 1000, "client stopped").await;
                notify_close(opts, 1000, "client stopped");
                return Some(ConnEnd::Terminal(None));
            }
        }

        "admin_kick" => {
            let retry_after = match msg.retry_after {
                Some(ms) if ms > 0 => Duration::from_millis(ms as u64),
                _ => DEFAULT_ADMIN_KICK_RETRY,
            };
            *reconnect_at = Some(Instant::now() + retry_after);
            tracing::warn!(?retry_after, "finlight ws: admin kick");
            close(write, CLOSE_ADMIN_KICK, "Admin kick").await;
            notify_close(opts, CLOSE_ADMIN_KICK, "Admin kick");
            return Some(ConnEnd::Reconnect { connected: true });
        }

        "error" => {
            let err_text = value_to_string(msg.data.as_ref())
                .or_else(|| value_to_string(msg.error.as_ref()))
                .unwrap_or_default();
            tracing::error!(error = %err_text, "finlight ws: server error");
            let lowered = err_text.to_lowercase();
            if lowered.contains("limit") {
                *reconnect_at = Some(Instant::now() + ERROR_RATE_LIMIT_BACKOFF);
                close(write, CLOSE_RATE_LIMITED, "Rate limited").await;
                notify_close(opts, CLOSE_RATE_LIMITED, "Rate limited");
                return Some(ConnEnd::Reconnect { connected: true });
            } else if lowered.contains("blocked") {
                *reconnect_at = Some(Instant::now() + ERROR_BLOCKED_BACKOFF);
                close(write, CLOSE_USER_BLOCKED, "User blocked").await;
                notify_close(opts, CLOSE_USER_BLOCKED, "User blocked");
                return Some(ConnEnd::Reconnect { connected: true });
            }
        }

        action => {
            tracing::warn!(action = %action, "finlight ws: unknown message action");
        }
    }
    None
}

async fn close(write: &mut WsSink, code: u16, reason: &str) {
    let frame = CloseFrame {
        code: CloseCode::from(code),
        reason: reason.to_owned().into(),
    };
    let _ = write.send(Message::Close(Some(frame))).await;
}

fn notify_close(opts: &WebSocketOptions, code: u16, reason: &str) {
    if let Some(on_close) = &opts.on_close {
        on_close(code, reason);
    }
}

/// Renders a JSON value that may be a string or arbitrary JSON.
fn value_to_string(v: Option<&Value>) -> Option<String> {
    match v {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(other) => Some(other.to_string()),
    }
}
