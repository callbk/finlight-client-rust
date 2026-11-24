//! Real-time streams (`client.stream()`, `client.raw_stream()`).
//!
//! One background task per stream owns the socket. It answers control frames (pong, admitted,
//! preempted, errors) before your code sees anything, so a slow consumer never trips the
//! watchdog. It pings every `ping_interval`, replaces a connection without a pong for
//! `watchdog_timeout`, renews the connection after `rotation_interval` and redials with the
//! same subscribe after every drop. Duplicates are filtered per stream.

use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime};

use futures_core::Stream;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, Interval, MissedTickBehavior};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use crate::config::Config;
use crate::error::{
    ApiError, ApiErrorData, ApiErrorKind, Error, FieldIssue, PreemptedError, Result,
};
use crate::frames::{self, Frame};
use crate::http::backoff;
use crate::models::{StreamEvent, StreamEventKind, decode_article};
use crate::time::{epoch_millis, parse_rfc3339_nanos};
use crate::vocab::{ConnectionErrorCode, PreemptReason, ProblemCode, ReconnectReason};
use crate::ws::{DialOutcome, WsStream, dial};

type Hook<T> = Arc<dyn Fn(&T) + Send + Sync>;

/// Sent with every `connection.admitted`. It arrives before the subscribe is validated, so it
/// does not mean the subscribe was accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AdmittedInfo {
    /// Lease id of the connection.
    pub lease_id: Option<String>,
    /// Server time in epoch milliseconds.
    pub server_now: Option<i64>,
    /// The `clientNonce` echoed back.
    pub client_nonce: Option<String>,
}

/// Sent after every new connection except the first one.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReconnectInfo {
    /// Last timestamp seen on this stream: enriched `availableAt` (falling back to
    /// `firstSeenAt`), raw `firstSeenAt`; `None` when nothing arrived yet. Use it to fill gaps
    /// through `articles().search()`; the SDK does not backfill.
    pub last_seen_at: Option<String>,
    /// Why the stream connected again.
    pub reason: ReconnectReason,
    /// Connections this stream opened so far, the new one included.
    pub connection: u32,
}

/// Sent each time a stream waits for the WebSocket quota of the next period.
#[derive(Debug)]
#[non_exhaustive]
pub struct QuotaWaitInfo {
    /// The preemption (`Error::Preempted`) or refused dial (`Error::Api`, 403) that caused the wait.
    pub error: Error,
    /// Time until the next attempt.
    pub next_attempt_in: Duration,
    /// Quota waits in a row, starting at 1.
    pub attempt: u32,
}

/// Options of a stream: `takeover` and hooks. Hooks run on the stream's task; a panic in a
/// hook is caught and logged and never ends the stream.
#[derive(Clone, Default)]
pub struct StreamOptions {
    takeover: bool,
    on_admitted: Option<Hook<AdmittedInfo>>,
    on_reconnect: Option<Hook<ReconnectInfo>>,
    on_error: Option<Hook<Error>>,
    on_quota_wait: Option<Hook<QuotaWaitInfo>>,
}

impl StreamOptions {
    /// No takeover, no hooks.
    pub fn new() -> Self {
        Self::default()
    }

    /// Send `x-takeover: true`: at the connection limit, replace your stalest connection.
    /// Default off.
    pub fn takeover(mut self, value: bool) -> Self {
        self.takeover = value;
        self
    }

    /// Called for every `connection.admitted`.
    pub fn on_admitted(mut self, hook: impl Fn(&AdmittedInfo) + Send + Sync + 'static) -> Self {
        self.on_admitted = Some(Arc::new(hook));
        self
    }

    /// Called after every new connection except the first one.
    pub fn on_reconnect(mut self, hook: impl Fn(&ReconnectInfo) + Send + Sync + 'static) -> Self {
        self.on_reconnect = Some(Arc::new(hook));
        self
    }

    /// Called for problems that do not end the stream.
    pub fn on_error(mut self, hook: impl Fn(&Error) + Send + Sync + 'static) -> Self {
        self.on_error = Some(Arc::new(hook));
        self
    }

    /// Called for every wait for the WebSocket quota (see `Config::quota_reconnect`).
    pub fn on_quota_wait(mut self, hook: impl Fn(&QuotaWaitInfo) + Send + Sync + 'static) -> Self {
        self.on_quota_wait = Some(Arc::new(hook));
        self
    }
}

impl fmt::Debug for StreamOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamOptions")
            .field("takeover", &self.takeover)
            .field("on_admitted", &self.on_admitted.is_some())
            .field("on_reconnect", &self.on_reconnect.is_some())
            .field("on_error", &self.on_error.is_some())
            .field("on_quota_wait", &self.on_quota_wait.is_some())
            .finish()
    }
}

struct Shared {
    last_seen_at: Mutex<Option<String>>,
    closed: AtomicBool,
}

/// A live stream of [`StreamEvent`]s.
///
/// Poll it with `while let Some(e) = stream.next().await` (`futures_util::StreamExt`). A
/// terminal problem (refused key, plan without WebSockets, validation, a `connection_limit`
/// preemption, a used-up quota with `quota_reconnect` off) arrives as one `Err`, then the
/// stream ends with `None`. Dropping the stream or calling [`FinlightStream::close`] closes the
/// socket with 1000 and stops all background work.
pub struct FinlightStream {
    rx: mpsc::UnboundedReceiver<Result<StreamEvent>>,
    stop: Option<watch::Sender<bool>>,
    task: Option<JoinHandle<()>>,
    shared: Arc<Shared>,
    finished: bool,
}

impl FinlightStream {
    /// Stops the stream: closes the socket with 1000 and waits until the background task ended.
    pub async fn close(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(true);
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
        self.finished = true;
        self.shared.closed.store(true, Ordering::SeqCst);
    }

    /// Last timestamp seen (see [`ReconnectInfo::last_seen_at`]).
    pub fn last_seen_at(&self) -> Option<String> {
        self.shared
            .last_seen_at
            .lock()
            .map(|g| g.clone())
            .unwrap_or(None)
    }

    /// `true` once the stream ended or was closed.
    pub fn closed(&self) -> bool {
        self.finished || self.shared.closed.load(Ordering::SeqCst)
    }
}

impl Stream for FinlightStream {
    type Item = Result<StreamEvent>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.finished {
            return Poll::Ready(None);
        }
        match this.rx.poll_recv(cx) {
            Poll::Ready(Some(Ok(item))) => Poll::Ready(Some(Ok(item))),
            Poll::Ready(Some(Err(err))) => {
                this.finished = true;
                Poll::Ready(Some(Err(err)))
            }
            Poll::Ready(None) => {
                this.finished = true;
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for FinlightStream {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(true);
        }
    }
}

impl fmt::Debug for FinlightStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FinlightStream")
            .field("closed", &self.closed())
            .field("last_seen_at", &self.last_seen_at())
            .finish()
    }
}

pub(crate) fn start(
    cfg: Arc<Config>,
    raw: bool,
    subscribe: Map<String, Value>,
    options: StreamOptions,
) -> FinlightStream {
    let (tx, rx) = mpsc::unbounded_channel();
    let (stop_tx, stop_rx) = watch::channel(false);
    let shared = Arc::new(Shared {
        last_seen_at: Mutex::new(None),
        closed: AtomicBool::new(false),
    });
    let driver = Driver {
        url: cfg.stream_url(raw),
        dedupe: Dedupe::new(cfg.dedupe_cache_size),
        cfg,
        raw,
        subscribe,
        options,
        tx,
        stop: stop_rx,
        shared: shared.clone(),
        connections: 0,
        failures: 0,
        quota_waits: 0,
        search_failed: 0,
        pending_quota_error: None,
        last_seen_nanos: None,
    };
    let task = tokio::spawn(driver.run());
    FinlightStream {
        rx,
        stop: Some(stop_tx),
        task: Some(task),
        shared,
        finished: false,
    }
}

#[derive(Debug, Clone, Copy)]
enum Delay {
    Now,
    Backoff,
    Long,
    Quota,
}

enum Plan {
    Terminal(Error),
    Reconnect(ReconnectReason, Delay),
}

enum Next {
    Stop,
    Plan(Plan),
}

struct Conn {
    sink: SplitSink<WsStream, Message>,
    stream: SplitStream<WsStream>,
    ping: Interval,
    watchdog: Instant,
    rotation: Instant,
}

impl Conn {
    fn new(ws: WsStream, cfg: &Config) -> Self {
        let (sink, stream) = ws.split();
        let now = Instant::now();
        let mut ping = tokio::time::interval_at(now + cfg.ping_interval, cfg.ping_interval);
        ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
        Conn {
            sink,
            stream,
            ping,
            watchdog: now + cfg.watchdog_timeout,
            rotation: now + cfg.rotation_interval,
        }
    }

    async fn send_text(&mut self, text: String) -> bool {
        self.sink.send(Message::text(text)).await.is_ok()
    }

    /// Closes with 1000 (graceful) and drops the socket.
    async fn close(mut self) {
        let frame = CloseFrame {
            code: CloseCode::Normal,
            reason: "".into(),
        };
        let _ = tokio::time::timeout(Duration::from_secs(5), async {
            let _ = self.sink.send(Message::Close(Some(frame))).await;
            let _ = self.sink.flush().await;
        })
        .await;
    }
}

struct Dedupe {
    cap: usize,
    order: VecDeque<String>,
    keys: HashSet<String>,
}

impl Dedupe {
    fn new(cap: usize) -> Self {
        Dedupe {
            cap,
            order: VecDeque::new(),
            keys: HashSet::new(),
        }
    }

    /// `true` when the key was seen before (and refreshes it).
    fn seen(&mut self, key: String) -> bool {
        if self.cap == 0 {
            return false;
        }
        if self.keys.contains(&key) {
            if let Some(pos) = self.order.iter().position(|k| *k == key) {
                self.order.remove(pos);
            }
            self.order.push_back(key);
            return true;
        }
        self.keys.insert(key.clone());
        self.order.push_back(key);
        while self.order.len() > self.cap {
            if let Some(old) = self.order.pop_front() {
                self.keys.remove(&old);
            }
        }
        false
    }
}

const NO_WEBSOCKETS_PREFIX: &str = "Your subscription does not include WebSockets";
const QUOTA_PREFIX: &str = "WebSocket quota exceeded";
const TOO_MANY_PREFIX: &str = "Too many concurrent connections";

fn refusal_message(body: Option<&str>) -> Option<String> {
    let value: Value = serde_json::from_str(body?).ok()?;
    let obj = value.as_object()?;
    obj.get("message")
        .or_else(|| obj.get("error"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn dial_error(
    status: u16,
    message: Option<&str>,
    body: Option<&str>,
    detail: Option<String>,
) -> ApiError {
    let detail = detail.or_else(|| message.map(str::to_owned));
    let mut text = format!("WebSocket connection refused with HTTP {status}");
    if let Some(d) = &detail {
        text.push_str(&format!(": {d}"));
    }
    ApiError::new(ApiErrorData {
        status,
        code: None,
        r#type: None,
        title: Some("WebSocket connection refused".to_owned()),
        detail,
        request_id: None,
        errors: Vec::new(),
        api_kind: ApiErrorKind::from_status(status),
        limit_type: None,
        retry_after: None,
        raw: body.map(str::to_owned),
        message: text,
    })
}

fn stream_error(
    code: Option<&ConnectionErrorCode>,
    message: Option<&str>,
    details: Option<&Value>,
) -> ApiError {
    let mut errors = Vec::new();
    if let Some(Value::Array(items)) = details {
        for item in items.iter().filter_map(Value::as_object) {
            let constraints: std::collections::BTreeMap<String, String> = item
                .get("constraints")
                .and_then(Value::as_object)
                .map(|c| {
                    c.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                        .collect()
                })
                .unwrap_or_default();
            let issue = if constraints.is_empty() {
                None
            } else {
                Some(constraints.values().cloned().collect::<Vec<_>>().join("; "))
            };
            errors.push(FieldIssue {
                field: item.get("field").and_then(Value::as_str).map(str::to_owned),
                issue,
                allowed_values: None,
                constraints: Some(constraints),
            });
        }
    }
    let problem = code.map(|c| ProblemCode::from(c.as_str()));
    let api_kind = problem
        .as_ref()
        .map(ApiErrorKind::from_code)
        .unwrap_or(ApiErrorKind::Other);
    let mut raw = Map::new();
    raw.insert(
        "code".into(),
        code.map(|c| Value::String(c.as_str().to_owned()))
            .unwrap_or(Value::Null),
    );
    raw.insert(
        "message".into(),
        message
            .map(|m| Value::String(m.to_owned()))
            .unwrap_or(Value::Null),
    );
    if let Some(d) = details {
        raw.insert("details".into(), d.clone());
    }
    let text = format!(
        "stream error {}: {}",
        code.map(|c| c.as_str()).unwrap_or("(no code)"),
        message.unwrap_or_default()
    );
    ApiError::new(ApiErrorData {
        status: 0,
        code: problem,
        r#type: None,
        title: Some("Stream error".to_owned()),
        detail: message.map(str::to_owned),
        request_id: None,
        errors,
        api_kind,
        limit_type: None,
        retry_after: None,
        raw: Some(Value::Object(raw).to_string()),
        message: text.trim_end().to_owned(),
    })
}

fn call_hook<T>(name: &str, hook: &Option<Hook<T>>, arg: &T) {
    if let Some(hook) = hook {
        if catch_unwind(AssertUnwindSafe(|| hook(arg))).is_err() {
            tracing::error!(target: "finlight", "the {name} hook panicked; the stream continues");
        }
    }
}

enum Event {
    Stop,
    Current(Option<std::result::Result<Message, tokio_tungstenite::tungstenite::Error>>),
    Old(Option<std::result::Result<Message, tokio_tungstenite::tungstenite::Error>>),
    Ping,
    Watchdog,
    Rotate,
}

struct Driver {
    cfg: Arc<Config>,
    url: String,
    raw: bool,
    subscribe: Map<String, Value>,
    options: StreamOptions,
    tx: mpsc::UnboundedSender<Result<StreamEvent>>,
    stop: watch::Receiver<bool>,
    shared: Arc<Shared>,
    connections: u32,
    failures: u32,
    quota_waits: u32,
    search_failed: u32,
    pending_quota_error: Option<Error>,
    dedupe: Dedupe,
    last_seen_nanos: Option<i128>,
}

impl Driver {
    async fn run(mut self) {
        let mut reason = ReconnectReason::Closed;
        let mut first = true;
        loop {
            if *self.stop.borrow() {
                break;
            }
            self.connections += 1;
            let outcome = tokio::select! {
                o = dial(&self.cfg, &self.url, self.options.takeover) => o,
                _ = self.stop.changed() => break,
            };
            let plan = match outcome {
                DialOutcome::Open(ws) => match self.run_connection(*ws, reason, first).await {
                    Next::Stop => break,
                    Next::Plan(plan) => plan,
                },
                DialOutcome::Refused { status, body } => self.classify_refusal(status, body),
                DialOutcome::Failed(err) => {
                    if matches!(err, Error::Validation(_)) {
                        Plan::Terminal(err)
                    } else {
                        self.report(err);
                        Plan::Reconnect(ReconnectReason::DialFailed, Delay::Backoff)
                    }
                }
            };
            first = false;
            match plan {
                Plan::Terminal(err) => {
                    tracing::warn!(target: "finlight", "the stream ended: {err}");
                    let _ = self.tx.send(Err(err));
                    break;
                }
                Plan::Reconnect(next_reason, delay) => {
                    let wait = self.delay(delay);
                    if wait > Duration::ZERO {
                        tracing::info!(target: "finlight", reason = %next_reason, "reconnecting the stream in {wait:?}");
                        tokio::select! {
                            _ = tokio::time::sleep(wait) => {}
                            _ = self.stop.changed() => break,
                        }
                    }
                    reason = next_reason;
                }
            }
        }
        self.shared.closed.store(true, Ordering::SeqCst);
    }

    fn delay(&mut self, kind: Delay) -> Duration {
        let cfg = &self.cfg;
        match kind {
            Delay::Now => Duration::ZERO,
            Delay::Backoff => {
                let d = backoff(
                    cfg.reconnect_base_delay,
                    cfg.reconnect_max_delay,
                    self.failures,
                );
                self.failures = self.failures.saturating_add(1);
                d
            }
            Delay::Long => cfg.long_backoff + cfg.long_backoff.mul_f64(fastrand::f64() * 0.2),
            Delay::Quota => {
                let wait = cfg.quota_reconnect_interval;
                self.quota_waits = self.quota_waits.saturating_add(1);
                let error = self
                    .pending_quota_error
                    .take()
                    .unwrap_or_else(|| Error::Api(dial_error(403, Some(QUOTA_PREFIX), None, None)));
                tracing::warn!(
                    target: "finlight",
                    attempt = self.quota_waits,
                    "the WebSocket quota is used up ({error}); the stream tries again in {wait:?}"
                );
                let info = QuotaWaitInfo {
                    error,
                    next_attempt_in: wait,
                    attempt: self.quota_waits,
                };
                call_hook("on_quota_wait", &self.options.on_quota_wait, &info);
                wait
            }
        }
    }

    fn report(&self, err: Error) {
        tracing::warn!(target: "finlight", "stream problem: {err}");
        call_hook("on_error", &self.options.on_error, &err);
    }

    fn quota_plan(&mut self, error: Error) -> Plan {
        if !self.cfg.quota_reconnect {
            return Plan::Terminal(error);
        }
        self.pending_quota_error = Some(error);
        Plan::Reconnect(ReconnectReason::QuotaExceeded, Delay::Quota)
    }

    fn classify_refusal(&mut self, status: u16, body: Option<String>) -> Plan {
        let message = refusal_message(body.as_deref());
        let text = message.clone().unwrap_or_default();
        let err = |detail: Option<String>| {
            dial_error(status, message.as_deref(), body.as_deref(), detail)
        };
        match status {
            401 => Plan::Terminal(Error::Api(err(None))),
            403 if text.starts_with(NO_WEBSOCKETS_PREFIX) => Plan::Terminal(Error::Api(err(None))),
            403 if text.starts_with(QUOTA_PREFIX) => self.quota_plan(Error::Api(err(None))),
            403 if text.starts_with(TOO_MANY_PREFIX) => {
                self.report(Error::Api(err(Some(format!(
                    "{text} The connection limit is shared by enriched, raw and older SDK streams; \
                     the SDK retries after a long wait. Set StreamOptions::takeover(true) to replace \
                     your stalest connection."
                )))));
                Plan::Reconnect(ReconnectReason::ConnectionLimit, Delay::Long)
            }
            403 => {
                self.report(Error::Api(err(None)));
                Plan::Reconnect(ReconnectReason::DialRefused, Delay::Long)
            }
            429 => {
                self.report(Error::Api(err(None)));
                Plan::Reconnect(ReconnectReason::RateLimited, Delay::Long)
            }
            500..=599 => Plan::Reconnect(ReconnectReason::DialRefused, Delay::Backoff),
            _ => {
                self.report(Error::Api(err(None)));
                Plan::Reconnect(ReconnectReason::DialRefused, Delay::Backoff)
            }
        }
    }

    fn subscribe_text(&self) -> String {
        let mut frame = self.subscribe.clone();
        frame.insert(
            "clientNonce".into(),
            Value::String(format!("c-{}", uuid::Uuid::new_v4())),
        );
        Value::Object(frame).to_string()
    }

    fn set_last_seen(&mut self, value: &str) {
        let Some(nanos) = parse_rfc3339_nanos(value) else {
            return;
        };
        if self.last_seen_nanos.is_none_or(|seen| nanos >= seen) {
            self.last_seen_nanos = Some(nanos);
            if let Ok(mut guard) = self.shared.last_seen_at.lock() {
                *guard = Some(value.to_owned());
            }
        }
    }

    fn last_seen(&self) -> Option<String> {
        self.shared
            .last_seen_at
            .lock()
            .map(|g| g.clone())
            .unwrap_or(None)
    }

    async fn run_connection(&mut self, ws: WsStream, reason: ReconnectReason, first: bool) -> Next {
        let mut cur = Conn::new(ws, &self.cfg);
        let mut old: Option<Conn> = None;
        if !cur.send_text(self.subscribe_text()).await {
            return Next::Plan(Plan::Reconnect(ReconnectReason::Closed, Delay::Backoff));
        }
        if !first {
            let info = ReconnectInfo {
                last_seen_at: self.last_seen(),
                reason,
                connection: self.connections,
            };
            call_hook("on_reconnect", &self.options.on_reconnect, &info);
        }
        loop {
            let rotating = old.is_some();
            let event = {
                let old_next = async {
                    match old.as_mut() {
                        Some(c) => c.stream.next().await,
                        None => std::future::pending().await,
                    }
                };
                tokio::select! {
                    biased;
                    _ = self.stop.changed() => Event::Stop,
                    m = cur.stream.next() => Event::Current(m),
                    m = old_next => Event::Old(m),
                    _ = cur.ping.tick() => Event::Ping,
                    _ = tokio::time::sleep_until(cur.watchdog) => Event::Watchdog,
                    _ = tokio::time::sleep_until(cur.rotation), if !rotating => Event::Rotate,
                }
            };
            match event {
                Event::Stop => {
                    cur.close().await;
                    if let Some(o) = old.take() {
                        o.close().await;
                    }
                    return Next::Stop;
                }
                Event::Ping => {
                    let ping = format!(
                        "{{\"action\":\"ping\",\"t\":{}}}",
                        epoch_millis(SystemTime::now())
                    );
                    if !cur.send_text(ping).await {
                        tracing::debug!(target: "finlight", "could not send a ping");
                    }
                }
                Event::Watchdog => {
                    tracing::warn!(
                        target: "finlight",
                        "no pong for {:?}; replacing the stream connection",
                        self.cfg.watchdog_timeout
                    );
                    drop(cur);
                    if let Some(o) = old.take() {
                        o.close().await;
                    }
                    return Next::Plan(Plan::Reconnect(ReconnectReason::Watchdog, Delay::Now));
                }
                Event::Rotate => {
                    tracing::info!(target: "finlight", "renewing the stream connection before the 2 h gateway limit");
                    self.connections += 1;
                    let outcome = tokio::select! {
                        o = dial(&self.cfg, &self.url, self.options.takeover) => o,
                        _ = self.stop.changed() => {
                            cur.close().await;
                            return Next::Stop;
                        }
                    };
                    match outcome {
                        DialOutcome::Open(ws) => {
                            let mut fresh = Conn::new(*ws, &self.cfg);
                            if !fresh.send_text(self.subscribe_text()).await {
                                cur.close().await;
                                return Next::Plan(Plan::Reconnect(
                                    ReconnectReason::Rotation,
                                    Delay::Now,
                                ));
                            }
                            let info = ReconnectInfo {
                                last_seen_at: self.last_seen(),
                                reason: ReconnectReason::Rotation,
                                connection: self.connections,
                            };
                            call_hook("on_reconnect", &self.options.on_reconnect, &info);
                            old = Some(std::mem::replace(&mut cur, fresh));
                        }
                        DialOutcome::Refused { status, body } => {
                            cur.close().await;
                            return Next::Plan(match self.classify_refusal(status, body) {
                                Plan::Reconnect(_, delay) => {
                                    Plan::Reconnect(ReconnectReason::Rotation, delay)
                                }
                                terminal => terminal,
                            });
                        }
                        DialOutcome::Failed(err) => {
                            self.report(err);
                            cur.close().await;
                            return Next::Plan(Plan::Reconnect(
                                ReconnectReason::Rotation,
                                Delay::Backoff,
                            ));
                        }
                    }
                }
                Event::Old(msg) => match msg {
                    Some(Ok(Message::Text(text))) => {
                        // The old connection of a rotation still delivers until the new one is admitted.
                        if let Some(frame) = frames::decode(text.as_str()) {
                            if let Frame::Article { .. } | Frame::Other { .. } = frame {
                                self.deliver(frame, text.as_str());
                            }
                        }
                    }
                    Some(Ok(_)) => {}
                    _ => old = None,
                },
                Event::Current(msg) => {
                    let close_code = match msg {
                        Some(Ok(Message::Text(text))) => {
                            match self.handle_frame(text.as_str(), &mut cur, &mut old).await {
                                None => continue,
                                Some(plan) => {
                                    cur.close().await;
                                    if let Some(o) = old.take() {
                                        o.close().await;
                                    }
                                    return Next::Plan(plan);
                                }
                            }
                        }
                        Some(Ok(Message::Close(frame))) => frame.map(|f| u16::from(f.code)),
                        Some(Ok(_)) => continue,
                        Some(Err(e)) => {
                            tracing::debug!(target: "finlight", "stream connection error: {e}");
                            None
                        }
                        None => None,
                    };
                    if let Some(o) = old.take() {
                        o.close().await;
                    }
                    if close_code == Some(1008) {
                        return Next::Plan(Plan::Terminal(Error::Api(ApiError::new(
                            ApiErrorData {
                                status: 1008,
                                code: None,
                                r#type: None,
                                title: Some("Policy violation".to_owned()),
                                detail: Some(
                                    "the server closed the stream with code 1008".to_owned(),
                                ),
                                request_id: None,
                                errors: Vec::new(),
                                api_kind: ApiErrorKind::Other,
                                limit_type: None,
                                retry_after: None,
                                raw: None,
                                message:
                                    "the server closed the stream with code 1008 (policy violation)"
                                        .to_owned(),
                            },
                        ))));
                    }
                    tracing::info!(target: "finlight", code = ?close_code, "the stream connection closed");
                    return Next::Plan(Plan::Reconnect(ReconnectReason::Closed, Delay::Backoff));
                }
            }
        }
    }

    /// Handles a frame of the current connection. `Some(plan)` ends the connection.
    async fn handle_frame(
        &mut self,
        text: &str,
        cur: &mut Conn,
        old: &mut Option<Conn>,
    ) -> Option<Plan> {
        let Some(frame) = frames::decode(text) else {
            tracing::warn!(target: "finlight", "ignored a stream frame that is not a JSON object");
            return None;
        };
        match frame {
            Frame::Pong => {
                cur.watchdog = Instant::now() + self.cfg.watchdog_timeout;
                None
            }
            Frame::Admitted {
                lease_id,
                server_now,
                client_nonce,
            } => {
                self.failures = 0;
                self.quota_waits = 0;
                if let Some(o) = old.take() {
                    o.close().await;
                }
                let info = AdmittedInfo {
                    lease_id,
                    server_now,
                    client_nonce,
                };
                call_hook("on_admitted", &self.options.on_admitted, &info);
                None
            }
            Frame::Preempted {
                reason,
                new_lease_id,
                timestamp,
            } => {
                let quota = reason == PreemptReason::QuotaExceeded;
                let err = Error::Preempted(PreemptedError::new(reason, new_lease_id, timestamp));
                Some(if quota {
                    self.quota_plan(err)
                } else {
                    Plan::Terminal(err)
                })
            }
            Frame::Error {
                code,
                message,
                details,
            } => {
                let err = stream_error(code.as_ref(), message.as_deref(), details.as_ref());
                match code {
                    Some(ConnectionErrorCode::ValidationFailed) => {
                        Some(Plan::Terminal(Error::Api(err)))
                    }
                    Some(ConnectionErrorCode::HandshakeFailed) => {
                        tracing::warn!(target: "finlight", "the stream handshake failed; reconnecting");
                        Some(Plan::Reconnect(
                            ReconnectReason::HandshakeFailed,
                            Delay::Backoff,
                        ))
                    }
                    Some(ConnectionErrorCode::SearchFailed) => {
                        self.search_failed += 1;
                        if self.search_failed >= self.cfg.search_failed_threshold.max(1) {
                            self.search_failed = 0;
                            self.report(Error::Api(err));
                        } else {
                            tracing::warn!(target: "finlight", "the initial stream search failed; reconnecting");
                        }
                        Some(Plan::Reconnect(
                            ReconnectReason::SearchFailed,
                            Delay::Backoff,
                        ))
                    }
                    _ => {
                        // PING_ON_DEFAULT_ROUTE, INTERNAL_ERROR, unknown codes and v2-form errors.
                        self.report(Error::Api(err));
                        None
                    }
                }
            }
            frame @ (Frame::Article { .. } | Frame::Other { .. }) => {
                self.deliver(frame, text);
                None
            }
            Frame::Ignored(why) => {
                tracing::debug!(target: "finlight", "ignored a stream frame: {why}");
                None
            }
        }
    }

    fn deliver(&mut self, frame: Frame, text: &str) {
        match frame {
            Frame::Article {
                event,
                updated,
                data,
            } => {
                self.search_failed = 0;
                let article = match decode_article(&data, "data") {
                    Ok(a) => a,
                    Err(err) => {
                        self.report(Error::decode(
                            format!("skipped a stream article that could not be decoded: {err}"),
                            None,
                            Some(text.to_owned()),
                            None,
                        ));
                        return;
                    }
                };
                let digest = hex::encode(Sha256::digest(data.get().as_bytes()));
                let key = format!("{}\n{}\n{digest}", event.as_str(), article.id);
                if self.dedupe.seen(key) {
                    tracing::debug!(target: "finlight", id = %article.id, "dropped a duplicate {event}");
                    return;
                }
                let seen = if self.raw {
                    article.first_seen_at.clone()
                } else {
                    article
                        .available_at
                        .clone()
                        .or_else(|| article.first_seen_at.clone())
                };
                if let Some(seen) = seen {
                    self.set_last_seen(&seen);
                }
                let kind = if updated {
                    StreamEventKind::Updated
                } else {
                    StreamEventKind::Created
                };
                let _ = self.tx.send(Ok(StreamEvent::new(
                    kind,
                    event,
                    Some(article),
                    Arc::from(text),
                )));
            }
            Frame::Other { event } => {
                tracing::debug!(target: "finlight", %event, "unknown stream event");
                let _ = self.tx.send(Ok(StreamEvent::new(
                    StreamEventKind::Other,
                    event,
                    None,
                    Arc::from(text),
                )));
            }
            _ => {}
        }
    }
}
