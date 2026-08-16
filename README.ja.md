# finlight-client (Rust)

*[English](README.md) | [简体中文](README.zh-CN.md) | 日本語 | [한국어](README.ko.md)*

[![crates.io](https://img.shields.io/crates/v/finlight-client)](https://crates.io/crates/finlight-client)
[![docs.rs](https://img.shields.io/docsrs/finlight-client)](https://docs.rs/finlight-client)
[![CI](https://github.com/callbk/finlight-client-rust/actions/workflows/test.yml/badge.svg)](https://github.com/callbk/finlight-client-rust/actions/workflows/test.yml)

[finlight.me](https://finlight.me) API の公式 Rust クライアントです。センチメント分析、エンティティ認識、リアルタイムストリーミングを備えた金融ニュースを提供します。

📚 **完全な API ドキュメント: [docs.finlight.me](https://docs.finlight.me)**

## 主な機能

- **REST API**: 記事の検索、リンクによる単一記事の取得、ニュースソースの一覧取得
- **リアルタイムストリーミング**: WebSocket 経由の enhanced および raw 記事ストリームを、非同期の `Stream` として提供
- **標準で堅牢**: 指数バックオフによるリクエストのリトライ、WebSocket の自動再接続、pong ウォッチドッグ付きキープアライブ、先回りのコネクションローテーション、レート制限のハンドリング
- **Webhook 対応**: HMAC-SHA256 の署名検証とリプレイ攻撃対策
- **非同期**: tokio、reqwest、tokio-tungstenite（rustls 使用、OpenSSL 不要）の上に構築

Rust 1.85 以降が必要です。

## インストール

```bash
cargo add finlight-client tokio futures-util
```

## クイックスタート

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

### 記事を検索する

`query` の完全な構文は[クエリ言語リファレンス](https://docs.finlight.me)を参照してください。

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

### リンクから記事を取得する

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

### ニュースソースを一覧取得する

```rust
let sources = client.sources.get_sources().await?;
```

## WebSocket ストリーミング

ストリームは [`futures::Stream`](https://docs.rs/futures-core/latest/futures_core/stream/trait.Stream.html) を実装しています。`StreamExt::next` で消費し、drop すると切断されます。再接続、キープアライブ、レート制限による待機は内部で処理されます。`Err` 要素は終端であり、それを返した時点でストリームは終了します。

### Enhanced ストリーム（センチメント、エンティティ、重複除去あり）

```rust
use finlight_client::GetArticlesWebSocketParams;
use futures_util::StreamExt;

let mut stream = client.websocket.stream(GetArticlesWebSocketParams {
    tickers: Some(vec!["AAPL".into(), "NVDA".into()]),
    include_content: Some(true),
    ..Default::default()
});
while let Some(item) = stream.next().await {
    let article = item?; // 例: Error::Blocked
    println!(
        "{}: {} (sentiment: {})",
        article.source,
        article.title,
        article.sentiment.as_deref().unwrap_or("-")
    );
}
```

### Raw ストリーム（最小遅延、エンリッチメントなし）

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

### WebSocket オプションのカスタマイズ

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
    takeover: true, // 同じキーの既存コネクションをテイクオーバーする
    on_close: Some(Arc::new(|code, reason| {
        eprintln!("connection closed: {code} {reason}");
    })),
});
```

## Webhook

finlight ダッシュボードで取得したエンドポイントシークレットで、受信した Webhook を検証します:

```rust
use finlight_client::construct_webhook_event;

// raw_body: 変更されていないリクエストボディのバイト列
// signature: X-Webhook-Signature ヘッダー（"sha256=" 接頭辞の有無は問いません）
// timestamp: X-Webhook-Timestamp ヘッダー。存在しない場合は None
let article = construct_webhook_event(
    &raw_body,
    &signature,
    &std::env::var("FINLIGHT_WEBHOOK_SECRET")?,
    timestamp.as_deref(),
)?;
println!("new article: {}", article.title);
```

## 設定

| フィールド    | デフォルト                | 説明                                               |
| ------------- | ------------------------- | -------------------------------------------------- |
| `api_key`     | —（必須）                 | あなたの finlight API キー                         |
| `base_url`    | `https://api.finlight.me` | REST のベース URL                                  |
| `wss_url`     | `wss://wss.finlight.me`   | WebSocket の URL（raw では `/raw` が付加されます） |
| `timeout`     | `5s`                      | リクエストごとのタイムアウト                       |
| `retry_count` | `3`                       | リクエストの総試行回数                             |

WebSocket オプションのデフォルト: ping は `25s` ごと、pong タイムアウト `60s`、再接続バックオフ `500ms → 10s`、`115min` 経過後に先回りローテーション。

### ロギング

クライアントは [`tracing`](https://docs.rs/tracing) 経由でログを出力し、subscriber を設定するまでは何も出力しません:

```rust
tracing_subscriber::fmt()
    .with_env_filter("finlight_client=debug")
    .init();
```

## エラーハンドリング

- REST: リトライ対象のステータス（429、500、502、503、504）は指数バックオフでリトライされます。それ以外の失敗は `Error::Api { status, body }` を返します。
- ストリーミング: ストリームを drop した場合、またはサーバーが接続を先取りした場合、ストリームは静かに終了します。`Err` 要素は終端であり、`Error::Blocked` はサーバーが接続を恒久的に拒否したことを意味します。
- Webhook: 検証失敗時は `WebhookVerificationError` を返します。

## テスト

```bash
cargo test                                            # ユニット + プロトコルテスト（オフライン）
FINLIGHT_API_KEY=sk_... cargo test -- --ignored       # 実 API に対して実行
FINLIGHT_API_KEY=sk_... cargo run --example smoke     # 一度きりのスモーク実行
```

## ライセンス

MIT — [LICENSE](LICENSE) を参照してください。

## サポート

- ドキュメント: https://docs.finlight.me
- ウェブサイト: https://finlight.me
- 日本語の製品ページ: https://finlight.me/ja/news-api
