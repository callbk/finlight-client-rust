# finlight Rust 客户端

*[English](README.md) | 简体中文 | [日本語](README.ja.md) | [한국어](README.ko.md)*

> **Breaking release.** 0.2 只支持 finlight API v3（REST `/v3`、WebSocket `payloadVersion` 3、v3 Webhook），不能运行 0.1 代码。请先阅读[从 0.1 迁移](#从-01-迁移)和[更新日志](CHANGELOG.md)。如需继续使用 API v2，请保留 `finlight-client = "0.1"`。

*本文译自英文 README，会随英文版后续更新。如有出入，以英文版为准。*

[finlight.me](https://finlight.me) 的异步客户端：搜索金融新闻，查询文章、公司和来源，通过 WebSocket 接收实时文章，并验证 Webhook。crate 名为 `finlight-client`，库以 `finlight` 导入。

## 版本

| SDK version | API version | Status |
|---|---|---|
| 0.2.x | v3（`/v3`、WebSocket `payloadVersion` 3、Webhook v3） | 当前 |
| 0.1.x | v2 | 不再发布 |

SDK 版本与 API 版本相互独立：新的 API 字段和枚举值通过 0.2.x 补丁版本提供。支持 API v3 的 SDK：TypeScript 4.x、Python 3.x、PHP 2.x、Go `/v2`、.NET 2.x、Java 1.x、Ruby 1.x、Rust 0.2。

Cargo 不会把 `"0.1"` 升级到 0.2。`"*"` 这样的通配符会，所以请固定次版本号。

## 环境要求

- Rust 1.85 或更高（edition 2024）
- tokio 1.47 或更高（每个流会启动一个任务，需要 tokio 运行时）
- reqwest 0.13（rustls）、serde 1、serde_json 1.0.142 或更高；WebSocket 协议由 SDK 自带实现
- TLS 使用 rustls（aws-lc-rs 提供程序）；REST 和流都使用操作系统的信任存储（rustls-platform-verifier）验证证书，因此安装在系统中的企业 CA 对两者都有效；不需要 OpenSSL

## 安装

```bash
cargo add finlight-client@0.2 tokio --features tokio/full
cargo add futures-util   # for StreamExt on streams
```

## 快速开始

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

SDK 只发送你设置的参数，不会添加默认值：`limit`（20）、`languages`（`["en"]`）和时间范围由服务器决定。参数名是结构体字段，拼错会无法编译；`SearchParams::from_json` 会在发出请求前以 `Error::Validation` 拒绝未知的键。

## 搜索文章

`articles().search()` 返回一页。`articles().search_all()` 会替你遍历游标。

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

- `search_all` 在调用时冻结请求体。后续页面只改变 `cursor`，因此游标始终有效。
- 在 `has_more: false`、`next_cursor: None` 或达到 `max_pages`（默认 10）时停止。达到上限时会记录警告，`truncated()` 为 true，可用 `next_cursor()` 继续。
- 每一页都是一次计费调用。结果只能遍历一次：按文章（`next()` 或作为 `futures` 的 `Stream`）或按页（`next_page()`），另一种方式会返回 `Error::Validation`。
- `from` 和 `to` 接受 `SystemTime`（以带毫秒的 UTC 发送）、`DateInput::date(y, m, d)`（以 `YYYY-MM-DD` 发送）或字符串（原样发送），作用于 `order_by` 指定的时间戳。
- `source_types` 和 `exclude_source_types` 按来源类型筛选：`SourceType::NEWSWIRE`、`PRESS_RELEASE`、`FINANCIAL_MEDIA`、`GENERAL_MEDIA`、`AGGREGATOR`、`BLOG`、`GOVERNMENT`。未分类来源的文章不会匹配 `source_types`，在 `exclude_source_types` 下会保留。两者只在来源池内筛选，不会添加来源。`StreamParams` 和 `RawStreamParams` 也支持这两个参数。
- 无法解码的条目会被跳过、记录日志并列在 `page.skipped` 中，页面其余部分仍可使用。

## 查询文章、公司和来源

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

- 404 是 `api_kind() == ApiErrorKind::NotFound` 的 `Error::Api`，而不是空结果。
- 在引入 ID 之前建立索引的文章无法按 ID 查询，请使用 `get_by_link`。
- 公司接口需要 `entityAccess` 权限，否则返回 403（`ApiErrorKind::Forbidden`）。
- `companies().get()` 会保存 ETag 并重新验证；收到 304 时返回已保存的记录，且 `cache_info().not_modified == true`。使用自己的 ETag 时，`get_if_none_match` 在 304 时返回 `Ok(None)`。
- 查询接口和文章调用一样消耗 credit：`companies()`（`search`、`get_many`、`get`、`get_if_none_match`）和 `sources().list()` 每次请求消耗 1 个 credit，包含多个 ID 的 `get_many` 也只算 1 个。304 重新验证和错误也消耗 1 个 credit。
- `CompanyList` 和 `SourceList` 可当作切片使用，无法解码的条目在 `skipped()` 中。

## 缺失、null 与空

部分键取决于你的套餐和 `include`。键缺失、`null` 和 `[]` 含义不同，SDK 会区分它们。

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

- 键缺失：未请求、不在套餐内，或 raw 通道。
- `null`：已请求且有权限，但没有值（例如 `content` 没有正文）。
- `[]`：真正的空列表（例如已打标签，但没有找到公司）。
- `content` 和 `sentiment` 是 `Field<T>`（`Absent`、`Null`、`Value`）。`companies`、`events`、`events[].company_ids` 和 `Source::has_content` 是 `Option<_>`：`None` 表示缺失，而不是“空”。
- `content_present()`、`sentiment_present()`、`companies_present()` 和 `events_present()` 表示该键是否出现在 JSON 中（包括 `null`）。
- 来自新版服务器的未知键会被忽略；`raw_json()` 返回收到的原始 JSON。用 serde 写回模型时，缺失的键仍然缺失。
- 词表是开放的：SDK 不认识的值保存在 `CompanyRole::Other("...")` 中。请与常量比较，例如 `role == CompanyRole::ACQUIRER`。

## 实时流

`client.stream()` 接收增强后的文章，`client.raw_stream()` 接收增强前的原始骨架。后台任务管理套接字：每 25 秒发送 ping，60 秒内没有 pong 就重连，115 分钟后更换连接，每次重连后重新订阅，并去除重复。

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

需要了解：

- **连接数上限。** 套餐的 WebSocket 上限由增强流、原始流和旧版 SDK 共享。不要同时运行新旧流。达到上限时，SDK 至少等待 60 秒再重试；`StreamOptions::takeover(true)` 会替换你最旧的连接。
- **两小时上限。** 网关会在 2 小时后断开每个连接。SDK 会提前打开新连接并重新订阅，新连接 admitted 后关闭旧连接。如果你的套餐已达到连接上限（例如上限为 1），旧连接仍被计入，新连接会被拒绝；SDK 随即关闭旧连接并立即重新连接，中断时间大约只有一次连接的长度。只有这次连接也被拒绝时，才会按常规至少等待 60 秒。
- **首次推送。** 每次订阅后，增强流会发送一篇最新的匹配文章，它可能是旧的。如果该文章已在本流中送达，SDK 会丢弃它。
- **大帧。** 超过 32,000 字节的实时帧会被服务器静默丢弃。对长文章使用 `include: content` 时可能发生。请改用 `articles().get(id, ...)` 获取正文。
- `on_admitted` 在服务器检查订阅之前触发，并不表示订阅已被接受。
- SDK 不会自动补齐缺失数据。`on_reconnect` 提供 `last_seen_at`，你可以通过 REST 补齐。
- 丢弃流或调用 `stream.close().await` 会以 1000 关闭套接字并停止所有后台工作。
- 密钥被拒、套餐不含 WebSocket、订阅无效、关闭码 1008 以及 `connection_limit` 抢占（`Error::Preempted`）时，流以 `Err` 结束。其他问题交给 `on_error`，流继续运行。钩子中的 panic 会被捕获并记录。
- **WebSocket 配额用尽。** 当本周期配额用尽（拨号被拒或 `quota_exceeded` 抢占）时，流不会结束，而是每小时重试一次，直到配额允许。每次等待都会记录警告并通知 `on_quota_wait`。把 `Config::quota_reconnect` 设为 `false` 则以错误结束；`quota_reconnect_interval` 可修改间隔。REST 调用从不等待：月度 429 会立即失败。
- `raw_stream` 没有 `include`，也没有增强过滤条件，但支持 `source_types` 和 `exclude_source_types`。

## Webhook

`webhooks::construct_event(raw_body, headers, secret, &options)` 先校验签名和时间戳（默认容差 300 秒），再读取正文。它返回 `WebhookPayload::Event`；对于签名正确的旧格式正文，返回 `WebhookPayload::WrongPayloadVersion`。

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

签名覆盖原始字节。请在任何 JSON 解析器运行之前获取它们：

- axum：接收 `axum::body::Bytes`（不要用 `Json<T>`）和 `HeaderMap`。
- actix-web：接收 `web::Bytes`，并把 `req.headers()` 的值传给 `construct_event_from_values`。
- warp：`warp::body::bytes()` 搭配 `warp::header::headers_cloned()`。
- hyper：把正文收集为 `Bytes`，并传入 `parts.headers`。

- `headers` 接受 `http::HeaderMap`、字符串的 `HashMap` 或 `BTreeMap`，以及 `(name, value)` 键值对。名称不区分大小写，也支持 CGI 形式（`HTTP_X_WEBHOOK_TIMESTAMP`）。
- 投递可能重复，请按 `event_id` 去重。
- 在 5 秒内返回 2xx，耗时的工作放到之后。
- 不要返回 410：它会立即停用 Webhook。
- 来自控制台的测试投递 `is_test()` 为 true。
- `webhooks::verify_signature(raw_body, signature, timestamp, secret, &options)` 只校验签名；`construct_event_from_values` 直接接收两个请求头的值。
- SDK 不管理 Webhook。请在 finlight 控制台中创建和修改。

## 错误与重试

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

每个错误都是 `finlight::Error` 的一个变体，`kind()` 以文本给出类型。API 错误带有 `status()`、`code()`、`title()`、`detail()`、`request_id()`、`errors()`（`FieldIssue`）、`api_kind()` 和 `raw()`（收到的正文）。每次逻辑调用发送自己的 `X-Request-Id`，重试时沿用。原因可通过 `std::error::Error::source` 获取。

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

重试：

| 响应 | 重试 |
|---|---|
| 400、401、403、404 | 从不 |
| 429 monthly / unknown | 从不 |
| 429 burst | 按 `Retry-After` 等待后重试一次（缺少该头时为 10 秒）；默认开启，`RetryOnBurst::Off` 关闭 |
| 5xx、网络错误、超时 | 最多 3 次，等待时间递增并带抖动 |

通过丢弃 future 来取消调用，例如使用 `tokio::time::timeout` 或 `tokio::select!`。等待会立即结束，之后不再重试。`Config::total_timeout` 限制整个调用，包括重试和 burst 等待。

## 配置

`Config::new(key)` 使用默认值；`Config::builder(key)....build()?` 会校验配置。时间使用 `std::time::Duration`。

| 字段 | 默认值 | 含义 |
|---|---|---|
| `api_key` | - | 必填，不能为空。 |
| `base_url` | `https://api.finlight.me` | REST 基础 URL。 |
| `wss_url` | `wss://wss.finlight.me` | WebSocket URL；raw 使用 `<wss_url>/raw`。 |
| `timeout` | 30 秒 | 单次 HTTP 尝试的超时。 |
| `total_timeout` | 无 | 可选：整个调用（含重试和等待）的上限。 |
| `max_retries` | 3 | 5xx、网络错误和超时的重试次数。 |
| `retry_base_delay` / `retry_max_delay` | 0.5 秒 / 8 秒 | 重试间的退避（带抖动）。 |
| `retry_on_burst` | `RetryOnBurst::Once` | 429 burst 的处理方式（`Once` 或 `Off`）。 |
| `burst_wait` | 10 秒 | 429 没有 `Retry-After` 时的等待时间。 |
| `max_pages` | 10 | `search_all` 的页数上限。 |
| `max_response_bytes` | 32 MiB | REST 响应正文上限。 |
| `company_cache_size` | 256 | `companies().get()` 的 ETag 缓存条数；0 表示关闭。 |
| `ping_interval` / `watchdog_timeout` / `rotation_interval` | 25 秒 / 60 秒 / 115 分钟 | 流的 ping、pong 看门狗和连接更换。 |
| `reconnect_base_delay` / `reconnect_max_delay` | 1 秒 / 30 秒 | 流重连退避。 |
| `long_backoff` | 60 秒 | “连接过多”和 429 拨号被拒后的等待时间。 |
| `quota_reconnect` / `quota_reconnect_interval` | `true` / 1 小时 | 仅限流：WebSocket 配额用尽时每小时重试（`false` 则结束流）。REST 从不等待新配额。 |
| `search_failed_threshold` | 5 | 连续出现这么多次 `SEARCH_FAILED` 后通知 `on_error`。 |
| `dedupe_cache_size` | 1000 | 每个流的去重过滤器条数。 |
| `max_message_bytes` | 32 MiB（最少 16 MiB） | 接收的流消息上限。 |
| `connect_timeout` | 30 秒 | WebSocket 拨号和握手超时。 |
| `http_client` | - | 用于 REST 的自定义 `reqwest::Client`（例如代理）。此时使用它自己的超时，不使用 `timeout`；请关闭它自己的重试。SDK 不会关闭它。 |
| `ws_connector` | - | 为流连接打开字节流的自定义 `WsConnector`（例如代理隧道）；TLS（操作系统信任存储）和握手由 SDK 完成。 |

测试使用 tokio 的暂停时钟（`tokio` 的 `test-util` 特性），没有单独的时钟选项。

## 日志

SDK 通过 `tracing` 以目标 `finlight` 记录日志。没有订阅者时不输出任何内容；它从不直接写 stdout 或 stderr，也不安装全局处理器。重试、重连和配额等待记为警告；逐条消息的细节为 debug。API 密钥、Webhook 密钥、签名和完整正文绝不会出现在日志中，`Config`、`Client` 和错误的 `Debug` 输出也不包含它们。SDK 自己实现 WebSocket 协议，因此没有任何传输库会在任何级别记录流的帧内容。

## 数据说明

- 部分旧文章的 `published_at` 为 `1970-01-01T00:00:00.000Z`，SDK 原样提供；没有文本时，`summary` 和 `content` 为 `None` / `Field::Null`。
- 在角色功能之前打标签的文章中，`companies[].role` 和 `relevance` 为 `None`，`events` 为 `Some([])`。
- 在引入 ID 之前建立索引的文章，`articles().get()` 返回 404；`get_by_link` 可以找到它们。
- `available_at` 在 raw 中始终为 `None`，在其上线前的文章中也为 `None`。
- 来源未分类，或文章在来源类型上线之前被索引时，`source.type` 为 `None`。重新分类后，它可能落后于 `sources().list()` 中的 `type`。

## 从 0.1 迁移

完整指南见 [docs.finlight.me](https://docs.finlight.me)。主要变化：

| 0.1（API v2） | 0.2（API v3） |
|---|---|
| `use finlight_client::...` | `use finlight::...`（crate 仍为 `finlight-client`） |
| `client.articles.fetch_articles(&GetArticlesParams {..})` | `client.articles().search(&SearchParams::new()...)`、`search_all(...)` |
| `client.articles.fetch_article_by_link(&GetArticleByLinkParams {..})` | `client.articles().get_by_link(link, &options)`、`get(id, &options)` |
| `client.sources.get_sources()` | `client.sources().list()` |
| `client.websocket.stream(GetArticlesWebSocketParams {..})` | `client.stream(StreamParams::new()..., StreamOptions::new()).await?` |
| `client.raw_websocket.stream(..)` | `client.raw_stream(RawStreamParams::new()..., options).await?` |
| `WebSocketClient::new(config, WebSocketOptions {..})` | 流设置在 `Config` 中（`ping_interval`、`watchdog_timeout` 等），钩子在 `StreamOptions` 中 |
| `construct_webhook_event(body, signature, secret, timestamp)` | `webhooks::construct_event(body, &headers, secret, &options)` 或 `construct_event_from_values(...)` |
| `Error::Api { status, body }`、`Error::Blocked` | 带有 `status()`、`code()`、`api_kind()` 的 `Error::Api(ApiError)`；`Error::Preempted` |
| `publish_date: DateTime<Utc>` | `published_at: Option<String>` + `published_at_time()` |
| `created_at` | `first_seen_at` |
| `revised_date` | `revised_at` |
| `images: Vec<String>` | `media: Vec<Media>` |
| `source: String` | `source.domain` |
| `sentiment`、`confidence` | `sentiment: Field<Sentiment>`（`label`、`confidence`） |
| `include_content`、`include_entities` | `include([IncludeBlock::CONTENT, IncludeBlock::COMPANIES, IncludeBlock::EVENTS])` |
| `page`、`page_size` | `limit`、`cursor`（由 `search_all` 翻页） |
| `language` | `languages(["en"])` |
| `is_update` | `StreamEventKind::Updated` |
| Webhook 正文 = 文章 | `event.data` + `event.event_id` |

将 Webhook 切换到 v3 的四个步骤：

1. 部署新版 SDK。
2. 对 `WrongPayloadVersion` 返回 2xx，如需数据请保存其 `raw` JSON。
3. 在 finlight 控制台中把 Webhook 切换到 `payloadVersion` 3。
4. 等待约 2 小时。已排队的重试仍以旧格式到达。

## 开发

```bash
cargo test                                   # unit, rule and conformance tests; no network, no keys
cargo test --test conformance -- S-18        # one kit case
scripts/sync-conformance.sh <monorepo>/shared/contracts/api-v3
FINLIGHT_API_KEY=... FINLIGHT_BASE_URL=https://api.dev.finlight.me \
  FINLIGHT_WSS_URL=wss://wss.dev.finlight.me cargo test --test live -- --ignored
```

契约套件位于 `conformance/`（`CONTRACT_VERSION`、sha256 `MANIFEST`），不会发布。套件中的每个用例在 `tests/conformance.rs` 中作为独立测试运行；`CONFORMANCE.md` 将用例映射到测试。当副本与 MANIFEST 不一致或某个用例没有测试时，测试会失败。实时测试只针对 dev 运行，需要全部三个变量；生产主机会被拒绝。

## 许可证

MIT
