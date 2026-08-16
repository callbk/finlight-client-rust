# finlight-client (Rust)

*[English](README.md) | 简体中文 | [日本語](README.ja.md) | [한국어](README.ko.md)*

[![crates.io](https://img.shields.io/crates/v/finlight-client)](https://crates.io/crates/finlight-client)
[![docs.rs](https://img.shields.io/docsrs/finlight-client)](https://docs.rs/finlight-client)
[![CI](https://github.com/callbk/finlight-client-rust/actions/workflows/test.yml/badge.svg)](https://github.com/callbk/finlight-client-rust/actions/workflows/test.yml)

[finlight.me](https://finlight.me) API 的官方 Rust 客户端 —— 提供带情感分析、实体识别和实时流式推送的财经新闻。

📚 **完整 API 文档：[docs.finlight.me](https://docs.finlight.me)**

## 功能特性

- **REST API**：检索文章、按链接获取单篇文章、列出新闻源
- **实时流式推送**：通过 WebSocket 提供 enhanced 和 raw 两种文章流，以异步 `Stream` 形式暴露
- **默认具备韧性**：请求失败按指数退避重试；WebSocket 自动重连、带 pong 看门狗的保活、主动连接轮换，以及速率限制处理
- **支持 Webhook**：HMAC-SHA256 签名验证，含重放防护
- **异步实现**：基于 tokio、reqwest 和 tokio-tungstenite（使用 rustls，不依赖 OpenSSL）

需要 Rust 1.85 及以上版本。

## 安装

```bash
cargo add finlight-client tokio futures-util
```

## 快速开始

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

### 检索文章

完整的 `query` 语法参见[查询语言参考](https://docs.finlight.me)。

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

### 按链接获取文章

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

### 列出新闻源

```rust
let sources = client.sources.get_sources().await?;
```

## WebSocket 流式推送

数据流实现了 [`futures::Stream`](https://docs.rs/futures-core/latest/futures_core/stream/trait.Stream.html)：用 `StreamExt::next` 消费，drop 即断开连接。重连、保活和速率限制等待均在内部处理。`Err` 项是终止项 —— 流在产生它之后即结束。

### Enhanced 流（含情感、实体，已去重）

```rust
use finlight_client::GetArticlesWebSocketParams;
use futures_util::StreamExt;

let mut stream = client.websocket.stream(GetArticlesWebSocketParams {
    tickers: Some(vec!["AAPL".into(), "NVDA".into()]),
    include_content: Some(true),
    ..Default::default()
});
while let Some(item) = stream.next().await {
    let article = item?; // 例如 Error::Blocked
    println!(
        "{}: {} (sentiment: {})",
        article.source,
        article.title,
        article.sentiment.as_deref().unwrap_or("-")
    );
}
```

### Raw 流（延迟最低，无增强处理）

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

### 自定义 WebSocket 选项

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
    takeover: true, // 接管该密钥下已有的连接
    on_close: Some(Arc::new(|code, reason| {
        eprintln!("connection closed: {code} {reason}");
    })),
});
```

## Webhook

使用 finlight 控制台中的 endpoint secret 验证收到的 Webhook：

```rust
use finlight_client::construct_webhook_event;

// raw_body：未经修改的请求体字节
// signature：X-Webhook-Signature 请求头（可带或不带 "sha256=" 前缀）
// timestamp：X-Webhook-Timestamp 请求头，缺失时为 None
let article = construct_webhook_event(
    &raw_body,
    &signature,
    &std::env::var("FINLIGHT_WEBHOOK_SECRET")?,
    timestamp.as_deref(),
)?;
println!("new article: {}", article.title);
```

## 配置

| 字段          | 默认值                    | 说明                                    |
| ------------- | ------------------------- | --------------------------------------- |
| `api_key`     | ——（必填）                | 你的 finlight API 密钥                  |
| `base_url`    | `https://api.finlight.me` | REST 基础地址                           |
| `wss_url`     | `wss://wss.finlight.me`   | WebSocket 地址（raw 流会追加 `/raw`）   |
| `timeout`     | `5s`                      | 单次请求超时                            |
| `retry_count` | `3`                       | 请求总尝试次数                          |

WebSocket 选项默认值：每 `25s` 发送一次 ping，pong 超时 `60s`，重连退避 `500ms → 10s`，`115min` 后主动轮换连接。

### 日志

客户端通过 [`tracing`](https://docs.rs/tracing) 输出日志，未安装 subscriber 时保持静默：

```rust
tracing_subscriber::fmt()
    .with_env_filter("finlight_client=debug")
    .init();
```

## 错误处理

- REST：可重试的状态码（429、500、502、503、504）会按指数退避重试；其他失败返回 `Error::Api { status, body }`。
- 流式推送：当你 drop 数据流或服务端抢占连接时，流会静默结束。`Err` 项是终止项 —— `Error::Blocked` 表示服务端永久拒绝了该连接。
- Webhook：验证失败返回 `WebhookVerificationError`。

## 测试

```bash
cargo test                                            # 单元测试 + 协议测试（离线）
FINLIGHT_API_KEY=sk_... cargo test -- --ignored       # 针对线上 API
FINLIGHT_API_KEY=sk_... cargo run --example smoke     # 一次性冒烟测试
```

## 许可证

MIT —— 参见 [LICENSE](LICENSE)。

## 支持

- 文档：https://docs.finlight.me
- 官网：https://finlight.me
- 中文产品页：https://finlight.me/zh/news-api
