//! WebSocket dial and framing: transport, TLS, handshake, refusal bodies and frames.
//!
//! The SDK speaks the WebSocket protocol itself (RFC 6455, client side, no extensions). The
//! upgrade request is written byte by byte: that keeps the header names exactly as the gateway
//! needs them (`x-api-key`, lowercase), lets the SDK read the body of a refused dial in full,
//! and keeps the request (and the API key) out of every library's logs. Frames are read and
//! written here too, so no transport library ever logs a frame payload (an article body or a
//! subscribe frame).
//!
//! TLS uses rustls with the operating system's trust store (`rustls-platform-verifier`), the
//! same verifier reqwest uses for REST.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use sha1::{Digest, Sha1};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::Instant;

use crate::config::Config;
use crate::error::Error;
use crate::version::{VERSION, user_agent};

/// A byte stream for a WebSocket connection. Any `AsyncRead + AsyncWrite` tokio stream fits.
pub trait WsIo: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> WsIo for T {}

/// Future returned by [`WsConnector::connect`].
pub type WsConnectFuture = Pin<Box<dyn Future<Output = std::io::Result<Box<dyn WsIo>>> + Send>>;

/// Your own transport for stream dials, e.g. a tunnel through a proxy.
///
/// It opens a plain byte stream to `host:port`. For `wss://` URLs the SDK adds TLS on top
/// (verified against the operating system's trust store, like REST), then performs the
/// WebSocket handshake with its own headers.
pub trait WsConnector: Send + Sync + 'static {
    /// Opens a byte stream to `host:port`.
    fn connect(&self, host: &str, port: u16) -> WsConnectFuture;
}

pub(crate) enum DialOutcome {
    Open(Box<WsConn>),
    Refused { status: u16, body: Option<String> },
    Failed(Error),
}

const MAX_HEAD: usize = 64 * 1024;
const MAX_REFUSAL_BODY: usize = 64 * 1024;
/// Longest wait for the body of a refused dial. A body that does not arrive in time counts as
/// missing; the refusal is then judged by its status alone.
const REFUSAL_BODY_TIMEOUT: Duration = Duration::from_secs(5);
const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

fn tls_config() -> std::result::Result<Arc<rustls::ClientConfig>, String> {
    static CONFIG: OnceLock<std::result::Result<Arc<rustls::ClientConfig>, String>> =
        OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let provider = rustls::crypto::CryptoProvider::get_default()
                .cloned()
                .unwrap_or_else(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()));
            let verifier = rustls_platform_verifier::Verifier::new(provider.clone())
                .map_err(|e| format!("could not load the system trust store: {e}"))?;
            let builder = rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map_err(|e| format!("could not set up TLS: {e}"))?;
            Ok(Arc::new(
                builder
                    .dangerous()
                    .with_custom_certificate_verifier(Arc::new(verifier))
                    .with_no_client_auth(),
            ))
        })
        .clone()
}

struct Target {
    tls: bool,
    host: String,
    port: u16,
    host_header: String,
    path: String,
}

fn parse_url(url: &str) -> Option<Target> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let tls = match parsed.scheme() {
        "wss" => true,
        "ws" => false,
        _ => return None,
    };
    let host = parsed.host_str()?.trim_matches(['[', ']']).to_owned();
    let port = parsed.port_or_known_default()?;
    let host_header = match parsed.port() {
        Some(p) => format!("{}:{p}", parsed.host_str()?),
        None => parsed.host_str()?.to_owned(),
    };
    let mut path = parsed.path().to_owned();
    if path.is_empty() {
        path.push('/');
    }
    if let Some(q) = parsed.query() {
        path.push('?');
        path.push_str(q);
    }
    Some(Target {
        tls,
        host,
        port,
        host_header,
        path,
    })
}

fn header_safe(value: &str) -> bool {
    value.bytes().all(|b| (0x20..=0x7e).contains(&b))
}

fn random_bytes<const N: usize>() -> [u8; N] {
    // uuid's v4 draws from the operating system's random source (getrandom).
    let mut out = [0u8; N];
    for chunk in out.chunks_mut(16) {
        let id = uuid::Uuid::new_v4();
        chunk.copy_from_slice(&id.as_bytes()[..chunk.len()]);
    }
    out
}

fn generate_key() -> String {
    BASE64.encode(random_bytes::<16>())
}

fn accept_key(key: &str) -> String {
    let mut sha = Sha1::new();
    sha.update(key.as_bytes());
    sha.update(WS_GUID.as_bytes());
    BASE64.encode(sha.finalize())
}

async fn open_io(cfg: &Config, target: &Target) -> std::io::Result<Box<dyn WsIo>> {
    let io: Box<dyn WsIo> = match &cfg.ws_connector {
        Some(c) => c.connect(&target.host, target.port).await?,
        None => {
            let stream =
                tokio::net::TcpStream::connect((target.host.as_str(), target.port)).await?;
            let _ = stream.set_nodelay(true);
            Box::new(stream)
        }
    };
    if !target.tls {
        return Ok(io);
    }
    let config = tls_config().map_err(io::Error::other)?;
    let name = rustls::pki_types::ServerName::try_from(target.host.clone())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let tls = tokio_rustls::TlsConnector::from(config)
        .connect(name, io)
        .await?;
    Ok(Box::new(tls))
}

struct Head {
    status: u16,
    headers: Vec<(String, String)>,
}

impl Head {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

async fn read_head(io: &mut Box<dyn WsIo>) -> std::io::Result<Head> {
    let bad = |m: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, m.to_owned());
    let mut buf = Vec::with_capacity(512);
    let mut byte = [0u8; 1];
    // Read byte by byte: nothing after the head may be consumed (WebSocket frames follow).
    while !buf.ends_with(b"\r\n\r\n") {
        if io.read(&mut byte).await? == 0 {
            return Err(bad("the connection closed during the handshake"));
        }
        buf.push(byte[0]);
        if buf.len() > MAX_HEAD {
            return Err(bad("the handshake response head is too large"));
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let mut lines = text.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let mut parts = status_line.split(' ');
    if !parts.next().unwrap_or_default().starts_with("HTTP/1.") {
        return Err(bad("the handshake response is not HTTP/1.x"));
    }
    let status = parts
        .next()
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| bad("the handshake response has no status"))?;
    let headers = lines
        .filter(|l| !l.is_empty())
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    Ok(Head { status, headers })
}

/// Reads one byte before `deadline`; `None` at the end of the stream, on an error or when the
/// deadline passed.
async fn read_byte(io: &mut Box<dyn WsIo>, deadline: Instant) -> Option<u8> {
    let mut byte = [0u8; 1];
    match tokio::time::timeout_at(deadline, io.read(&mut byte)).await {
        Ok(Ok(1)) => Some(byte[0]),
        _ => None,
    }
}

/// Reads up to `want` bytes into `body` before `deadline`; `false` when the stream ended early.
async fn read_exactly(
    io: &mut Box<dyn WsIo>,
    body: &mut Vec<u8>,
    mut want: usize,
    deadline: Instant,
) -> bool {
    let mut chunk = [0u8; 4096];
    while want > 0 {
        let n = want.min(chunk.len());
        match tokio::time::timeout_at(deadline, io.read(&mut chunk[..n])).await {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => return false,
            Ok(Ok(got)) => {
                body.extend_from_slice(&chunk[..got]);
                want -= got;
            }
        }
    }
    true
}

/// Decodes a `Transfer-Encoding: chunked` body (trailers are not read).
async fn read_chunked(io: &mut Box<dyn WsIo>, body: &mut Vec<u8>, deadline: Instant) {
    loop {
        let mut line = Vec::new();
        while !line.ends_with(b"\r\n") {
            let Some(b) = read_byte(io, deadline).await else {
                return;
            };
            line.push(b);
            if line.len() > 1024 {
                return;
            }
        }
        let text = String::from_utf8_lossy(&line);
        let size = text.trim().split(';').next().unwrap_or_default().trim();
        let Ok(size) = usize::from_str_radix(size, 16) else {
            return;
        };
        if size == 0 || body.len() + size > MAX_REFUSAL_BODY {
            return;
        }
        if !read_exactly(io, body, size, deadline).await {
            return;
        }
        // The CRLF after the chunk data.
        for _ in 0..2 {
            if read_byte(io, deadline).await.is_none() {
                return;
            }
        }
    }
}

/// Reads the body of a refused dial: `Content-Length`, chunked, or up to the end of the
/// stream, for at most [`REFUSAL_BODY_TIMEOUT`]. A body that does not arrive in time is
/// treated as missing.
async fn read_body(io: &mut Box<dyn WsIo>, head: &Head) -> Option<String> {
    let deadline = Instant::now() + REFUSAL_BODY_TIMEOUT;
    let mut body = Vec::new();
    let chunked = head
        .header("transfer-encoding")
        .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    let length = head
        .header("content-length")
        .and_then(|v| v.parse::<usize>().ok());
    if chunked {
        read_chunked(io, &mut body, deadline).await;
    } else if let Some(len) = length {
        if !read_exactly(io, &mut body, len.min(MAX_REFUSAL_BODY), deadline).await {
            return None;
        }
    } else {
        let mut chunk = [0u8; 4096];
        while body.len() < MAX_REFUSAL_BODY {
            match tokio::time::timeout_at(deadline, io.read(&mut chunk)).await {
                Ok(Ok(n)) if n > 0 => body.extend_from_slice(&chunk[..n]),
                Ok(Ok(_)) | Ok(Err(_)) => break,
                Err(_) => return None,
            }
        }
    }
    let text = String::from_utf8_lossy(&body).into_owned();
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

enum Handshake {
    Open(Box<WsConn>),
    Refused(u16, Head, Box<dyn WsIo>),
    Failed(Error),
}

async fn handshake(cfg: &Config, target: &Target, takeover: bool) -> Handshake {
    let mut io = match open_io(cfg, target).await {
        Ok(io) => io,
        Err(e) => {
            return Handshake::Failed(Error::network(
                "could not connect to the stream endpoint",
                Some(Box::new(e)),
            ));
        }
    };
    let key = generate_key();
    // Header names exactly like this: the gateway looks up x-api-key case-sensitively.
    let mut request = format!(
        "GET {} HTTP/1.1\r\nhost: {}\r\nconnection: Upgrade\r\nupgrade: websocket\r\n\
         sec-websocket-version: 13\r\nsec-websocket-key: {key}\r\nuser-agent: {}\r\n\
         x-client-version: {VERSION}\r\nx-api-key: {}\r\n",
        target.path,
        target.host_header,
        user_agent(),
        cfg.api_key(),
    );
    if takeover {
        request.push_str("x-takeover: true\r\n");
    }
    request.push_str("\r\n");
    let sent = async {
        io.write_all(request.as_bytes()).await?;
        io.flush().await
    };
    if let Err(e) = sent.await {
        return Handshake::Failed(Error::network(
            "the stream handshake failed",
            Some(Box::new(e)),
        ));
    }
    drop(request);
    let head = match read_head(&mut io).await {
        Ok(h) => h,
        Err(e) => {
            return Handshake::Failed(Error::network(
                "the stream handshake failed",
                Some(Box::new(e)),
            ));
        }
    };
    if head.status != 101 {
        return Handshake::Refused(head.status, head, io);
    }
    let upgrade_ok = head
        .header("upgrade")
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
        && head.header("sec-websocket-accept") == Some(accept_key(&key).as_str());
    if !upgrade_ok {
        return Handshake::Failed(Error::network(
            "the stream handshake failed: the server did not confirm the WebSocket upgrade",
            None,
        ));
    }
    Handshake::Open(Box::new(WsConn::new(io, cfg.max_message_bytes)))
}

/// Dials `url` with the SDK headers. The API key goes out only as the `x-api-key` header,
/// never in the URL.
pub(crate) async fn dial(cfg: &Config, url: &str, takeover: bool) -> DialOutcome {
    let Some(target) = parse_url(url) else {
        return DialOutcome::Failed(Error::validation(
            "wss_url is not a ws:// or wss:// URL",
            Some("wss_url"),
        ));
    };
    if !header_safe(cfg.api_key()) {
        return DialOutcome::Failed(Error::validation(
            "api_key is not a valid header value",
            Some("api_key"),
        ));
    }
    match tokio::time::timeout(cfg.connect_timeout, handshake(cfg, &target, takeover)).await {
        Ok(Handshake::Open(conn)) => DialOutcome::Open(conn),
        Ok(Handshake::Refused(status, head, mut io)) => {
            // The refusal has arrived; its body gets its own short time limit.
            let body = read_body(&mut io, &head).await;
            DialOutcome::Refused { status, body }
        }
        Ok(Handshake::Failed(err)) => DialOutcome::Failed(err),
        Err(_) => DialOutcome::Failed(Error::timeout(
            format!(
                "the stream dial did not finish within connect_timeout ({:?})",
                cfg.connect_timeout
            ),
            None,
        )),
    }
}

/// A message read from the server.
#[derive(Debug)]
pub(crate) enum WsMessage {
    Text(String),
    Binary,
    Ping(Vec<u8>),
    Pong,
    /// A close frame and its code (`None` without a code).
    Close(Option<u16>),
}

const OP_CONTINUATION: u8 = 0x0;
const OP_TEXT: u8 = 0x1;
const OP_BINARY: u8 = 0x2;
const OP_CLOSE: u8 = 0x8;
const OP_PING: u8 = 0x9;
const OP_PONG: u8 = 0xA;

fn protocol_error(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

/// An open WebSocket connection (client side).
pub(crate) struct WsConn {
    io: Box<dyn WsIo>,
    /// Bytes read but not yet parsed.
    buf: Vec<u8>,
    /// Opcode and data of a fragmented message in progress.
    partial: Option<(u8, Vec<u8>)>,
    max_message: usize,
    done: bool,
}

impl WsConn {
    fn new(io: Box<dyn WsIo>, max_message: usize) -> Self {
        WsConn {
            io,
            buf: Vec::new(),
            partial: None,
            max_message,
            done: false,
        }
    }

    /// The next message; `None` once the connection ended. Cancel-safe: a call dropped while
    /// waiting loses no data.
    pub(crate) async fn next(&mut self) -> Option<io::Result<WsMessage>> {
        if self.done {
            return None;
        }
        loop {
            match self.parse() {
                Ok(Some(msg)) => {
                    if matches!(msg, WsMessage::Close(_)) {
                        self.done = true;
                    }
                    return Some(Ok(msg));
                }
                Ok(None) => {}
                Err(e) => {
                    self.done = true;
                    return Some(Err(e));
                }
            }
            self.buf.reserve(8192);
            match self.io.read_buf(&mut self.buf).await {
                Ok(0) => {
                    self.done = true;
                    return None;
                }
                Ok(_) => {}
                Err(e) => {
                    self.done = true;
                    return Some(Err(e));
                }
            }
        }
    }

    /// Parses complete frames from the buffer until one completes a message.
    fn parse(&mut self) -> io::Result<Option<WsMessage>> {
        loop {
            let Some((fin, opcode, start, len)) = self.frame_header()? else {
                return Ok(None);
            };
            if self.buf.len() < start + len {
                return Ok(None);
            }
            let payload: Vec<u8> = self.buf[start..start + len].to_vec();
            self.buf.drain(..start + len);
            match opcode {
                OP_CLOSE => {
                    let code =
                        (payload.len() >= 2).then(|| u16::from_be_bytes([payload[0], payload[1]]));
                    return Ok(Some(WsMessage::Close(code)));
                }
                OP_PING => return Ok(Some(WsMessage::Ping(payload))),
                OP_PONG => return Ok(Some(WsMessage::Pong)),
                OP_TEXT | OP_BINARY => {
                    if self.partial.is_some() {
                        return Err(protocol_error(
                            "a new message started inside a fragmented one",
                        ));
                    }
                    if fin {
                        return finish(opcode, payload).map(Some);
                    }
                    self.partial = Some((opcode, payload));
                }
                OP_CONTINUATION => {
                    let Some((first, mut data)) = self.partial.take() else {
                        return Err(protocol_error("a continuation frame without a message"));
                    };
                    if data.len() + payload.len() > self.max_message {
                        return Err(protocol_error(
                            "the stream message is larger than max_message_bytes",
                        ));
                    }
                    data.extend_from_slice(&payload);
                    if fin {
                        return finish(first, data).map(Some);
                    }
                    self.partial = Some((first, data));
                }
                _ => return Err(protocol_error("unknown WebSocket opcode")),
            }
        }
    }

    /// `(fin, opcode, payload start, payload length)` of the frame at the start of the buffer.
    fn frame_header(&self) -> io::Result<Option<(bool, u8, usize, usize)>> {
        let b = &self.buf;
        if b.len() < 2 {
            return Ok(None);
        }
        let fin = b[0] & 0x80 != 0;
        if b[0] & 0x70 != 0 {
            return Err(protocol_error("reserved WebSocket bits are set"));
        }
        let opcode = b[0] & 0x0f;
        if b[1] & 0x80 != 0 {
            return Err(protocol_error("the server sent a masked frame"));
        }
        let (len, start) = match b[1] & 0x7f {
            126 => {
                if b.len() < 4 {
                    return Ok(None);
                }
                (u64::from(u16::from_be_bytes([b[2], b[3]])), 4)
            }
            127 => {
                if b.len() < 10 {
                    return Ok(None);
                }
                let mut n = [0u8; 8];
                n.copy_from_slice(&b[2..10]);
                (u64::from_be_bytes(n), 10)
            }
            n => (u64::from(n), 2),
        };
        if opcode >= 0x8 && (!fin || len > 125) {
            return Err(protocol_error("invalid WebSocket control frame"));
        }
        let partial = self.partial.as_ref().map_or(0, |(_, d)| d.len());
        if opcode < 0x8 && len > (self.max_message - partial.min(self.max_message)) as u64 {
            return Err(protocol_error(
                "the stream message is larger than max_message_bytes",
            ));
        }
        Ok(Some((fin, opcode, start, len as usize)))
    }

    async fn send(&mut self, opcode: u8, payload: &[u8]) -> io::Result<()> {
        let mut frame = Vec::with_capacity(payload.len() + 14);
        frame.push(0x80 | opcode);
        match payload.len() {
            n if n < 126 => frame.push(0x80 | n as u8),
            n if n <= usize::from(u16::MAX) => {
                frame.push(0x80 | 126);
                frame.extend_from_slice(&(n as u16).to_be_bytes());
            }
            n => {
                frame.push(0x80 | 127);
                frame.extend_from_slice(&(n as u64).to_be_bytes());
            }
        }
        let mask = random_bytes::<4>();
        frame.extend_from_slice(&mask);
        frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        self.io.write_all(&frame).await?;
        self.io.flush().await
    }

    pub(crate) async fn send_text(&mut self, text: &str) -> io::Result<()> {
        self.send(OP_TEXT, text.as_bytes()).await
    }

    pub(crate) async fn send_pong(&mut self, payload: &[u8]) -> io::Result<()> {
        self.send(OP_PONG, payload).await
    }

    pub(crate) async fn send_close(&mut self, code: u16) -> io::Result<()> {
        self.send(OP_CLOSE, &code.to_be_bytes()).await
    }
}

fn finish(opcode: u8, data: Vec<u8>) -> io::Result<WsMessage> {
    if opcode == OP_BINARY {
        return Ok(WsMessage::Binary);
    }
    String::from_utf8(data)
        .map(WsMessage::Text)
        .map_err(|_| protocol_error("a text frame is not valid UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_key_matches_rfc_6455() {
        assert_eq!(
            accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
        assert_eq!(generate_key().len(), 24);
    }

    fn conn(bytes: &[u8], max: usize) -> WsConn {
        let mut c = WsConn::new(Box::new(tokio::io::empty()), max);
        c.buf.extend_from_slice(bytes);
        c
    }

    #[test]
    fn parses_fragments_controls_and_limits() {
        // "Hel" + ping + "lo" (continuation), then a close with 1000.
        let mut bytes = vec![
            0x01, 3, b'H', b'e', b'l', 0x89, 1, b'p', 0x80, 2, b'l', b'o',
        ];
        bytes.extend_from_slice(&[0x88, 2, 0x03, 0xe8]);
        let mut c = conn(&bytes, 1024);
        assert!(matches!(c.parse().unwrap(), Some(WsMessage::Ping(p)) if p == b"p"));
        assert!(matches!(c.parse().unwrap(), Some(WsMessage::Text(t)) if t == "Hello"));
        assert!(matches!(
            c.parse().unwrap(),
            Some(WsMessage::Close(Some(1000)))
        ));
        assert!(c.parse().unwrap().is_none());

        // 16-bit length, split across reads.
        let text = "x".repeat(300);
        let mut bytes = vec![0x81, 126, 0x01, 0x2c];
        bytes.extend_from_slice(text.as_bytes());
        let mut c = conn(&bytes[..100], 1024);
        assert!(c.parse().unwrap().is_none());
        c.buf.extend_from_slice(&bytes[100..]);
        assert!(matches!(c.parse().unwrap(), Some(WsMessage::Text(t)) if t == text));

        assert!(conn(&bytes, 100).parse().is_err(), "over max_message_bytes");
        assert!(
            conn(&[0x81, 0x81, 0, 0, 0, 0, b'x'], 100).parse().is_err(),
            "masked"
        );
        assert!(
            conn(&[0x80, 1, b'x'], 100).parse().is_err(),
            "stray continuation"
        );
        assert!(conn(&[0x81, 1, 0xff], 100).parse().is_err(), "bad UTF-8");
    }
}
