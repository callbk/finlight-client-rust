# finlight-client (Rust)

[![crates.io](https://img.shields.io/crates/v/finlight-client)](https://crates.io/crates/finlight-client)
[![docs.rs](https://img.shields.io/docsrs/finlight-client)](https://docs.rs/finlight-client)
[![CI](https://github.com/callbk/finlight-client-rust/actions/workflows/test.yml/badge.svg)](https://github.com/callbk/finlight-client-rust/actions/workflows/test.yml)

The official Rust client for the [finlight.me](https://finlight.me) API — financial news with sentiment analysis, entity recognition, and real-time streaming.

📚 **Full API documentation: [docs.finlight.me](https://docs.finlight.me)**

## Features

- **REST API**: search articles, fetch single articles by link, list sources
- **Real-time streaming**: enhanced and raw article streams over WebSocket, exposed as async `Stream`s
- **Resilient by default**: request retries with exponential backoff; WebSocket auto-reconnect, keepalive with pong watchdog, proactive connection rotation, and rate-limit handling
- **Webhook support**: HMAC-SHA256 signature verification with replay protection
- **Async**: built on tokio, reqwest, and tokio-tungstenite (rustls, no OpenSSL)

Requires Rust 1.85+.

## Installation

```bash
cargo add finlight-client tokio futures-util
```

## Quick Start

```rust
use finlight_client::{Client, Config, GetArticlesParams};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(Config::new(std::env::var("FINLIGHT_API_KEY")?))?;

    let resp = client
        .articles
        .fetch_articles(&GetArticlesParams {
            query: Some("nvidia".into()),
            page_size: Some(10),
            ..Default::default()
        })
        .await?;
    for article in resp.articles {
        println!("[{}] {}", article.source, article.title);
    }
    Ok(())
}
```

## REST API

### Search articles

See the [query language reference](https://docs.finlight.me) for the full `query` syntax.

```rust
use finlight_client::{Category, GetArticlesParams, OrderBy, SortOrder};

let resp = client
    .articles
    .fetch_articles(&GetArticlesParams {
        query: Some(r#"(ticker:AAPL OR ticker:NVDA) AND "Elon Musk""#.into()),
        from: Some("2024-01-01".into()),
        to: Some("2024-12-31".into()),
        language: Some("en".into()),
        categories: Some(vec![Category::Technology, Category::Markets]),
        include_content: Some(true),
        include_entities: Some(true),
        order_by: Some(OrderBy::PublishDate),
        order: Some(SortOrder::Desc),
        page_size: Some(50),
        ..Default::default()
    })
    .await?;
```

### Fetch an article by link

```rust
use finlight_client::GetArticleByLinkParams;

let article = client
    .articles
    .fetch_article_by_link(&GetArticleByLinkParams {
        link: "https://example.com/some-article".into(),
        include_content: true,
        include_entities: false,
    })
    .await?;
```

### List sources

```rust
let sources = client.sources.get_sources().await?;
```

## WebSocket Streaming

Streams implement [`futures::Stream`](https://docs.rs/futures-core/latest/futures_core/stream/trait.Stream.html): consume them with `StreamExt::next`, drop them to disconnect. Reconnects, keepalive, and rate-limit waits are handled internally. An `Err` item is terminal — the stream ends after yielding it.

### Enhanced stream (sentiment, entities, deduplicated)

```rust
use finlight_client::GetArticlesWebSocketParams;
use futures_util::StreamExt;

let mut stream = client.websocket.stream(GetArticlesWebSocketParams {
    tickers: Some(vec!["AAPL".into(), "NVDA".into()]),
    include_content: Some(true),
    ..Default::default()
});
while let Some(item) = stream.next().await {
    let article = item?; // e.g. Error::Blocked
    println!(
        "{}: {} (sentiment: {})",
        article.source,
        article.title,
        article.sentiment.as_deref().unwrap_or("-")
    );
}
```

### Raw stream (lowest latency, no enrichment)

```rust
use finlight_client::GetRawArticlesWebSocketParams;

let mut stream = client.raw_websocket.stream(GetRawArticlesWebSocketParams {
    sources: Some(vec!["www.reuters.com".into()]),
    ..Default::default()
});
while let Some(item) = stream.next().await {
    println!("{}", item?.title);
}
```

### Custom WebSocket options

```rust
use std::sync::Arc;
use std::time::Duration;
use finlight_client::{WebSocketClient, WebSocketOptions};

let ws = WebSocketClient::new(config, WebSocketOptions {
    ping_interval: Duration::from_secs(25),
    pong_timeout: Duration::from_secs(60),
    base_reconnect_delay: Duration::from_millis(500),
    max_reconnect_delay: Duration::from_secs(10),
    connection_lifetime: Duration::from_secs(115 * 60),
    takeover: true, // take over an existing connection for this key
    on_close: Some(Arc::new(|code, reason| {
        eprintln!("connection closed: {code} {reason}");
    })),
});
```

## Webhooks

Verify incoming webhooks with your endpoint secret from the finlight dashboard:

```rust
use finlight_client::construct_webhook_event;

// raw_body: unmodified request body bytes
// signature: X-Webhook-Signature header (with or without "sha256=" prefix)
// timestamp: X-Webhook-Timestamp header, None if absent
let article = construct_webhook_event(
    &raw_body,
    &signature,
    &std::env::var("FINLIGHT_WEBHOOK_SECRET")?,
    timestamp.as_deref(),
)?;
println!("new article: {}", article.title);
```

## Configuration

| Field         | Default                   | Description                             |
| ------------- | ------------------------- | --------------------------------------- |
| `api_key`     | — (required)              | Your finlight API key                   |
| `base_url`    | `https://api.finlight.me` | REST base URL                           |
| `wss_url`     | `wss://wss.finlight.me`   | WebSocket URL (`/raw` appended for raw) |
| `timeout`     | `5s`                      | Per-request timeout                     |
| `retry_count` | `3`                       | Total request attempts                  |

WebSocket option defaults: ping every `25s`, pong timeout `60s`, reconnect backoff `500ms → 10s`, proactive rotation after `115min`.

### Logging

The client logs through [`tracing`](https://docs.rs/tracing) and is silent unless you install a subscriber:

```rust
tracing_subscriber::fmt()
    .with_env_filter("finlight_client=debug")
    .init();
```

## Error Handling

- REST: retryable statuses (429, 500, 502, 503, 504) are retried with exponential backoff; other failures return `Error::Api { status, body }`.
- Streaming: the stream ends silently when you drop it or the server preempts the connection. An `Err` item is terminal — `Error::Blocked` means the server permanently rejected the connection.
- Webhooks: verification failures return `WebhookVerificationError`.

## Testing

```bash
cargo test                                            # unit + protocol tests (offline)
FINLIGHT_API_KEY=sk_... cargo test -- --ignored       # against the live API
FINLIGHT_API_KEY=sk_... cargo run --example smoke     # one-shot smoke run
```

## License

MIT — see [LICENSE](LICENSE).

## Support

- Documentation: https://docs.finlight.me
- Website: https://finlight.me
