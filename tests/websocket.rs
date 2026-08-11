//! WebSocket protocol tests against a local axum server: handshake, dedup,
//! preemption, blocked close code, reconnect, keepalive, rotation, consumer
//! drop, and exact on-the-wire header casing.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::any;
use finlight_client::{
    Config, Error, GetArticlesWebSocketParams, GetRawArticlesWebSocketParams, RawWebSocketClient,
    WebSocketClient, WebSocketOptions,
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Mutex, mpsc};

fn test_config(addr: &str) -> Config {
    let mut cfg = Config::new("test-key");
    cfg.wss_url = format!("ws://{addr}");
    cfg
}

/// Fast reconnect delays keep the reconnect tests quick.
fn fast_options() -> WebSocketOptions {
    WebSocketOptions {
        base_reconnect_delay: Duration::from_millis(10),
        max_reconnect_delay: Duration::from_millis(50),
        ..Default::default()
    }
}

fn article_msg(link: &str) -> Value {
    json!({
        "action": "sendArticle",
        "data": {
            "link": link,
            "title": format!("Title {link}"),
            "publishDate": "2024-01-01T00:00:00Z",
            "source": "example.com",
            "language": "en",
        },
    })
}

async fn send_json(socket: &mut WebSocket, v: &Value) {
    socket
        .send(Message::Text(v.to_string().into()))
        .await
        .unwrap();
}

/// Reads the next text message and parses it as JSON.
async fn recv_json(socket: &mut WebSocket) -> Value {
    loop {
        match socket.recv().await {
            Some(Ok(Message::Text(text))) => return serde_json::from_str(&text).unwrap(),
            Some(Ok(_)) => continue,
            other => panic!("connection ended before message: {other:?}"),
        }
    }
}

/// Drains the connection until the peer closes; returns the close code if a
/// close frame was seen.
async fn wait_close(socket: &mut WebSocket) -> Option<u16> {
    loop {
        match socket.recv().await {
            Some(Ok(Message::Close(frame))) => return frame.map(|f| f.code),
            Some(Ok(_)) => continue,
            Some(Err(_)) | None => return None,
        }
    }
}

/// Builds a WS test server whose per-connection script is `handler`.
async fn serve_ws<F, Fut>(handler: F) -> String
where
    F: Fn(u32, HeaderMap, WebSocket) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let conns = Arc::new(AtomicU32::new(0));
    let app = Router::new()
        .route(
            "/",
            any(ws_entry::<F, Fut>).with_state((conns.clone(), handler.clone())),
        )
        .route("/raw", any(ws_entry::<F, Fut>).with_state((conns, handler)));
    common::serve(app).await
}

async fn ws_entry<F, Fut>(
    State((conns, handler)): State<(Arc<AtomicU32>, F)>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response
where
    F: Fn(u32, HeaderMap, WebSocket) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let conn = conns.fetch_add(1, Ordering::SeqCst) + 1;
    ws.on_upgrade(move |socket| handler(conn, headers, socket))
}

#[tokio::test]
async fn stream_handshake_dedup_and_preempt() {
    let captured: Arc<Mutex<Option<(HeaderMap, Value)>>> = Arc::default();
    let state = captured.clone();
    let addr = serve_ws(move |_conn, headers, mut socket| {
        let state = state.clone();
        async move {
            let handshake = recv_json(&mut socket).await;
            send_json(
                &mut socket,
                &json!({"action": "admit", "leaseId": "lease-1", "clientNonce": handshake["clientNonce"]}),
            )
            .await;
            send_json(&mut socket, &article_msg("https://example.com/a")).await;
            send_json(&mut socket, &article_msg("https://example.com/a")).await; // duplicate
            send_json(&mut socket, &article_msg("https://example.com/b")).await;
            send_json(&mut socket, &json!({"action": "preempted", "reason": "test over"})).await;
            *state.lock().await = Some((headers, handshake));
            wait_close(&mut socket).await;
        }
    })
    .await;

    let ws = WebSocketClient::new(
        test_config(&addr),
        WebSocketOptions {
            takeover: true,
            ..Default::default()
        },
    );
    let mut links = Vec::new();
    let mut stream = ws.stream(GetArticlesWebSocketParams {
        query: Some("nvidia".into()),
        ..Default::default()
    });
    while let Some(item) = stream.next().await {
        links.push(item.unwrap().link);
    }

    assert_eq!(
        links,
        ["https://example.com/a", "https://example.com/b"],
        "duplicate must be suppressed"
    );
    let (headers, handshake) = captured.lock().await.take().unwrap();
    assert_eq!(headers.get("x-api-key").unwrap(), "test-key");
    assert!(
        headers
            .get("x-client-version")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("rust/finlight-client-rust@")
    );
    assert_eq!(headers.get("x-takeover").unwrap(), "true");
    assert_eq!(handshake["query"], "nvidia");
    assert_eq!(handshake["clientNonce"].as_str().unwrap().len(), 36);
}

#[tokio::test]
async fn raw_stream_does_not_dedup() {
    let addr = serve_ws(|_conn, _headers, mut socket| async move {
        recv_json(&mut socket).await;
        send_json(&mut socket, &article_msg("https://example.com/a")).await;
        send_json(&mut socket, &article_msg("https://example.com/a")).await;
        send_json(&mut socket, &json!({"action": "preempted"})).await;
        wait_close(&mut socket).await;
    })
    .await;

    let ws = RawWebSocketClient::new(test_config(&addr), WebSocketOptions::default());
    let count = ws
        .stream(GetRawArticlesWebSocketParams::default())
        .filter(|item| std::future::ready(item.is_ok()))
        .count()
        .await;
    assert_eq!(count, 2, "raw stream must not dedup");
}

#[tokio::test]
async fn stream_blocked_close_code_is_terminal() {
    let addr = serve_ws(|_conn, _headers, mut socket| async move {
        recv_json(&mut socket).await;
        let _ = socket
            .send(Message::Close(Some(CloseFrame {
                code: 1008,
                reason: "blocked".into(),
            })))
            .await;
    })
    .await;

    let close_code = Arc::new(AtomicU16::new(0));
    let seen = close_code.clone();
    let ws = WebSocketClient::new(
        test_config(&addr),
        WebSocketOptions {
            on_close: Some(Arc::new(move |code, _reason| {
                seen.store(code, Ordering::SeqCst);
            })),
            ..Default::default()
        },
    );

    let mut stream = ws.stream(GetArticlesWebSocketParams::default());
    let item = stream.next().await.expect("terminal error expected");
    assert!(matches!(item, Err(Error::Blocked)), "got {item:?}");
    assert!(stream.next().await.is_none(), "stream must end after error");
    assert_eq!(close_code.load(Ordering::SeqCst), 1008);
}

#[tokio::test]
async fn stream_reconnects_after_server_close() {
    let addr = serve_ws(|conn, _headers, mut socket| async move {
        recv_json(&mut socket).await;
        if conn == 1 {
            let _ = socket
                .send(Message::Close(Some(CloseFrame {
                    code: 1000,
                    reason: "bye".into(),
                })))
                .await;
            return;
        }
        send_json(
            &mut socket,
            &article_msg("https://example.com/after-reconnect"),
        )
        .await;
        send_json(&mut socket, &json!({"action": "preempted"})).await;
        wait_close(&mut socket).await;
    })
    .await;

    let ws = WebSocketClient::new(test_config(&addr), fast_options());
    let links: Vec<_> = ws
        .stream(GetArticlesWebSocketParams::default())
        .map(|item| item.unwrap().link)
        .collect()
        .await;
    assert_eq!(links, ["https://example.com/after-reconnect"]);
}

#[tokio::test]
async fn stream_answers_pings_and_rotates_proactively() {
    let (done_tx, mut done_rx) = mpsc::channel::<(Value, Option<u16>)>(1);
    let addr = serve_ws(move |conn, _headers, mut socket| {
        let done_tx = done_tx.clone();
        async move {
            recv_json(&mut socket).await;
            if conn == 1 {
                // Answer the client's application-level ping, then wait for
                // the proactive rotation close (4000).
                let ping = recv_json(&mut socket).await;
                send_json(&mut socket, &json!({"action": "pong", "t": ping["t"]})).await;
                let code = wait_close(&mut socket).await;
                let _ = done_tx.send((ping, code)).await;
                return;
            }
            send_json(&mut socket, &article_msg("https://example.com/second-conn")).await;
            send_json(&mut socket, &json!({"action": "preempted"})).await;
            wait_close(&mut socket).await;
        }
    })
    .await;

    let mut opts = fast_options();
    opts.ping_interval = Duration::from_millis(50);
    opts.connection_lifetime = Duration::from_millis(300);
    let ws = WebSocketClient::new(test_config(&addr), opts);
    let links: Vec<_> = ws
        .stream(GetArticlesWebSocketParams::default())
        .map(|item| item.unwrap().link)
        .collect()
        .await;

    assert_eq!(links, ["https://example.com/second-conn"]);
    let (ping, close_code) = done_rx.recv().await.unwrap();
    assert_eq!(ping["action"], "ping");
    assert!(ping["t"].is_i64(), "ping must carry a millis timestamp");
    assert_eq!(close_code, Some(4000), "rotation must close with 4000");
}

#[tokio::test]
async fn dropping_stream_disconnects() {
    let (closed_tx, mut closed_rx) = mpsc::channel::<()>(1);
    let addr = serve_ws(move |_conn, _headers, mut socket| {
        let closed_tx = closed_tx.clone();
        async move {
            recv_json(&mut socket).await;
            for i in 0..5 {
                send_json(
                    &mut socket,
                    &article_msg(&format!("https://example.com/{i}")),
                )
                .await;
            }
            wait_close(&mut socket).await;
            let _ = closed_tx.send(()).await;
        }
    })
    .await;

    let ws = WebSocketClient::new(test_config(&addr), WebSocketOptions::default());
    let mut stream = ws.stream(GetArticlesWebSocketParams::default());
    let first = stream.next().await.unwrap().unwrap();
    assert_eq!(first.link, "https://example.com/0");
    drop(stream);

    tokio::time::timeout(Duration::from_secs(5), closed_rx.recv())
        .await
        .expect("server must observe the disconnect");
}

/// The wss.finlight.me authorizer reads headers case-sensitively in exact
/// lowercase. Assert the raw bytes on the wire, not the parsed headers.
#[tokio::test]
async fn handshake_headers_are_lowercase_on_the_wire() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let (head_tx, mut head_rx) = mpsc::channel::<String>(1);
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = sock.read(&mut chunk).await.unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        let _ = head_tx
            .send(String::from_utf8_lossy(&buf).into_owned())
            .await;
        let _ = sock
            .write_all(b"HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\n\r\n")
            .await;
    });

    let ws = WebSocketClient::new(test_config(&addr), fast_options());
    let stream = ws.stream(GetArticlesWebSocketParams::default());
    let head = tokio::time::timeout(Duration::from_secs(5), head_rx.recv())
        .await
        .unwrap()
        .unwrap();
    drop(stream);

    assert!(
        head.contains("x-api-key: test-key"),
        "x-api-key must be lowercase on the wire:\n{head}"
    );
    assert!(
        head.contains("x-client-version: rust/finlight-client-rust@"),
        "x-client-version must be lowercase on the wire:\n{head}"
    );
    assert!(
        !head.contains("X-Api-Key") && !head.contains("X-API-KEY"),
        "canonicalized header casing must not appear:\n{head}"
    );
}
