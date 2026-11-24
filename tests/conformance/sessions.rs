//! S-01 .. S-28, R-13, R-14: WebSocket sessions played against the in-process server, on the
//! paused tokio clock.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use finlight::{
    AdmittedInfo, Client, Config, Error, RawStreamParams, ReconnectInfo, StreamEvent,
    StreamEventKind, StreamOptions, StreamParams,
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::common::ws_mock::{DialScript, WsMock};
use crate::common::{
    KIT_API_KEY, advance, load, run_paused, settle, spawn_spinner, wait_for, without,
};

/// What a session test can see of an error.
#[derive(Debug, Clone, Default)]
pub struct ErrInfo {
    pub kind: &'static str,
    pub api_kind: Option<String>,
    pub code: Option<String>,
    pub status: Option<u16>,
    pub message: String,
    pub detail: Option<String>,
    pub reason: Option<String>,
    pub new_lease_id: Option<String>,
    pub fields: Vec<(Option<String>, bool)>,
}

pub fn err_info(e: &Error) -> ErrInfo {
    let mut info = ErrInfo {
        kind: e.kind(),
        message: e.to_string(),
        ..Default::default()
    };
    match e {
        Error::Api(a) => {
            info.api_kind = Some(a.api_kind().as_str().to_owned());
            info.code = a.code().map(|c| c.as_str().to_owned());
            info.status = Some(a.status());
            info.detail = a.detail().map(str::to_owned);
            info.fields = a
                .errors()
                .iter()
                .map(|i| {
                    (
                        i.field.clone(),
                        i.constraints.as_ref().is_some_and(|c| !c.is_empty()),
                    )
                })
                .collect();
        }
        Error::Preempted(p) => {
            info.reason = Some(p.reason().as_str().to_owned());
            info.new_lease_id = p.new_lease_id().map(str::to_owned);
        }
        _ => {}
    }
    info
}

#[derive(Default)]
pub struct Observed {
    pub items: Vec<StreamEvent>,
    pub admitted: Vec<AdmittedInfo>,
    pub reconnects: Vec<ReconnectInfo>,
    pub errors: Vec<ErrInfo>,
    pub quota: Vec<(ErrInfo, Duration, u32)>,
    pub terminal: Option<ErrInfo>,
    pub done: bool,
}

pub struct Probe {
    pub obs: Arc<Mutex<Observed>>,
    stop: Option<oneshot::Sender<()>>,
    pump: JoinHandle<Option<String>>,
}

impl Probe {
    pub fn obs(&self) -> MutexGuard<'_, Observed> {
        self.obs.lock().unwrap()
    }

    pub async fn start(
        ws: &WsMock,
        raw: bool,
        params: &Value,
        api_key: &str,
        quota_reconnect: bool,
    ) -> Probe {
        let config = Config::builder(api_key)
            .wss_url(ws.url.clone())
            .base_url("http://127.0.0.1:9")
            .quota_reconnect(quota_reconnect)
            .build()
            .unwrap();
        let client = Client::new(config).unwrap();
        let obs = Arc::new(Mutex::new(Observed::default()));
        let (o1, o2, o3, o4) = (obs.clone(), obs.clone(), obs.clone(), obs.clone());
        let options = StreamOptions::new()
            .on_admitted(move |a| o1.lock().unwrap().admitted.push(a.clone()))
            .on_reconnect(move |r| o2.lock().unwrap().reconnects.push(r.clone()))
            .on_error(move |e| o3.lock().unwrap().errors.push(err_info(e)))
            .on_quota_wait(move |q| {
                o4.lock()
                    .unwrap()
                    .quota
                    .push((err_info(&q.error), q.next_attempt_in, q.attempt))
            });
        let text = params.to_string();
        let mut stream = if raw {
            client
                .raw_stream(RawStreamParams::from_json(&text).unwrap(), options)
                .await
                .unwrap()
        } else {
            client
                .stream(StreamParams::from_json(&text).unwrap(), options)
                .await
                .unwrap()
        };
        let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
        let o = obs.clone();
        let pump = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    item = stream.next() => match item {
                        Some(Ok(e)) => o.lock().unwrap().items.push(e),
                        Some(Err(e)) => o.lock().unwrap().terminal = Some(err_info(&e)),
                        None => {
                            o.lock().unwrap().done = true;
                            let _ = (&mut stop_rx).await;
                            break;
                        }
                    }
                }
            }
            let last = stream.last_seen_at();
            stream.close().await;
            assert!(stream.closed());
            last
        });
        Probe {
            obs,
            stop: Some(stop_tx),
            pump,
        }
    }

    /// Stops the stream; returns `last_seen_at`.
    pub async fn stop(mut self) -> Option<String> {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
        let handle = self.pump;
        wait_for("stream task to stop", || handle.is_finished()).await;
        handle.await.expect("stream task")
    }
}

fn is_subscribe(frame: &Value) -> bool {
    frame.as_object().is_some_and(|o| {
        o.contains_key("payloadVersion") && !o.contains_key("type") && !o.contains_key("action")
    })
}

#[derive(Default, Clone, Copy)]
struct Counters {
    admitted: usize,
    items: usize,
    errors: usize,
    reconnects: usize,
    quota: usize,
}

#[derive(Default)]
pub struct Opts {
    pub forbidden: Vec<&'static str>,
    pub auto_pong: Option<bool>,
}

pub struct Session {
    pub ws: WsMock,
    pub probe: Probe,
    pub subscribe: Value,
    pub seen_dials: usize,
    spinner: JoinHandle<()>,
}

impl Session {
    /// Stops the stream and checks that no socket stays open.
    pub async fn finish(self) -> Option<String> {
        let last = self.probe.stop().await;
        let ws = self.ws.clone();
        settle().await;
        wait_for("all sockets closed", || ws.open_connections() == 0).await;
        self.spinner.abort();
        last
    }

    /// Advances until the next dial arrives (at most `max_ms`); returns the virtual time it took.
    pub async fn advance_until_dial(&mut self, max_ms: u64) -> Duration {
        let start = tokio::time::Instant::now();
        let target = self.seen_dials;
        let mut spent = 0;
        while self.ws.dial_count() <= target {
            assert!(spent < max_ms, "no redial within {max_ms} ms");
            advance(1000).await;
            spent += 1000;
        }
        start.elapsed()
    }

    /// Consumes the next dial and its subscribe frame.
    pub async fn take_redial(&mut self) {
        let dial = self.ws.wait_dial(self.seen_dials).await;
        self.seen_dials += 1;
        let conn = dial.conn.expect("redial accepted");
        let msg = conn.next("re-subscribe").await;
        assert_eq!(
            without(&msg, &["clientNonce"]),
            without(&self.subscribe, &["clientNonce"])
        );
    }
}

pub async fn play(fixture: &Value, opts: Opts) -> Session {
    let steps = fixture["steps"].as_array().unwrap().clone();
    let raw = fixture["endpoint"] == "raw";
    let ws = WsMock::start().await;
    ws.auto_pong.store(
        opts.auto_pong.unwrap_or(true),
        std::sync::atomic::Ordering::SeqCst,
    );
    let spinner = spawn_spinner();

    let first_subscribe = steps
        .iter()
        .find(|s| s["step"] == "clientSends" && is_subscribe(&s["frame"]))
        .map(|s| s["frame"].clone())
        .unwrap_or_else(|| json!({"payloadVersion": 3, "query": "ticker:XMPL"}));
    let subscribe = without(&first_subscribe, &opts.forbidden);
    let params = without(&subscribe, &["payloadVersion", "clientNonce"]);
    for step in &steps {
        if step["step"] == "dial" {
            let status = step["server"]["status"].as_u64().unwrap() as u16;
            ws.script(if status == 101 {
                DialScript::Accept
            } else {
                DialScript::Refuse {
                    status,
                    body: step["server"]["body"].as_str().map(str::to_owned),
                }
            });
        }
    }
    let api_key = steps
        .iter()
        .find(|s| s["step"] == "dial")
        .and_then(|s| s["expectRequestHeaders"]["x-api-key"].as_str())
        .unwrap_or(KIT_API_KEY)
        .to_owned();
    let quota_reconnect = fixture["clientConfig"]["quotaReconnect"]
        .as_bool()
        .unwrap_or(true);
    let probe = Probe::start(&ws, raw, &params, &api_key, quota_reconnect).await;
    let mut session = Session {
        ws: ws.clone(),
        probe,
        subscribe,
        seen_dials: 0,
        spinner,
    };

    let mut conn = None;
    let mut subscribed_on: Option<usize> = None;
    let mut c = Counters::default();
    for (i, step) in steps.iter().enumerate() {
        match step["step"].as_str().unwrap() {
            "dial" => {
                let dial = ws.wait_dial(session.seen_dials).await;
                session.seen_dials += 1;
                assert_eq!(dial.path, if raw { "/raw" } else { "/" }, "dial path");
                assert!(!dial.request_line.to_ascii_lowercase().contains("apikey="));
                if let Some(key) = step["expectRequestHeaders"]["x-api-key"].as_str() {
                    assert_eq!(dial.header("x-api-key"), Some(key));
                }
                conn = dial.conn.clone();
            }
            "clientSends" => {
                let frame = &step["frame"];
                let current = conn.as_ref().expect("an open connection");
                if let Some(pattern) = step.get("match") {
                    let msg = current.next("client frame").await;
                    for (k, v) in pattern.as_object().unwrap() {
                        assert_eq!(&msg[k], v);
                    }
                    if pattern["action"] == "ping" {
                        assert!(msg["t"].is_number());
                    }
                    continue;
                }
                // Frames no SDK sends (a broken frame, a ping before the subscribe, a ping on
                // $default, a re-subscribe on the same connection) are not produced; the server
                // side of the session is still replayed.
                if !is_subscribe(frame) || subscribed_on == Some(session.seen_dials) {
                    continue;
                }
                let msg = current.next("subscribe frame").await;
                assert!(msg["clientNonce"].is_string(), "clientNonce is a string");
                assert_eq!(
                    without(&msg, &["clientNonce"]),
                    without(&session.subscribe, &["clientNonce"]),
                    "subscribe frame"
                );
                assert!(msg["payloadVersion"].is_u64() && msg["payloadVersion"] == 3);
                for key in ["includeContent", "includeEntities", "extended"] {
                    assert!(msg.get(key).is_none());
                }
                subscribed_on = Some(session.seen_dials);
            }
            "serverSends" => {
                conn.as_ref()
                    .expect("an open connection")
                    .send(&step["frame"]);
                settle().await;
            }
            "serverCloses" => {
                if let Some(cn) = &conn {
                    cn.close(1000);
                }
                settle().await;
            }
            "advance" => advance(step["ms"].as_u64().unwrap()).await,
            "expect" => c = check_expect(step, steps.get(i + 1), &mut session, c).await,
            other => panic!("unknown step {other}"),
        }
    }
    session
}

async fn check_expect(
    step: &Value,
    next: Option<&Value>,
    s: &mut Session,
    mut c: Counters,
) -> Counters {
    let outcome = step["outcome"].as_str().unwrap();
    match outcome {
        "admitted" => {
            wait_for("admitted", || s.probe.obs().admitted.len() > c.admitted).await;
            let a = s.probe.obs().admitted[c.admitted].clone();
            c.admitted += 1;
            if let Some(l) = step["leaseId"].as_str() {
                assert_eq!(a.lease_id.as_deref(), Some(l));
            }
            if let Some(n) = step["clientNonce"].as_str() {
                assert_eq!(a.client_nonce.as_deref(), Some(n));
            }
        }
        "noAdmitted" => {
            settle().await;
            let o = s.probe.obs();
            assert_eq!(o.admitted.len(), c.admitted);
            assert!(o.terminal.is_none());
        }
        "item" => {
            wait_for("item", || {
                let o = s.probe.obs();
                o.items
                    .get(c.items..)
                    .is_some_and(|rest| rest.iter().any(|i| i.kind != StreamEventKind::Other))
            })
            .await;
            let o = s.probe.obs();
            while o.items[c.items].kind == StreamEventKind::Other {
                c.items += 1;
            }
            let item = &o.items[c.items];
            c.items += 1;
            assert_eq!(item.kind.as_str(), step["kind"].as_str().unwrap());
            if let Some(id) = step["id"].as_str() {
                assert_eq!(item.data.as_ref().unwrap().id, id);
            }
        }
        "duplicateDropped" => {
            settle().await;
            let o = s.probe.obs();
            assert_eq!(o.items.len(), c.items, "the duplicate is not delivered");
            assert!(o.terminal.is_none());
        }
        "liveness" => {
            settle().await;
            let o = s.probe.obs();
            assert!(o.terminal.is_none() && !o.done);
            assert_eq!(s.ws.dial_count(), s.seen_dials, "no reconnect");
        }
        "logOnly" => {
            wait_for("a reported error or an other item", || {
                let o = s.probe.obs();
                o.errors.len() > c.errors
                    || o.items[c.items..]
                        .iter()
                        .any(|i| i.kind == StreamEventKind::Other)
            })
            .await;
            settle().await;
            let o = s.probe.obs();
            if let Some(code) = step["code"].as_str() {
                assert!(
                    o.errors[c.errors..]
                        .iter()
                        .any(|e| e.code.as_deref() == Some(code)),
                    "reported {code}"
                );
            }
            c.errors = o.errors.len();
            while c.items < o.items.len() && o.items[c.items].kind == StreamEventKind::Other {
                c.items += 1;
            }
            assert!(o.terminal.is_none() && !o.done);
            assert_eq!(s.ws.dial_count(), s.seen_dials, "no reconnect");
            let conn = s.ws.dial(s.seen_dials - 1).conn.unwrap();
            assert!(conn.open(), "the connection stays open");
        }
        "terminalError" => {
            wait_for("terminal error", || s.probe.obs().done).await;
            let o = s.probe.obs();
            let err = o.terminal.clone().expect("a terminal error");
            match step["error"].as_str().unwrap() {
                "PreemptedError" => {
                    assert_eq!(err.kind, "preempted");
                    assert_eq!(err.reason.as_deref(), step["reason"].as_str());
                    assert_eq!(err.new_lease_id.as_deref(), step["newLeaseId"].as_str());
                }
                "FinlightApiError" => {
                    assert_eq!(err.kind, "api");
                    assert_eq!(err.api_kind.as_deref(), step["apiKind"].as_str());
                    if let Some(status) = step["status"].as_u64() {
                        assert_eq!(err.status, Some(status as u16));
                    }
                    if let Some(code) = step["code"].as_str() {
                        assert_eq!(err.code.as_deref(), Some(code));
                        assert!(!err.fields.is_empty(), "errors[] carries the details");
                        assert!(
                            err.fields
                                .iter()
                                .all(|(f, constraints)| f.is_some() && *constraints)
                        );
                    }
                }
                other => panic!("unknown terminal error {other}"),
            }
            assert_eq!(s.ws.dial_count(), s.seen_dials, "no reconnect");
        }
        "noReconnect" => {
            settle().await;
            advance(0).await;
            assert_eq!(s.ws.dial_count(), s.seen_dials, "no reconnect");
        }
        "reported" => {
            wait_for("a reported quota wait", || {
                s.probe.obs().quota.len() > c.quota
            })
            .await;
            let (err, next_in, attempt) = s.probe.obs().quota[c.quota].clone();
            c.quota += 1;
            assert_eq!(step["hook"], "onQuotaWait");
            assert_eq!(attempt as u64, step["attempt"].as_u64().unwrap());
            assert_eq!(next_in, Duration::from_secs(3600));
            if step["error"] == "PreemptedError" {
                assert_eq!(err.kind, "preempted");
                assert_eq!(err.reason.as_deref(), step["reason"].as_str());
                assert_eq!(err.new_lease_id.as_deref(), step["newLeaseId"].as_str());
            }
            let o = s.probe.obs();
            assert!(o.terminal.is_none() && !o.done, "the stream keeps running");
            // A quota wait goes to on_quota_wait only.
            assert_eq!(o.errors.len(), c.errors);
        }
        "reconnect" => {
            let backoff = &step["backoff"];
            if s.ws.dial_count() <= s.seen_dials {
                assert!(!s.probe.obs().done, "the stream keeps running");
                let min_ms = step["firstRetryMinMs"].as_u64().or(if backoff == "long" {
                    Some(60_000)
                } else {
                    None
                });
                let start = tokio::time::Instant::now();
                if let Some(min) = min_ms {
                    advance(min - 1000).await;
                    assert_eq!(s.ws.dial_count(), s.seen_dials, "no redial before {min} ms");
                }
                s.advance_until_dial(3 * 3600 * 1000).await;
                let waited = start.elapsed();
                if backoff == true || backoff == "long" || backoff == "quota" {
                    assert!(waited > Duration::ZERO, "the redial waits");
                }
                if let Some(min) = min_ms {
                    assert!(waited >= Duration::from_millis(min));
                }
            }
            if step["hint"] == "takeover" {
                assert!(
                    s.probe
                        .obs()
                        .errors
                        .iter()
                        .any(|e| e.message.contains("takeover"))
                );
            }
            if let Some(reported) = step.get("reported") {
                let (err, next_in, attempt) = s.probe.obs().quota[c.quota].clone();
                c.quota += 1;
                assert_eq!(reported["hook"], "onQuotaWait");
                assert_eq!(err.kind, "api");
                assert_eq!(err.api_kind.as_deref(), reported["apiKind"].as_str());
                assert_eq!(
                    err.status,
                    Some(reported["status"].as_u64().unwrap() as u16)
                );
                assert_eq!(attempt as u64, reported["attempt"].as_u64().unwrap());
                assert_eq!(next_in, Duration::from_secs(3600));
            }
            let next_is_dial = next.is_some_and(|n| n["step"] == "dial");
            if !next_is_dial {
                let dial = s.ws.wait_dial(s.seen_dials).await;
                s.seen_dials += 1;
                if step["resubscribe"] == true {
                    let conn = dial.conn.clone().expect("redial accepted");
                    let msg = conn.next("re-subscribe").await;
                    assert_eq!(
                        without(&msg, &["clientNonce"]),
                        without(&s.subscribe, &["clientNonce"])
                    );
                }
            }
            if let Some(reason) = step["reason"].as_str() {
                wait_for("on_reconnect", || {
                    s.probe.obs().reconnects.len() > c.reconnects
                })
                .await;
                let r = s.probe.obs().reconnects[c.reconnects].clone();
                c.reconnects += 1;
                assert_eq!(r.reason.as_str(), reason);
            }
            assert!(!s.probe.obs().done, "not terminal");
        }
        other => panic!("unknown outcome {other}"),
    }
    c
}

fn session(file: &str) -> Value {
    load(&format!("ws-sessions/{file}"))
}

fn simple(file: &'static str) -> impl FnOnce() {
    move || {
        run_paused(async move {
            let s = play(&session(file), Opts::default()).await;
            s.finish().await;
        })
    }
}

fn r13() {
    subscribe_frame("fixtures/requests/ws-subscribe-enriched.json", false)
}

fn r14() {
    subscribe_frame("fixtures/requests/ws-subscribe-raw.json", true)
}

fn subscribe_frame(file: &str, raw: bool) {
    let f = load(file);
    run_paused(async move {
        let ws = WsMock::start().await;
        let spinner = spawn_spinner();
        let probe = Probe::start(&ws, raw, &f["sdk"]["args"], KIT_API_KEY, true).await;
        let dial = ws.wait_dial(0).await;
        assert_eq!(dial.path, if raw { "/raw" } else { "/" });
        let conn = dial.conn.unwrap();
        let frame = conn.next("subscribe").await;
        assert_eq!(
            without(&frame, &["clientNonce"]),
            without(&f["frame"], &["clientNonce"])
        );
        assert!(frame["clientNonce"].is_string());
        assert!(frame["payloadVersion"].is_u64() && frame["payloadVersion"] == 3);
        assert!(frame.get("action").is_none());
        if raw {
            assert!(frame.get("include").is_none());
        }
        for k in ["includeContent", "includeEntities", "extended"] {
            assert!(frame.get(k).is_none());
        }
        for server in f["server"]["frames"].as_array().unwrap() {
            conn.send(server);
        }
        // Wait for everything the server sent: the admitted hook fires before the article
        // reaches the stream.
        let want_items = if raw { 0 } else { 1 };
        wait_for("admitted and the article", || {
            let o = probe.obs();
            o.admitted.len() == 1 && o.items.len() >= want_items
        })
        .await;
        settle().await;
        {
            let o = probe.obs();
            assert_eq!(
                o.admitted[0].client_nonce.as_deref(),
                f["frame"]["clientNonce"].as_str()
            );
            assert_eq!(o.items.len(), want_items);
            if !raw {
                assert_eq!(o.items[0].kind, StreamEventKind::Created);
            }
            assert!(o.errors.is_empty());
        }
        probe.stop().await;
        spinner.abort();
    });
}

fn s01() {
    run_paused(async {
        let s = play(&session("v2-pong-before-subscribe.json"), Opts::default()).await;
        s.finish().await;
        // The v2 pong refreshes the watchdog: with auto-pong off, a v2 pong at 50 s keeps the
        // connection past 60 s.
        let fixture = json!({"endpoint": "enriched", "steps": [
            {"step": "dial", "server": {"status": 101}},
            {"step": "advance", "ms": 50000},
        ]});
        let mut s = play(
            &fixture,
            Opts {
                auto_pong: Some(false),
                ..Default::default()
            },
        )
        .await;
        let conn = s.ws.dial(0).conn.unwrap();
        conn.send(&json!({"action": "pong", "t": 1, "serverNow": 1}));
        settle().await;
        advance(59_000).await;
        assert_eq!(s.ws.dial_count(), 1, "the v2 pong refreshed the watchdog");
        advance(2_000).await;
        assert_eq!(
            s.ws.dial_count(),
            2,
            "the watchdog fires 60 s after the last pong"
        );
        s.seen_dials = 2;
        s.finish().await;
    })
}

fn s02() {
    run_paused(async {
        let spinner = spawn_spinner();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let head = Arc::new(Mutex::new(Vec::<u8>::new()));
        let h = head.clone();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            loop {
                let n = socket.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                let mut g = h.lock().unwrap();
                g.extend_from_slice(&buf[..n]);
                if g.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
        });
        let config = Config::builder(KIT_API_KEY)
            .wss_url(format!("ws://{addr}/"))
            .build()
            .unwrap();
        let client = Client::new(config).unwrap();
        let mut stream = client
            .stream(
                StreamParams::new().query("ticker:XMPL"),
                StreamOptions::new().takeover(true),
            )
            .await
            .unwrap();
        wait_for("dial request bytes", || {
            head.lock().unwrap().windows(4).any(|w| w == b"\r\n\r\n")
        })
        .await;
        let text = String::from_utf8(head.lock().unwrap().clone()).unwrap();
        let mut lines = text.split("\r\n");
        let request_line = lines.next().unwrap();
        assert_eq!(request_line, "GET / HTTP/1.1");
        assert!(!request_line.to_ascii_lowercase().contains("apikey"));
        let lines: Vec<&str> = lines.collect();
        assert!(
            lines.contains(&format!("x-api-key: {KIT_API_KEY}").as_str()),
            "{lines:?}"
        );
        assert!(lines.contains(&format!("x-client-version: {}", finlight::VERSION).as_str()));
        assert!(lines.contains(&"x-takeover: true"));
        assert!(
            lines.contains(
                &format!("user-agent: finlight-client-rust/{}", finlight::VERSION).as_str()
            )
        );
        assert!(
            !lines
                .iter()
                .any(|l| l.starts_with("X-API-KEY") || l.starts_with("X-Api-Key"))
        );
        stream.close().await;
        server.abort();
        spinner.abort();
    })
}

fn s05() {
    run_paused(async {
        let s = play(&session("unknown-event.json"), Opts::default()).await;
        {
            let o = s.probe.obs();
            let other = o
                .items
                .iter()
                .find(|i| i.kind == StreamEventKind::Other)
                .unwrap();
            assert_eq!(other.event, "connection.draining");
            assert!(other.data.is_none());
            let frame: Value = serde_json::from_str(other.raw_json()).unwrap();
            assert_eq!(frame["data"]["inMs"], 30000);
            assert!(o.errors.is_empty());
        }
        s.finish().await;
    })
}

fn s07() {
    run_paused(async {
        let s = play(
            &session("validation-failed-forbidden-key.json"),
            Opts {
                forbidden: vec!["includeContent"],
                ..Default::default()
            },
        )
        .await;
        s.finish().await;
    })
}

fn s08() {
    run_paused(async {
        let fixture = session("raw-include-rejected.json");
        // Locally: include on the raw stream is rejected before any dial.
        let err = RawStreamParams::from_json(r#"{"include":["content"]}"#).unwrap_err();
        match err {
            Error::Validation(v) => assert_eq!(v.param(), Some("include")),
            other => panic!("expected a validation error, got {other:?}"),
        }
        // If the server rejects anyway, the stream ends with the terminal validation error.
        let s = play(
            &fixture,
            Opts {
                forbidden: vec!["include"],
                ..Default::default()
            },
        )
        .await;
        s.finish().await;
    })
}

fn s12() {
    run_paused(async {
        let s = play(&session("v2-invalid-json-error.json"), Opts::default()).await;
        {
            let o = s.probe.obs();
            assert_eq!(o.errors.len(), 1);
            assert_eq!(
                o.errors[0].detail.as_deref(),
                Some("Message processing failed")
            );
        }
        s.finish().await;
    })
}

fn s14() {
    run_paused(async {
        let mut s = play(&session("search-failed.json"), Opts::default()).await;
        let failed = json!({"event": "connection.error", "payloadVersion": 3,
            "data": {"code": "SEARCH_FAILED", "message": "Search temporarily unavailable"}});
        let count = |s: &Session| {
            s.probe
                .obs()
                .errors
                .iter()
                .filter(|e| e.code.as_deref() == Some("SEARCH_FAILED"))
                .count()
        };
        let admitted = json!({"event": "connection.admitted", "payloadVersion": 3,
            "data": {"leaseId": "l", "serverNow": 1795467845123_i64, "clientNonce": null}});
        // Failures 2 to 5 in a row, each after connection.admitted like the real server: only
        // the 5th is surfaced, the stream keeps reconnecting, and admitted does not reset the
        // backoff (the wait after failure k is at least 2^(k-2) s).
        let mut waits = Vec::new();
        for k in 2..=5u32 {
            assert_eq!(count(&s), 0);
            let conn = s.ws.dial(s.seen_dials - 1).conn.unwrap();
            conn.send(&admitted);
            conn.send(&failed);
            // The client closes the connection when it handles SEARCH_FAILED and starts its
            // backoff sleep in the same step; only then may the clock move.
            wait_for("the client to close after SEARCH_FAILED", || !conn.open()).await;
            settle().await;
            let waited = s.advance_until_dial(60_000).await;
            assert!(
                waited >= Duration::from_secs(1 << (k - 2)),
                "the wait grows: {waited:?} after failure {k}"
            );
            assert!(
                waited <= Duration::from_secs(31),
                "capped by reconnect_max_delay"
            );
            waits.push(waited);
            s.take_redial().await;
        }
        assert!(waits[3] > waits[0], "backoff grows: {waits:?}");
        assert_eq!(count(&s), 1);
        assert!(!s.probe.obs().done);
        s.finish().await;
    })
}

fn s16() {
    run_paused(async {
        let s = play(
            &session("pong-watchdog.json"),
            Opts {
                auto_pong: Some(false),
                ..Default::default()
            },
        )
        .await;
        s.finish().await;
    })
}

fn s17() {
    run_paused(async {
        let s = play(&session("raw-stream.json"), Opts::default()).await;
        let (first_seen, article) = {
            let o = s.probe.obs();
            let a = o.items[0].data.clone().unwrap();
            (a.first_seen_at.clone(), a)
        };
        assert!(!article.content_present() && !article.sentiment_present());
        assert!(!article.companies_present() && !article.events_present());
        assert!(article.available_at.is_none());
        let last = s.finish().await;
        assert_eq!(last, first_seen);
    })
}

fn s18() {
    run_paused(async {
        let mut fixture = session("rotation.json");
        let pushed = load("ws-sessions/admitted-nonce.json")["steps"][3]["frame"].clone();
        let admitted = fixture["steps"][2]["frame"].clone();
        // Deliver the initial push once before the rotation.
        let steps = fixture["steps"].as_array_mut().unwrap();
        steps.insert(3, json!({"step": "serverSends", "frame": pushed}));
        steps.insert(
            4,
            json!({"step": "expect", "outcome": "item", "kind": "created"}),
        );
        let s = play(&fixture, Opts::default()).await;
        let first = s.ws.dial(0).conn.unwrap();
        let second = s.ws.dial(1).conn.unwrap();
        assert!(
            first.open(),
            "the old connection stays until the new one is admitted"
        );
        second.send(&admitted);
        second.send(&pushed);
        settle().await;
        wait_for("old connection closed", || first.closed_code().is_some()).await;
        assert_eq!(first.closed_code(), Some(Some(1000)));
        {
            let o = s.probe.obs();
            assert_eq!(
                o.items
                    .iter()
                    .filter(|i| i.kind == StreamEventKind::Created)
                    .count(),
                1,
                "the replayed initial push is dropped"
            );
            assert_eq!(o.reconnects.len(), 1);
            assert_eq!(o.reconnects[0].reason.as_str(), "rotation");
            assert_eq!(
                o.reconnects[0].last_seen_at.as_deref(),
                pushed["data"]["availableAt"].as_str()
            );
        }
        s.finish().await;
    })
}

/// Kit cases of this module.
pub fn cases() -> Vec<crate::common::Case> {
    vec![
        ("R-13", Box::new(r13)),
        ("R-14", Box::new(r14)),
        ("S-01", Box::new(s01)),
        ("S-02", Box::new(s02)),
        ("S-03", Box::new(simple("admitted-nonce.json"))),
        ("S-04", Box::new(simple("created-then-updated.json"))),
        ("S-05", Box::new(s05)),
        ("S-06", Box::new(simple("ping-on-default-route.json"))),
        ("S-07", Box::new(s07)),
        ("S-08", Box::new(s08)),
        ("S-09", Box::new(simple("preempted-connection-limit.json"))),
        ("S-10", Box::new(simple("preempted-quota-exceeded.json"))),
        ("S-11", Box::new(simple("v2-preempt-before-subscribe.json"))),
        ("S-12", Box::new(s12)),
        ("S-13", Box::new(simple("handshake-failed.json"))),
        ("S-14", Box::new(s14)),
        ("S-15", Box::new(simple("internal-error.json"))),
        ("S-16", Box::new(s16)),
        ("S-17", Box::new(s17)),
        ("S-18", Box::new(s18)),
        ("S-19", Box::new(simple("dial-missing-key.json"))),
        ("S-20", Box::new(simple("dial-invalid-key.json"))),
        ("S-21", Box::new(simple("dial-malformed-key.json"))),
        ("S-22", Box::new(simple("dial-no-websockets.json"))),
        ("S-23", Box::new(simple("dial-too-many-connections.json"))),
        ("S-24", Box::new(simple("dial-quota-exceeded.json"))),
        ("S-25", Box::new(simple("dial-admit-race.json"))),
        ("S-26", Box::new(simple("dial-server-error.json"))),
        (
            "S-27",
            Box::new(simple("preempted-quota-exceeded-no-reconnect.json")),
        ),
        (
            "S-28",
            Box::new(simple("dial-quota-exceeded-no-reconnect.json")),
        ),
    ]
}

/// F-03, WebSocket part: an article frame and a control frame with added keys.
pub fn f03_ws(envelopes: &[Value]) {
    let get = |kind: &str| envelopes.iter().find(|e| e["kind"] == kind).unwrap()["wire"].clone();
    let article = get("ws.article");
    let control = get("ws.control");
    run_paused(async move {
        let fixture = json!({"endpoint": "enriched", "steps": [
            {"step": "dial", "server": {"status": 101}},
            {"step": "serverSends", "frame": {"event": "connection.admitted", "payloadVersion": 3,
                "data": {"leaseId": "l", "serverNow": 1, "clientNonce": null, "region": "eu"}}},
            {"step": "expect", "outcome": "admitted"},
            {"step": "serverSends", "frame": control},
            {"step": "serverSends", "frame": article},
            {"step": "expect", "outcome": "item", "kind": "created", "id": "EU8JwpsZ8iJehuyPZjMqRz"},
        ]});
        let s = play(&fixture, Opts::default()).await;
        {
            let o = s.probe.obs();
            assert!(o.errors.is_empty() && o.terminal.is_none());
            let frame: Value = serde_json::from_str(o.items[0].raw_json()).unwrap();
            assert_eq!(frame["sequence"], 17);
        }
        s.finish().await;
    })
}
