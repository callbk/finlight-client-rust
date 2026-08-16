# finlight-client (Rust)

*[English](README.md) | [简体中文](README.zh-CN.md) | [日本語](README.ja.md) | 한국어*

[![crates.io](https://img.shields.io/crates/v/finlight-client)](https://crates.io/crates/finlight-client)
[![docs.rs](https://img.shields.io/docsrs/finlight-client)](https://docs.rs/finlight-client)
[![CI](https://github.com/callbk/finlight-client-rust/actions/workflows/test.yml/badge.svg)](https://github.com/callbk/finlight-client-rust/actions/workflows/test.yml)

[finlight.me](https://finlight.me) API의 공식 Rust 클라이언트입니다. 감성 분석, 엔티티 인식, 실시간 스트리밍을 갖춘 금융 뉴스를 제공합니다.

📚 **전체 API 문서: [docs.finlight.me](https://docs.finlight.me)**

## 주요 기능

- **REST API**: 기사 검색, 링크로 단일 기사 조회, 뉴스 소스 목록 조회
- **실시간 스트리밍**: WebSocket을 통한 enhanced 및 raw 기사 스트림을 비동기 `Stream`으로 제공
- **기본적으로 견고함**: 지수 백오프를 적용한 요청 재시도, WebSocket 자동 재연결, pong 워치독 기반 킵얼라이브, 선제적 연결 교체, 속도 제한 처리
- **Webhook 지원**: HMAC-SHA256 서명 검증 및 재생 공격 방지
- **비동기**: tokio, reqwest, tokio-tungstenite(rustls 사용, OpenSSL 불필요) 기반

Rust 1.85 이상이 필요합니다.

## 설치

```bash
cargo add finlight-client tokio futures-util
```

## 빠른 시작

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

### 기사 검색

`query`의 전체 문법은 [쿼리 언어 레퍼런스](https://docs.finlight.me)를 참조하세요.

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

### 링크로 기사 조회

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

### 뉴스 소스 목록 조회

```rust
let sources = client.sources.get_sources().await?;
```

## WebSocket 스트리밍

스트림은 [`futures::Stream`](https://docs.rs/futures-core/latest/futures_core/stream/trait.Stream.html)을 구현합니다. `StreamExt::next`로 소비하고, drop하면 연결이 끊깁니다. 재연결, 킵얼라이브, 속도 제한 대기는 내부에서 처리됩니다. `Err` 항목은 종단 항목이며, 이를 반환한 시점에 스트림이 종료됩니다.

### Enhanced 스트림(감성, 엔티티, 중복 제거 포함)

```rust
use finlight_client::GetArticlesWebSocketParams;
use futures_util::StreamExt;

let mut stream = client.websocket.stream(GetArticlesWebSocketParams {
    tickers: Some(vec!["AAPL".into(), "NVDA".into()]),
    include_content: Some(true),
    ..Default::default()
});
while let Some(item) = stream.next().await {
    let article = item?; // 예: Error::Blocked
    println!(
        "{}: {} (sentiment: {})",
        article.source,
        article.title,
        article.sentiment.as_deref().unwrap_or("-")
    );
}
```

### Raw 스트림(최저 지연, 보강 없음)

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

### WebSocket 옵션 사용자 지정

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
    takeover: true, // 동일한 키의 기존 연결을 테이크오버
    on_close: Some(Arc::new(|code, reason| {
        eprintln!("connection closed: {code} {reason}");
    })),
});
```

## Webhook

finlight 대시보드에서 발급받은 엔드포인트 시크릿으로 수신한 Webhook을 검증합니다:

```rust
use finlight_client::construct_webhook_event;

// raw_body: 수정되지 않은 요청 본문 바이트
// signature: X-Webhook-Signature 헤더("sha256=" 접두사는 있어도 없어도 됩니다)
// timestamp: X-Webhook-Timestamp 헤더, 없으면 None
let article = construct_webhook_event(
    &raw_body,
    &signature,
    &std::env::var("FINLIGHT_WEBHOOK_SECRET")?,
    timestamp.as_deref(),
)?;
println!("new article: {}", article.title);
```

## 설정

| 필드          | 기본값                    | 설명                                             |
| ------------- | ------------------------- | ------------------------------------------------ |
| `api_key`     | —(필수)                   | 사용자의 finlight API 키                         |
| `base_url`    | `https://api.finlight.me` | REST 기본 URL                                    |
| `wss_url`     | `wss://wss.finlight.me`   | WebSocket URL(raw는 `/raw`가 덧붙습니다)         |
| `timeout`     | `5s`                      | 요청당 타임아웃                                  |
| `retry_count` | `3`                       | 요청 총 시도 횟수                                |

WebSocket 옵션 기본값: ping은 `25s`마다, pong 타임아웃 `60s`, 재연결 백오프 `500ms → 10s`, `115min` 후 선제적 교체.

### 로깅

클라이언트는 [`tracing`](https://docs.rs/tracing)을 통해 로그를 남기며, subscriber를 설치하기 전까지는 아무것도 출력하지 않습니다:

```rust
tracing_subscriber::fmt()
    .with_env_filter("finlight_client=debug")
    .init();
```

## 오류 처리

- REST: 재시도 대상 상태 코드(429, 500, 502, 503, 504)는 지수 백오프로 재시도됩니다. 그 외 실패는 `Error::Api { status, body }`를 반환합니다.
- 스트리밍: 스트림을 drop하거나 서버가 연결을 선점하면 스트림은 조용히 종료됩니다. `Err` 항목은 종단 항목이며, `Error::Blocked`는 서버가 연결을 영구적으로 거부했음을 의미합니다.
- Webhook: 검증 실패 시 `WebhookVerificationError`를 반환합니다.

## 테스트

```bash
cargo test                                            # 단위 + 프로토콜 테스트(오프라인)
FINLIGHT_API_KEY=sk_... cargo test -- --ignored       # 실제 API 대상
FINLIGHT_API_KEY=sk_... cargo run --example smoke     # 일회성 스모크 실행
```

## 라이선스

MIT — [LICENSE](LICENSE) 참조.

## 지원

- 문서: https://docs.finlight.me
- 웹사이트: https://finlight.me
- 한국어 제품 페이지: https://finlight.me/ko/news-api
