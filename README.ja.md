# finlight クライアント（Rust）

*[English](README.md) | [简体中文](README.zh-CN.md) | 日本語 | [한국어](README.ko.md)*

> **Breaking release.** 0.2 は finlight API v3 のみに対応し（REST `/v3`、WebSocket `payloadVersion` 3、v3 Webhook）、0.1 のコードは動きません。まず [0.1 からの移行](#01-からの移行)と[変更履歴](CHANGELOG.md)をお読みください。API v2 を使い続ける場合は `finlight-client = "0.1"` のままにしてください。

*この文書は英語版 README の翻訳で、英語版に合わせて順次更新されます。内容が異なる場合は英語版が優先されます。*

[finlight.me](https://finlight.me) 用の非同期クライアントです。金融ニュースの検索、記事・企業・ソースの参照、WebSocket による記事のライブ受信、Webhook の検証ができます。crate 名は `finlight-client`、ライブラリは `finlight` として import します。

## バージョン

| SDK version | API version | Status |
|---|---|---|
| 0.2.x | v3（`/v3`、WebSocket `payloadVersion` 3、Webhook v3） | 現行 |
| 0.1.x | v2 | 今後のリリースなし |

SDK と API のバージョンは独立しています。新しい API フィールドや列挙値は 0.2.x のパッチリリースで追加されます。API v3 に対応する SDK：TypeScript 4.x、Python 3.x、PHP 2.x、Go `/v2`、.NET 2.x、Java 1.x、Ruby 1.x、Rust 0.2。

Cargo は `"0.1"` を 0.2 に上げません。`"*"` のようなワイルドカードは上げるので、マイナーバージョンを固定してください。

## 動作要件

- Rust 1.85 以上（edition 2024）
- tokio 1.47 以上（ストリームごとにタスクを 1 つ起動するため tokio ランタイムが必要）
- reqwest 0.13（rustls）、serde 1、serde_json 1.0.142 以上。WebSocket プロトコルは SDK に内蔵
- TLS は rustls（aws-lc-rs プロバイダー）。REST もストリームも OS の証明書ストア（rustls-platform-verifier）で検証するため、そこに入れた社内 CA は両方で使えます。OpenSSL は不要

## インストール

```bash
cargo add finlight-client@0.2 tokio --features tokio/full
cargo add futures-util   # for StreamExt on streams
```

## クイックスタート

```rust,no_run
use finlight::{Client, Config, IncludeBlock, SearchParams};

#[tokio::main]
async fn main() -> Result<(), finlight::Error> {
    let client = Client::new(Config::new(std::env::var("FINLIGHT_API_KEY").unwrap_or_default()))?;
    let page = client
        .articles()
        .search(
            &SearchParams::new()
                .query("ticker:NVDA AND eventType:earnings")
                .languages(["en"])
                .include([IncludeBlock::COMPANIES, IncludeBlock::EVENTS])
                .limit(20),
        )
        .await?;
    for article in &page.articles {
        println!("{:?} {}", article.published_at, article.title);
    }
    Ok(())
}
```

SDK は指定したパラメータだけを送ります。既定値は追加しません。`limit`（20）、`languages`（`["en"]`）、期間はサーバーが決めます。パラメータ名は構造体のフィールドなので、綴りを間違えるとコンパイルできません。`SearchParams::from_json` は未知のキーをリクエスト前に `Error::Validation` で拒否します。

## 記事の検索

`articles().search()` は 1 ページを返します。`articles().search_all()` はカーソルをたどります。

```rust,no_run
use finlight::{Client, Config, OrderBy, SearchAllOptions, SearchParams, SortOrder};

#[tokio::main]
async fn main() -> Result<(), finlight::Error> {
    let client = Client::new(Config::new("your-api-key"))?;

    // One page
    let params = SearchParams::new()
        .tickers(["AAPL"])
        .order_by(OrderBy::AVAILABLE_AT)
        .order(SortOrder::DESC);
    let page = client.articles().search(&params).await?;
    println!("{} {:?}", page.pagination.has_more, page.pagination.next_cursor);

    // Many pages: the request body is frozen, only the cursor changes
    let params = SearchParams::new().query("eventType:mna").limit(100);
    let mut all = client.articles().search_all(params, SearchAllOptions::new().max_pages(5))?;
    while let Some(article) = all.next().await {
        let article = article?;
        println!("{} {}", article.id, article.title);
    }
    if all.truncated() {
        println!("continue with cursor {:?}", all.next_cursor());
    }
    Ok(())
}
```

- `search_all` は呼び出し時にリクエストボディを固定します。以降のページで変わるのは `cursor` だけなので、カーソルは有効なままです。
- `has_more: false`、`next_cursor: None`、または `max_pages`（既定 10）で止まります。上限に達すると警告を記録し、`truncated()` が true になり、`next_cursor()` で続きを取得できます。
- 各ページは課金対象の呼び出しです。結果は 1 回だけ反復できます。記事単位（`next()` または `futures` の `Stream`）かページ単位（`next_page()`）のどちらかで、もう一方は `Error::Validation` を返します。
- `from` と `to` は `SystemTime`（UTC、ミリ秒付きで送信）、`DateInput::date(y, m, d)`（`YYYY-MM-DD` で送信）、文字列（そのまま送信）を受け付けます。`order_by` のタイムスタンプに適用されます。
- `source_types` と `exclude_source_types` はソースの種類で絞り込みます：`SourceType::NEWSWIRE`、`PRESS_RELEASE`、`FINANCIAL_MEDIA`、`GENERAL_MEDIA`、`AGGREGATOR`、`BLOG`、`GOVERNMENT`。未分類のソースの記事は `source_types` に一致せず、`exclude_source_types` では残ります。どちらもソースプールの中で絞り込むだけで、ソースを追加しません。`StreamParams` と `RawStreamParams` でも使えます。
- デコードできないエントリはスキップされ、ログに記録され、`page.skipped` に入ります。ページの残りはそのまま使えます。

## 記事・企業・ソースの参照

```rust,no_run
use finlight::{Client, CompanySearchParams, Config, GetArticleOptions};

#[tokio::main]
async fn main() -> Result<(), finlight::Error> {
    let client = Client::new(Config::new("your-api-key"))?;
    let with_content = GetArticleOptions::new().include(["content"]);
    let article = client.articles().get("pXhK9f2aQ1mZ0bT7cY3rLw", &with_content).await?;
    let by_link = client
        .articles()
        .get_by_link("https://www.reuters.com/markets/example", &GetArticleOptions::new())
        .await?;
    println!("{} / {}", article.title, by_link.title);

    let found = client.companies().search(&CompanySearchParams::new("NVDA").mic("XNAS")).await?;
    let many = client.companies().get_many(&[4821, 1234]).await?;
    let record = client.companies().get(4821).await?; // ETag revalidation is built in
    println!("{} {:?} {}", found.len(), record.cache_info(), many.skipped().len());

    // With your own ETag: Ok(None) means "not modified"
    let fresh = client.companies().get_if_none_match(4821, "\"80b88d6b\"").await?;
    println!("changed: {}", fresh.is_some());

    let sources = client.sources().list().await?;
    println!("{} sources", sources.len());
    Ok(())
}
```

- 404 は空の結果ではなく、`api_kind() == ApiErrorKind::NotFound` の `Error::Api` です。
- ID 導入前にインデックスされた記事は ID で参照できません。`get_by_link` を使ってください。
- 企業エンドポイントには `entityAccess` 権限が必要です。ない場合は 403（`ApiErrorKind::Forbidden`）になります。
- `companies().get()` は ETag を保持して再検証します。304 のときは保存済みのレコードを返し、`cache_info().not_modified == true` になります。独自の ETag を使う `get_if_none_match` は 304 で `Ok(None)` を返します。
- 参照の呼び出しも記事と同じくクレジットを消費します。`companies()`（`search`、`get_many`、`get`、`get_if_none_match`）と `sources().list()` は成功したリクエストごとに 1 クレジットで、複数 ID の `get_many` も 1 クレジットです。304 の再検証とエラーは無料です。
- `CompanyList` と `SourceList` はスライスとして参照でき、デコードできなかったエントリは `skipped()` にあります。

## キーなし、null、空リスト

一部のキーはプランと `include` に依存します。キーがないこと、`null`、`[]` は意味が異なり、SDK はこれらを区別します。

```rust
use finlight::{Article, Field};

fn describe(article: &Article) {
    match &article.content {
        Field::Value(text) => println!("{} characters", text.len()),
        Field::Null => println!("requested and entitled, but there is no text"),
        Field::Absent => println!("not requested, not in your plan, or the raw channel"),
    }
    match &article.companies {
        Some(list) if list.is_empty() => println!("tagged, no company found"),
        Some(list) => println!("{} companies", list.len()),
        None => println!("companies not requested or not in your plan"),
    }
    // Timestamps stay strings, exactly as sent; the accessor parses strictly (RFC 3339).
    let published: Option<std::time::SystemTime> = article.published_at_time();
    println!("{:?} {}", published, article.companies_present());
}
```

- キーなし：リクエストしていない、プランに含まれない、または raw チャネル。
- `null`：リクエスト済みで権限もあるが、値がない（例：`content` に本文がない）。
- `[]`：本当に空のリスト（例：タグ付け済みだが企業が見つからない）。
- `content` と `sentiment` は `Field<T>`（`Absent`、`Null`、`Value`）です。`companies`、`events`、`events[].company_ids`、`Source::has_content` は `Option<_>` で、`None` は「キーなし」を意味し、「空」ではありません。
- `content_present()`、`sentiment_present()`、`companies_present()`、`events_present()` はキーが JSON にあったか（`null` を含む）を返します。
- 新しいサーバーからの未知のキーは無視されます。`raw_json()` は受信したままの JSON を返します。serde でモデルを書き戻しても、なかったキーはないままです。
- 語彙はオープンです。SDK が知らない値は `CompanyRole::Other("...")` として保持されます。`role == CompanyRole::ACQUIRER` のように定数と比較してください。

## リアルタイムストリーム

`client.stream()` はエンリッチ済みの記事を、`client.raw_stream()` はエンリッチ前の raw スケルトンを受信します。バックグラウンドタスクがソケットを管理し、25 秒ごとに ping を送り、60 秒 pong がなければ再接続し、115 分後に接続を更新し、再接続のたびに再購読し、重複を除去します。

```rust,no_run
use finlight::{Client, Config, Error, IncludeBlock, StreamEventKind, StreamOptions, StreamParams};
use futures_util::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let client = Client::new(Config::new("your-api-key"))?;
    let params = StreamParams::new()
        .query("ticker:NVDA")
        .language("en")
        .include_updates(true)
        .include([IncludeBlock::COMPANIES, IncludeBlock::EVENTS]);
    let options = StreamOptions::new()
        .on_admitted(|a| println!("admitted {:?}", a.lease_id))
        .on_reconnect(|r| println!("reconnected ({}), last seen {:?}", r.reason, r.last_seen_at))
        .on_error(|e| eprintln!("{e}")) // problems that do not end the stream
        .on_quota_wait(|q| eprintln!("quota used up, next try in {:?}", q.next_attempt_in));
    let mut stream = client.stream(params, options).await?;
    while let Some(item) = stream.next().await {
        let event = item?; // an Err is terminal; the stream then ends
        match event.kind {
            StreamEventKind::Created | StreamEventKind::Updated => {
                println!("{} {}", event.kind.as_str(), event.data.map(|a| a.title).unwrap_or_default())
            }
            _ => {} // an event this SDK version does not know; event.raw_json() has the frame
        }
    }
    Ok(())
}
```

知っておくべきこと：

- **接続数の上限。** プランの WebSocket 上限は、エンリッチストリーム、raw ストリーム、旧 SDK で共有されます。新旧のストリームを並行して動かさないでください。上限に達すると SDK は 60 秒以上待ってから再試行します。`StreamOptions::takeover(true)` を使うと、最も古い接続を置き換えます。
- **2 時間の上限。** ゲートウェイは 2 時間後に接続を切ります。SDK はその前に新しい接続を開いて再購読し、新しい接続が admitted になったら古い接続を閉じます。プランが接続数の上限に達している場合（上限 1 など）、古い接続もまだ数えられるため新しい接続は拒否されます。そのとき SDK は古い接続を閉じてすぐに接続し直すので、途切れるのは接続 1 回分ほどです。その接続も拒否された場合にだけ、通常どおり 60 秒以上待ちます。
- **初回プッシュ。** 購読のたびに、エンリッチストリームは条件に合う最新記事を 1 件送ります。古い場合があります。このストリームで配信済みなら SDK が除去します。
- **大きなフレーム。** 32,000 バイトを超えるライブフレームはサーバーが通知なしで破棄します。長い記事で `include: content` を使うと起こります。本文は `articles().get(id, ...)` で取得してください。
- `on_admitted` はサーバーが購読を検証する前に呼ばれます。購読が受理されたことを意味しません。
- SDK は欠落分を自動取得しません。`on_reconnect` の `last_seen_at` を使って REST で補完してください。
- ストリームを drop するか `stream.close().await` を呼ぶと、ソケットを 1000 で閉じ、バックグラウンド処理をすべて止めます。
- キーの拒否、WebSocket を含まないプラン、無効な購読、クローズコード 1008、`connection_limit` によるプリエンプト（`Error::Preempted`）では、ストリームは `Err` で終了します。その他の問題は `on_error` に渡され、ストリームは続きます。フック内の panic は捕捉されてログに記録されます。
- **WebSocket クォータの使い切り。** 期間のクォータを使い切った場合（ダイヤル拒否または `quota_exceeded` プリエンプト）、ストリームは終了せず、クォータが戻るまで 1 時間ごとに再試行します。待機のたびに警告を記録し、`on_quota_wait` に通知します。`Config::quota_reconnect` を `false` にするとエラーで終了します。`quota_reconnect_interval` で間隔を変えられます。REST は待ちません。月次の 429 はすぐに失敗します。
- `raw_stream` には `include` もエンリッチ用のフィルターもありません。`source_types` と `exclude_source_types` は使えます。

## Webhook

`webhooks::construct_event(raw_body, headers, secret, &options)` は署名とタイムスタンプ（既定の許容差 300 秒）を検証してから本文を読みます。`WebhookPayload::Event` を返すか、正しく署名された旧形式の本文には `WebhookPayload::WrongPayloadVersion` を返します。

```rust
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use finlight::Error;
use finlight::webhooks::{self, WebhookPayload, WebhookVerifyOptions};

// Take the body as Bytes: never verify re-serialized JSON.
async fn finlight_webhook(headers: HeaderMap, body: Bytes) -> StatusCode {
    let secret = std::env::var("FINLIGHT_WEBHOOK_SECRET").unwrap_or_default();
    match webhooks::construct_event(&body, &headers, &secret, &WebhookVerifyOptions::new()) {
        Ok(WebhookPayload::Event(event)) => {
            // Dedupe on event.event_id, answer within 5 s, do slow work afterwards.
            tokio::spawn(async move { println!("{} {}", event.event, event.data.title) });
            StatusCode::NO_CONTENT
        }
        Ok(_) => StatusCode::NO_CONTENT, // WrongPayloadVersion during the switch: keep its `raw`, answer 2xx
        Err(Error::WebhookVerification(_)) => StatusCode::BAD_REQUEST, // never 410
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
```

署名は生のバイト列に対して計算されます。JSON パーサーが動く前に取得してください：

- axum：`Json<T>` ではなく `axum::body::Bytes` と `HeaderMap` を受け取る。
- actix-web：`web::Bytes` を受け取り、`req.headers()` の値を `construct_event_from_values` に渡す。
- warp：`warp::body::bytes()` と `warp::header::headers_cloned()`。
- hyper：本文を `Bytes` に集めて `parts.headers` を渡す。

- `headers` には `http::HeaderMap`、文字列の `HashMap` や `BTreeMap`、`(name, value)` のペアを渡せます。名前は大文字小文字を区別せずに照合し、CGI 形式（`HTTP_X_WEBHOOK_TIMESTAMP`）にも対応します。
- 配信は重複することがあります。`event_id` で重複を除いてください。
- 5 秒以内に 2xx を返し、重い処理はその後で行ってください。
- 410 は絶対に返さないでください。Webhook がすぐに無効になります。
- ダッシュボードからのテスト配信では `is_test()` が true です。
- `webhooks::verify_signature(raw_body, signature, timestamp, secret, &options)` は署名だけを検証します。`construct_event_from_values` は 2 つのヘッダー値を直接受け取ります。
- SDK は Webhook の管理を行いません。作成と変更は finlight ダッシュボードで行ってください。

## エラーと再試行

```rust,no_run
use finlight::{ApiErrorKind, Client, Config, Error, RateLimitType, SearchParams};

#[tokio::main]
async fn main() {
    let client = Client::new(Config::new("your-api-key")).expect("config");
    match client.articles().search(&SearchParams::new().event_types(["earnigns"])).await {
        Ok(page) => println!("{} articles", page.articles.len()),
        Err(Error::Api(e)) if e.limit_type() == Some(&RateLimitType::Monthly) => {
            println!("quota used up for this period")
        }
        Err(Error::Api(e)) if e.api_kind() == ApiErrorKind::NotFound => println!("not found"),
        Err(Error::Api(e)) => {
            println!("{} {:?} {} {:?} {:?}", e.status(), e.code(), e.api_kind(), e.detail(), e.request_id());
            for issue in e.errors() {
                println!("{:?} {:?} {:?}", issue.field, issue.issue, issue.allowed_values);
            }
        }
        Err(other) => println!("{}: {other}", other.kind()),
    }
}
```

すべてのエラーは `finlight::Error` のいずれかのバリアントで、`kind()` が種類を文字列で返します。API エラーは `status()`、`code()`、`title()`、`detail()`、`request_id()`、`errors()`（`FieldIssue`）、`api_kind()`、`raw()`（受信した本文）を持ちます。論理的な呼び出しごとに独自の `X-Request-Id` を送り、再試行では同じ値を使います。原因は `std::error::Error::source` でたどれます。

```text
finlight::Error                       kind()
├── Error::Api(ApiError)              api (api_kind: validation_failed, cursor_invalid, cursor_mismatch,
│                                          unauthorized, forbidden, not_found, rate_limited
│                                          (limit_type, retry_after), internal_error, other)
├── Error::Validation(ValidationError)    validation (local, before any request)
├── Error::Network(NetworkError)          network
├── Error::Timeout(TimeoutError)          timeout
├── Error::Decode(DecodeError)            decode
├── Error::WebhookVerification(..)        webhook_verification (reason)
└── Error::Preempted(PreemptedError)      preempted (reason, new_lease_id)
```

再試行：

| 応答 | 再試行 |
|---|---|
| 400、401、403、404 | しない |
| 429 monthly / unknown | しない |
| 429 burst | `Retry-After` のとおりに待ってから 1 回（ヘッダーがなければ 10 秒）。既定で有効、`RetryOnBurst::Off` で無効 |
| 5xx、ネットワークエラー、タイムアウト | 最大 3 回、待ち時間を増やしながら（ジッターあり） |

呼び出しを中止するには future を drop します（例：`tokio::time::timeout` や `tokio::select!`）。待機はすぐに終わり、その後は再試行しません。`Config::total_timeout` は再試行とバーストの待機を含めた呼び出し全体を制限します。

## 設定

`Config::new(key)` は既定値を使います。`Config::builder(key)....build()?` は値を検証します。時間は `std::time::Duration` です。

| フィールド | 既定値 | 意味 |
|---|---|---|
| `api_key` | - | 必須、空不可。 |
| `base_url` | `https://api.finlight.me` | REST のベース URL。 |
| `wss_url` | `wss://wss.finlight.me` | WebSocket URL。raw は `<wss_url>/raw`。 |
| `timeout` | 30 秒 | HTTP 1 回あたりのタイムアウト。 |
| `total_timeout` | なし | 再試行と待機を含む呼び出し全体の上限（任意）。 |
| `max_retries` | 3 | 5xx、ネットワークエラー、タイムアウトの再試行回数。 |
| `retry_base_delay` / `retry_max_delay` | 0.5 秒 / 8 秒 | 再試行間のバックオフ（ジッターあり）。 |
| `retry_on_burst` | `RetryOnBurst::Once` | 429 burst への対応（`Once` または `Off`）。 |
| `burst_wait` | 10 秒 | 429 に `Retry-After` がないときの待ち時間。 |
| `max_pages` | 10 | `search_all` のページ上限。 |
| `max_response_bytes` | 32 MiB | REST 応答本文の上限。 |
| `company_cache_size` | 256 | `companies().get()` の ETag キャッシュの件数。0 で無効。 |
| `ping_interval` / `watchdog_timeout` / `rotation_interval` | 25 秒 / 60 秒 / 115 分 | ストリームの ping、pong ウォッチドッグ、接続更新。 |
| `reconnect_base_delay` / `reconnect_max_delay` | 1 秒 / 30 秒 | ストリーム再接続のバックオフ。 |
| `long_backoff` | 60 秒 | 「接続数超過」と 429 によるダイヤル拒否後の待ち時間。 |
| `quota_reconnect` / `quota_reconnect_interval` | `true` / 1 時間 | ストリームのみ：WebSocket クォータを使い切ったら 1 時間ごとに再試行（`false` で終了）。REST は新しいクォータを待ちません。 |
| `search_failed_threshold` | 5 | `SEARCH_FAILED` がこの回数続くと `on_error` に通知。 |
| `dedupe_cache_size` | 1000 | ストリームごとの重複フィルターの件数。 |
| `max_message_bytes` | 32 MiB（最小 16 MiB） | 受信するストリームメッセージの上限。 |
| `connect_timeout` | 30 秒 | WebSocket のダイヤルとハンドシェイクのタイムアウト。 |
| `http_client` | - | REST 用の独自 `reqwest::Client`（プロキシなど）。その場合は独自のタイムアウトが適用され `timeout` は使われません。独自の再試行は無効にしてください。SDK はこのクライアントを閉じません。 |
| `ws_connector` | - | ストリーム接続用のバイトストリームを開く独自の `WsConnector`（プロキシトンネルなど）。TLS（OS の証明書ストア）とハンドシェイクは SDK が行います。 |

テストでは tokio の一時停止クロック（`tokio` の `test-util` 機能）を使います。専用のクロック設定はありません。

## ログ

SDK は `tracing` でターゲット `finlight` にログを出します。サブスクライバーがなければ何も出力しません。stdout や stderr に直接書くことはなく、グローバルなハンドラーも登録しません。再試行、再接続、クォータ待機は警告、メッセージ単位の詳細は debug です。API キー、Webhook シークレット、署名、本文全体はログに出さず、`Config`、`Client`、エラーの `Debug` 出力にも含めません。WebSocket プロトコルは SDK 自身が実装しているため、どのレベルでも通信ライブラリがストリームのフレーム内容をログに出すことはありません。

## データについて

- 一部の古い記事は `published_at` が `1970-01-01T00:00:00.000Z` です。SDK はそのまま返します。本文がない場合、`summary` と `content` は `None` / `Field::Null` になります。
- ロール導入前にタグ付けされた記事では、`companies[].role` と `relevance` は `None`、`events` は `Some([])` です。
- ID 導入前にインデックスされた記事は `articles().get()` で 404 になります。`get_by_link` なら見つかります。
- `available_at` は raw では常に `None`、導入前の記事でも `None` です。
- `source.type` は、ソースが未分類の場合や、ソースの種類が導入される前にインデックスされた記事では `None` です。再分類の後は `sources().list()` の `type` より古い値のことがあります。

## 0.1 からの移行

詳しいガイドは [docs.finlight.me](https://docs.finlight.me) にあります。主な変更点：

| 0.1（API v2） | 0.2（API v3） |
|---|---|
| `use finlight_client::...` | `use finlight::...`（crate 名は引き続き `finlight-client`） |
| `client.articles.fetch_articles(&GetArticlesParams {..})` | `client.articles().search(&SearchParams::new()...)`、`search_all(...)` |
| `client.articles.fetch_article_by_link(&GetArticleByLinkParams {..})` | `client.articles().get_by_link(link, &options)`、`get(id, &options)` |
| `client.sources.get_sources()` | `client.sources().list()` |
| `client.websocket.stream(GetArticlesWebSocketParams {..})` | `client.stream(StreamParams::new()..., StreamOptions::new()).await?` |
| `client.raw_websocket.stream(..)` | `client.raw_stream(RawStreamParams::new()..., options).await?` |
| `WebSocketClient::new(config, WebSocketOptions {..})` | ストリーム設定は `Config`（`ping_interval`、`watchdog_timeout` など）、フックは `StreamOptions` |
| `construct_webhook_event(body, signature, secret, timestamp)` | `webhooks::construct_event(body, &headers, secret, &options)` または `construct_event_from_values(...)` |
| `Error::Api { status, body }`、`Error::Blocked` | `status()`、`code()`、`api_kind()` を持つ `Error::Api(ApiError)`、`Error::Preempted` |
| `publish_date: DateTime<Utc>` | `published_at: Option<String>` + `published_at_time()` |
| `created_at` | `first_seen_at` |
| `revised_date` | `revised_at` |
| `images: Vec<String>` | `media: Vec<Media>` |
| `source: String` | `source.domain` |
| `sentiment`、`confidence` | `sentiment: Field<Sentiment>`（`label`、`confidence`） |
| `include_content`、`include_entities` | `include([IncludeBlock::CONTENT, IncludeBlock::COMPANIES, IncludeBlock::EVENTS])` |
| `page`、`page_size` | `limit`、`cursor`（ページ送りは `search_all`） |
| `language` | `languages(["en"])` |
| `is_update` | `StreamEventKind::Updated` |
| Webhook 本文 = 記事 | `event.data` + `event.event_id` |

Webhook を v3 に切り替える 4 つの手順：

1. 新しい SDK をデプロイする。
2. `WrongPayloadVersion` に 2xx で応答し、必要なら `raw` の JSON を保存する。
3. finlight ダッシュボードで Webhook を `payloadVersion` 3 に切り替える。
4. 約 2 時間待つ。キューに入っている再送は旧形式のまま届きます。

## 開発

```bash
cargo test                                   # unit, rule and conformance tests; no network, no keys
cargo test --test conformance -- S-18        # one kit case
scripts/sync-conformance.sh <monorepo>/shared/contracts/api-v3
FINLIGHT_API_KEY=... FINLIGHT_BASE_URL=https://api.dev.finlight.me \
  FINLIGHT_WSS_URL=wss://wss.dev.finlight.me cargo test --test live -- --ignored
```

コントラクトキットは `conformance/`（`CONTRACT_VERSION`、sha256 の `MANIFEST`）にあり、公開パッケージには含まれません。キットの各ケースは `tests/conformance.rs` で個別のテストとして実行されます。`CONFORMANCE.md` がケースとテストを対応付けます。コピーが MANIFEST と一致しない場合や、テストのないケースがある場合はテストが失敗します。ライブテストは dev のみに対して実行し、3 つの変数すべてが必要です。本番ホストは拒否されます。

## ライセンス

MIT
