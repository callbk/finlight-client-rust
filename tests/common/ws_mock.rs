//! An in-process WebSocket server on 127.0.0.1. It reads the dial request itself (so the raw
//! header bytes are visible), refuses or accepts each dial as scripted, answers application
//! pings with a v3 pong when `auto_pong` is on, and lets the test send frames and close.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Role, WebSocketConfig};

#[derive(Debug, Clone)]
pub enum DialScript {
    Accept,
    Refuse {
        status: u16,
        body: Option<String>,
    },
    /// Writes these bytes as the answer and keeps the socket open until the client closes it.
    Raw(String),
}

enum Cmd {
    Send(String),
    Close(u16),
}

/// The server side of one accepted connection.
#[derive(Clone)]
pub struct ServerConn {
    tx: mpsc::UnboundedSender<Cmd>,
    received: Arc<Mutex<VecDeque<Value>>>,
    open: Arc<AtomicBool>,
    /// Close code the client sent: `Some(None)` closed without a code.
    pub closed_code: Arc<Mutex<Option<Option<u16>>>>,
    pings: Arc<AtomicUsize>,
}

impl ServerConn {
    pub fn send(&self, frame: &Value) {
        let text = match frame {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let _ = self.tx.send(Cmd::Send(text));
    }

    pub fn close(&self, code: u16) {
        let _ = self.tx.send(Cmd::Close(code));
    }

    pub fn open(&self) -> bool {
        self.open.load(Ordering::SeqCst)
    }

    pub fn closed_code(&self) -> Option<Option<u16>> {
        *self.closed_code.lock().unwrap()
    }

    /// Next frame the client sent (pings excluded while auto-pong is on).
    pub fn try_next(&self) -> Option<Value> {
        self.received.lock().unwrap().pop_front()
    }

    pub fn pending(&self) -> usize {
        self.received.lock().unwrap().len()
    }

    pub fn pings_answered(&self) -> usize {
        self.pings.load(Ordering::SeqCst)
    }

    pub async fn next(&self, what: &str) -> Value {
        let mut got = None;
        super::wait_for(what, || {
            got = self.try_next();
            got.is_some()
        })
        .await;
        got.unwrap()
    }
}

#[derive(Clone)]
pub struct Dial {
    pub request_line: String,
    pub path: String,
    pub head: String,
    pub headers: Vec<(String, String)>,
    pub conn: Option<ServerConn>,
}

impl Dial {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Default)]
struct State {
    script: VecDeque<DialScript>,
    dials: Vec<Dial>,
}

#[derive(Clone)]
pub struct WsMock {
    pub url: String,
    state: Arc<Mutex<State>>,
    pub auto_pong: Arc<AtomicBool>,
}

impl WsMock {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(State::default()));
        let auto_pong = Arc::new(AtomicBool::new(true));
        let (st, ap) = (state.clone(), auto_pong.clone());
        tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    return;
                };
                // Without TCP_NODELAY, Nagle holds back a small frame while the previous one is
                // unacknowledged, and on Linux loopback the client's delayed ACK takes ~40 ms of
                // real time. The paused clock does not wait for that, so the frame would arrive
                // virtual seconds late (or after a check). The SDK sets TCP_NODELAY too.
                let _ = socket.set_nodelay(true);
                let (st, ap) = (st.clone(), ap.clone());
                tokio::spawn(async move {
                    let _ = serve(socket, st, ap).await;
                });
            }
        });
        WsMock {
            url: format!("ws://{addr}"),
            state,
            auto_pong,
        }
    }

    pub fn script(&self, dial: DialScript) {
        self.state.lock().unwrap().script.push_back(dial);
    }

    pub fn dial_count(&self) -> usize {
        self.state.lock().unwrap().dials.len()
    }

    pub fn dial(&self, index: usize) -> Dial {
        self.state.lock().unwrap().dials[index].clone()
    }

    pub fn dials(&self) -> Vec<Dial> {
        self.state.lock().unwrap().dials.clone()
    }

    pub fn open_connections(&self) -> usize {
        self.state
            .lock()
            .unwrap()
            .dials
            .iter()
            .filter(|d| d.conn.as_ref().is_some_and(ServerConn::open))
            .count()
    }

    /// Waits for dial number `index` (0-based) and returns it.
    pub async fn wait_dial(&self, index: usize) -> Dial {
        super::wait_for(&format!("dial #{}", index + 1), || {
            self.dial_count() > index
        })
        .await;
        self.dial(index)
    }
}

async fn serve(
    mut socket: TcpStream,
    state: Arc<Mutex<State>>,
    auto_pong: Arc<AtomicBool>,
) -> std::io::Result<()> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if socket.read(&mut byte).await? == 0 {
            return Ok(());
        }
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default().to_owned();
    let path = request_line
        .split(' ')
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    let headers: Vec<(String, String)> = lines
        .filter(|l| !l.is_empty())
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.to_owned(), v.trim().to_owned()))
        .collect();
    let script = state
        .lock()
        .unwrap()
        .script
        .pop_front()
        .unwrap_or(DialScript::Accept);
    let mut dial = Dial {
        request_line,
        path,
        head,
        headers: headers.clone(),
        conn: None,
    };
    match script {
        DialScript::Raw(response) => {
            state.lock().unwrap().dials.push(dial);
            socket.write_all(response.as_bytes()).await?;
            socket.flush().await?;
            let mut sink = [0u8; 1024];
            while socket.read(&mut sink).await? > 0 {}
            Ok(())
        }
        DialScript::Refuse { status, body } => {
            state.lock().unwrap().dials.push(dial);
            let body = body.unwrap_or_default();
            let response = format!(
                "HTTP/1.1 {status} Refused\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await?;
            socket.flush().await?;
            let _ = socket.shutdown().await;
            Ok(())
        }
        DialScript::Accept => {
            let key = headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("sec-websocket-key"))
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            let response = format!(
                "HTTP/1.1 101 Switching Protocols\r\nupgrade: websocket\r\nconnection: Upgrade\r\nsec-websocket-accept: {}\r\n\r\n",
                derive_accept_key(key.as_bytes())
            );
            socket.write_all(response.as_bytes()).await?;
            let config = WebSocketConfig::default()
                .max_message_size(None)
                .max_frame_size(None);
            let ws = WebSocketStream::from_raw_socket(socket, Role::Server, Some(config)).await;
            let (tx, rx) = mpsc::unbounded_channel();
            let conn = ServerConn {
                tx,
                received: Arc::new(Mutex::new(VecDeque::new())),
                open: Arc::new(AtomicBool::new(true)),
                closed_code: Arc::new(Mutex::new(None)),
                pings: Arc::new(AtomicUsize::new(0)),
            };
            dial.conn = Some(conn.clone());
            state.lock().unwrap().dials.push(dial);
            run_conn(ws, conn, rx, auto_pong).await;
            Ok(())
        }
    }
}

async fn run_conn(
    ws: WebSocketStream<TcpStream>,
    conn: ServerConn,
    mut rx: mpsc::UnboundedReceiver<Cmd>,
    auto_pong: Arc<AtomicBool>,
) {
    let (mut sink, mut stream) = ws.split();
    loop {
        tokio::select! {
            msg = stream.next() => match msg {
                Some(Ok(Message::Text(text))) => {
                    let value: Value = serde_json::from_str(text.as_str()).unwrap_or(Value::String(text.as_str().to_owned()));
                    if auto_pong.load(Ordering::SeqCst) && value.get("action").and_then(Value::as_str) == Some("ping") {
                        let pong = serde_json::json!({
                            "event": "connection.pong",
                            "payloadVersion": 3,
                            "data": {"t": value.get("t").cloned().unwrap_or(Value::Null), "serverNow": 1_795_467_845_123_i64}
                        });
                        let _ = sink.send(Message::text(pong.to_string())).await;
                        conn.pings.fetch_add(1, Ordering::SeqCst);
                    } else {
                        conn.received.lock().unwrap().push_back(value);
                    }
                }
                Some(Ok(Message::Close(frame))) => {
                    *conn.closed_code.lock().unwrap() = Some(frame.map(|f| u16::from(f.code)));
                    conn.open.store(false, Ordering::SeqCst);
                    let _ = sink.close().await;
                    return;
                }
                Some(Ok(_)) => {}
                Some(Err(_)) | None => {
                    let mut code = conn.closed_code.lock().unwrap();
                    if code.is_none() {
                        *code = Some(None);
                    }
                    conn.open.store(false, Ordering::SeqCst);
                    return;
                }
            },
            cmd = rx.recv() => match cmd {
                Some(Cmd::Send(text)) => {
                    let _ = sink.send(Message::text(text)).await;
                }
                Some(Cmd::Close(code)) => {
                    let frame = CloseFrame { code: CloseCode::from(code), reason: "".into() };
                    let _ = sink.send(Message::Close(Some(frame))).await;
                    let _ = sink.flush().await;
                    conn.open.store(false, Ordering::SeqCst);
                    // Drop the socket after the close, like the gateway's DeleteConnection.
                    return;
                }
                None => return,
            }
        }
    }
}
