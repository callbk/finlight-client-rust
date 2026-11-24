//! WebSocket dial: transport, TLS, handshake and refusal bodies.
//!
//! The upgrade request is written by the SDK itself, byte by byte. That keeps the header names
//! exactly as the gateway needs them (`x-api-key`, lowercase), lets the SDK read the body of a
//! refused dial in full, and keeps the request (and the API key) out of every library's logs.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};

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
/// It opens a plain byte stream to `host:port`. For `wss://` URLs the SDK adds TLS on top,
/// then performs the WebSocket handshake with its own headers.
pub trait WsConnector: Send + Sync + 'static {
    /// Opens a byte stream to `host:port`.
    fn connect(&self, host: &str, port: u16) -> WsConnectFuture;
}

pub(crate) type WsStream = WebSocketStream<Box<dyn WsIo>>;

pub(crate) enum DialOutcome {
    Open(Box<WsStream>),
    Refused { status: u16, body: Option<String> },
    Failed(Error),
}

const MAX_HEAD: usize = 64 * 1024;
const MAX_REFUSAL_BODY: usize = 64 * 1024;

fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
            let builder = rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .expect("aws-lc-rs supports the default TLS versions");
            Arc::new(builder.with_root_certificates(roots).with_no_client_auth())
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
    let name = rustls::pki_types::ServerName::try_from(target.host.clone())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let tls = tokio_rustls::TlsConnector::from(tls_config())
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

async fn read_body(io: &mut Box<dyn WsIo>, head: &Head) -> Option<String> {
    let mut body = Vec::new();
    match head
        .header("content-length")
        .and_then(|v| v.parse::<usize>().ok())
    {
        Some(len) => {
            body.resize(len.min(MAX_REFUSAL_BODY), 0);
            io.read_exact(&mut body).await.ok()?;
        }
        None => {
            let mut chunk = [0u8; 4096];
            while body.len() < MAX_REFUSAL_BODY {
                match io.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => body.extend_from_slice(&chunk[..n]),
                }
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

async fn handshake(cfg: &Config, target: &Target, takeover: bool) -> DialOutcome {
    let mut io = match open_io(cfg, target).await {
        Ok(io) => io,
        Err(e) => {
            return DialOutcome::Failed(Error::network(
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
        return DialOutcome::Failed(Error::network(
            "the stream handshake failed",
            Some(Box::new(e)),
        ));
    }
    drop(request);
    let head = match read_head(&mut io).await {
        Ok(h) => h,
        Err(e) => {
            return DialOutcome::Failed(Error::network(
                "the stream handshake failed",
                Some(Box::new(e)),
            ));
        }
    };
    if head.status != 101 {
        let body = read_body(&mut io, &head).await;
        return DialOutcome::Refused {
            status: head.status,
            body,
        };
    }
    let upgrade_ok = head
        .header("upgrade")
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
        && head.header("sec-websocket-accept") == Some(derive_accept_key(key.as_bytes()).as_str());
    if !upgrade_ok {
        return DialOutcome::Failed(Error::network(
            "the stream handshake failed: the server did not confirm the WebSocket upgrade",
            None,
        ));
    }
    let ws_config = WebSocketConfig::default()
        .max_message_size(Some(cfg.max_message_bytes))
        .max_frame_size(Some(cfg.max_message_bytes));
    let ws = WebSocketStream::from_raw_socket(io, Role::Client, Some(ws_config)).await;
    DialOutcome::Open(Box::new(ws))
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
        Ok(outcome) => outcome,
        Err(_) => DialOutcome::Failed(Error::timeout(
            format!(
                "the stream dial did not finish within connect_timeout ({:?})",
                cfg.connect_timeout
            ),
            None,
        )),
    }
}
