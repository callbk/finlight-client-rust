//! A small HTTP/1.1 server on 127.0.0.1. It records every request as received (raw head
//! and body bytes) and answers with scripted responses. One request per connection.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    /// Header lines as received: (name exactly as sent, value).
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub head: String,
}

impl Recorded {
    /// Header value, name matched case-insensitively.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn body_text(&self) -> String {
        String::from_utf8(self.body.clone()).unwrap()
    }

    pub fn body_json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

#[derive(Debug, Clone)]
pub struct Scripted {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Scripted {
    pub fn json(status: u16, body: &Value) -> Self {
        Scripted {
            status,
            headers: vec![(
                "content-type".into(),
                "application/json; charset=utf-8".into(),
            )],
            body: body.to_string().into_bytes(),
        }
    }

    /// From a fixture `response` object (`status`, `headers`, `body`).
    pub fn from_fixture(resp: &Value) -> Self {
        let status = resp["status"].as_u64().unwrap() as u16;
        let headers = resp["headers"]
            .as_object()
            .map(|h| {
                h.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
                    .collect()
            })
            .unwrap_or_default();
        let body = match &resp["body"] {
            Value::Null => Vec::new(),
            Value::String(s) => s.clone().into_bytes(),
            other => other.to_string().into_bytes(),
        };
        Scripted {
            status,
            headers,
            body,
        }
    }
}

#[derive(Default)]
struct State {
    requests: Vec<Recorded>,
    queue: VecDeque<Scripted>,
    always: Option<Scripted>,
}

#[derive(Clone)]
pub struct HttpMock {
    pub base_url: String,
    state: Arc<Mutex<State>>,
}

impl HttpMock {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(State::default()));
        let st = state.clone();
        tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    return;
                };
                let st = st.clone();
                tokio::spawn(async move {
                    let _ = serve(socket, st).await;
                });
            }
        });
        HttpMock {
            base_url: format!("http://{addr}"),
            state,
        }
    }

    pub fn enqueue(&self, response: Scripted) {
        self.state.lock().unwrap().queue.push_back(response);
    }

    pub fn always(&self, response: Scripted) {
        self.state.lock().unwrap().always = Some(response);
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.state.lock().unwrap().requests.clone()
    }

    pub fn count(&self) -> usize {
        self.state.lock().unwrap().requests.len()
    }

    pub fn reset(&self) {
        let mut s = self.state.lock().unwrap();
        s.requests.clear();
        s.queue.clear();
        s.always = None;
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        _ => "Status",
    }
}

async fn serve(mut socket: TcpStream, state: Arc<Mutex<State>>) -> std::io::Result<()> {
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
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split(' ');
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default().to_owned();
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_owned(), Some(q.to_owned())),
        None => (target.clone(), None),
    };
    let headers: Vec<(String, String)> = lines
        .filter(|l| !l.is_empty())
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.to_owned(), v.trim().to_owned()))
        .collect();
    let length = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    socket.read_exact(&mut body).await?;
    let response = {
        let mut s = state.lock().unwrap();
        s.requests.push(Recorded {
            method,
            path,
            query,
            headers,
            body,
            head: head.clone(),
        });
        s.queue
            .pop_front()
            .or_else(|| s.always.clone())
            .unwrap_or(Scripted {
                status: 599,
                headers: vec![],
                body: b"no scripted response".to_vec(),
            })
    };
    let mut out = format!(
        "HTTP/1.1 {} {}\r\n",
        response.status,
        reason(response.status)
    );
    for (k, v) in &response.headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    if response.status != 304 {
        out.push_str(&format!("content-length: {}\r\n", response.body.len()));
    }
    out.push_str("connection: close\r\n\r\n");
    socket.write_all(out.as_bytes()).await?;
    if response.status != 304 {
        socket.write_all(&response.body).await?;
    }
    socket.flush().await?;
    socket.shutdown().await?;
    Ok(())
}
