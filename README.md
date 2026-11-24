# finlight client for Rust

*English | [简体中文](README.zh-CN.md) | [日本語](README.ja.md) | [한국어](README.ko.md)*

> **Breaking release.** 0.2 speaks finlight API v3 only (REST `/v3`, WebSocket `payloadVersion` 3, v3 webhooks) and does not run 0.1 code. Read [Migrating from 0.1](#migrating-from-01) and the [changelog](CHANGELOG.md) first. To stay on API v2, keep `finlight-client = "0.1"`.

Async client for [finlight.me](https://finlight.me): search financial news, look up articles, companies and sources, follow live articles over WebSocket, and verify webhooks. The crate is `finlight-client`; the library is imported as `finlight`.

## Versions

| SDK version | API version | Status |
|---|---|---|
| 0.2.x | v3 (`/v3`, WebSocket `payloadVersion` 3, webhooks v3) | current |
| 0.1.x | v2 | no further releases |

SDK and API versions are independent: new API fields and enum values arrive as 0.2.x patch releases. These SDKs speak API v3: TypeScript 4.x, Python 3.x, PHP 2.x, Go `/v2`, .NET 2.x, Java 1.x, Ruby 1.x, Rust 0.2.

Cargo never moves `"0.1"` to 0.2. A wildcard like `"*"` does, so pin the minor version.

## Requirements

- Rust 1.85 or newer (edition 2024)
- tokio 1.47 or newer (the SDK spawns one task per stream and needs a tokio runtime)
- reqwest 0.13 (rustls), serde 1, serde_json 1.0.142 or newer; the WebSocket protocol is built in
- TLS through rustls with the aws-lc-rs provider; REST and streams both verify against the operating system's trust store (rustls-platform-verifier), so a corporate CA installed there works for both; no OpenSSL

## Installation

```bash
cargo add finlight-client@0.2 tokio --features tokio/full
cargo add futures-util   # for StreamExt on streams
```

## Quick start

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

The SDK sends only the parameters you set. It adds no defaults: the server decides `limit` (20), `languages` (`["en"]`) and the time range. Parameter names are struct fields, so a misspelled name does not compile; `SearchParams::from_json` rejects unknown keys with `Error::Validation` before any request.

## Search articles

`articles().search()` returns one page. `articles().search_all()` walks the cursor for you.

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

- `search_all` freezes the request body at the call. Later pages only change `cursor`, so the cursor stays valid.
- It stops at `has_more: false`, at `next_cursor: None` or after `max_pages` (default 10). At the limit it logs a warning, `truncated()` is true and `next_cursor()` lets you continue.
- Every page is a metered call. Iterate a result once: either articles (`next()`, or as a `futures` `Stream`) or pages (`next_page()`); the other way then returns `Error::Validation`.
- `from` and `to` take a `SystemTime` (sent as UTC with milliseconds), `DateInput::date(y, m, d)` (sent as `YYYY-MM-DD`) or a string (sent as is). They apply to the `order_by` timestamp.
- `source_types` and `exclude_source_types` filter by source type: `SourceType::NEWSWIRE`, `PRESS_RELEASE`, `FINANCIAL_MEDIA`, `GENERAL_MEDIA`, `AGGREGATOR`, `BLOG`, `GOVERNMENT`. Articles of unclassified sources never match `source_types` and are kept by `exclude_source_types`. Both filter inside the source pool and never add a source. `StreamParams` and `RawStreamParams` take them too.
- Entries that cannot be decoded are skipped, logged and listed in `page.skipped`. The rest of the page stays usable.

## Look up articles, companies and sources

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

- A 404 is `Error::Api` with `api_kind() == ApiErrorKind::NotFound`, not an empty result.
- Articles indexed before the id rollout have no id lookup. Use `get_by_link` for them.
- Company endpoints need the `entityAccess` entitlement, otherwise they answer 403 (`ApiErrorKind::Forbidden`).
- `companies().get()` keeps the ETag and revalidates; on 304 you get the stored record with `cache_info().not_modified == true`. With your own ETag, `get_if_none_match` returns `Ok(None)` on 304.
- `CompanyList` and `SourceList` dereference to slices and have `skipped()` for entries that could not be decoded.

## Absent, null and empty

Some keys depend on your plan and on `include`. A missing key, `null` and `[]` mean different things, and the SDK keeps them apart.

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

- Missing key: not requested, not in your plan, or the raw channel.
- `null`: requested and entitled, but there is no value (for example no text for `content`).
- `[]`: a real empty list (for example tagged, but no company found).
- `content` and `sentiment` are `Field<T>` (`Absent`, `Null`, `Value`). `companies`, `events`, `events[].company_ids` and `Source::has_content` are `Option<_>`: `None` means absent, never "empty".
- `content_present()`, `sentiment_present()`, `companies_present()` and `events_present()` tell whether the key was in the JSON (`null` included).
- Unknown keys from newer servers are ignored; `raw_json()` returns the JSON as received. Writing a model back with serde keeps absent keys absent.
- Vocabularies are open: `CompanyRole::Other("...")` keeps a value this SDK does not know. Compare with the constants, e.g. `role == CompanyRole::ACQUIRER`.

## Real-time streams

`client.stream()` follows enriched articles, `client.raw_stream()` the raw skeleton before enrichment. A background task owns the socket: it pings every 25 s, reconnects when no pong arrives for 60 s, renews the connection after 115 minutes, re-subscribes after every reconnect and drops duplicates.

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

Things to know:

- **Connection limit.** Your plan's WebSocket limit is shared by enriched streams, raw streams and older SDK versions. Do not run old and new streams side by side. When the limit is reached the SDK waits at least 60 s and tries again; `StreamOptions::takeover(true)` replaces your stalest connection instead.
- **Two hour limit.** The gateway ends every connection after 2 h. The SDK opens a new connection before that, re-subscribes, and closes the old one once the new one is admitted. When your plan is at its connection limit (for example a limit of 1), the old connection still counts and the new dial is refused; the SDK then closes the old connection and dials again at once, so the gap is about one dial long. Only if that dial is refused too does the usual wait of at least 60 s apply.
- **Initial push.** After each subscribe the enriched stream sends the latest matching article. It can be old. The SDK drops it when it was already delivered on this stream.
- **Large frames.** Live frames over 32,000 bytes are dropped by the server without notice. This can happen with `include: content` on long articles. Fetch the text with `articles().get(id, ...)` instead.
- `on_admitted` fires before the server checks your subscribe. It does not mean the subscribe was accepted.
- The SDK does not backfill. `on_reconnect` gives you `last_seen_at` so you can fill gaps through REST.
- Dropping the stream or `stream.close().await` closes the socket with 1000 and stops all background work.
- A stream ends with an `Err` for a refused key, a plan without WebSockets, an invalid subscribe, close code 1008 and a `connection_limit` preemption (`Error::Preempted`). Other problems go to `on_error` and the stream keeps running. Panics in hooks are caught and logged.
- **Used-up WebSocket quota.** When the quota of the period is used up (refused dial or `quota_exceeded` preemption), the stream does not end: it tries again every hour until the quota allows it. Each wait is logged as a warning and reported to `on_quota_wait`. Set `Config::quota_reconnect` to `false` to end the stream with the error instead; `quota_reconnect_interval` changes the interval. REST calls never wait: a monthly 429 fails at once.
- `raw_stream` has no `include` and no enrichment filters. It does take `source_types` and `exclude_source_types`.

## Webhooks

`webhooks::construct_event(raw_body, headers, secret, &options)` checks the signature and the timestamp (300 s tolerance by default), then reads the body. It returns `WebhookPayload::Event`, or `WebhookPayload::WrongPayloadVersion` for a correctly signed body in the old format.

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

The signature covers the raw bytes. Get them before any JSON parser runs:

- axum: take `axum::body::Bytes` (not `Json<T>`) and `HeaderMap`.
- actix-web: take `web::Bytes` and pass `req.headers()` values to `construct_event_from_values`.
- warp: `warp::body::bytes()` with `warp::header::headers_cloned()`.
- hyper: collect the body into `Bytes` and pass `parts.headers`.

- `headers` takes an `http::HeaderMap`, a `HashMap` or `BTreeMap` of strings, or `(name, value)` pairs. Names match case-insensitively, CGI names (`HTTP_X_WEBHOOK_TIMESTAMP`) included.
- Deliveries can repeat. Dedupe on `event_id`.
- Answer with 2xx within 5 s and do slow work afterwards.
- Never answer with 410: it disables the webhook at once.
- `is_test()` is true for test deliveries from the dashboard.
- `webhooks::verify_signature(raw_body, signature, timestamp, secret, &options)` checks only the signature; `construct_event_from_values` takes the two header values directly.
- The SDK does not manage webhooks. Create and change them in the finlight dashboard.

## Errors and retries

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

Every error is one `finlight::Error` variant; `kind()` names it as text. API errors carry `status()`, `code()`, `title()`, `detail()`, `request_id()`, `errors()` (`FieldIssue`), `api_kind()` and `raw()` (the body as received). Each logical call sends its own `X-Request-Id`; retries reuse it. Causes stay reachable through `std::error::Error::source`.

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

Retries:

| Answer | Retry |
|---|---|
| 400, 401, 403, 404 | never |
| 429 monthly / unknown | never |
| 429 burst | once, after exactly `Retry-After` (10 s when the header is missing); on by default, `RetryOnBurst::Off` turns it off |
| 5xx, network errors, timeouts | up to 3 times with growing, jittered waits |

Cancel a call by dropping its future, for example with `tokio::time::timeout` or `tokio::select!`. Waits end at once and nothing is retried after that. `Config::total_timeout` bounds a whole call, retries and the burst wait included.

## Configuration

`Config::new(key)` gives the defaults; `Config::builder(key)....build()?` checks them. Durations are `std::time::Duration`.

| Field | Default | Meaning |
|---|---|---|
| `api_key` | - | Required, not empty. |
| `base_url` | `https://api.finlight.me` | REST base URL. |
| `wss_url` | `wss://wss.finlight.me` | WebSocket URL; raw uses `<wss_url>/raw`. |
| `timeout` | 30 s | Timeout of one HTTP attempt. |
| `total_timeout` | none | Optional limit for a whole call, retries and waits included. |
| `max_retries` | 3 | Retries for 5xx, network errors and timeouts. |
| `retry_base_delay` / `retry_max_delay` | 0.5 s / 8 s | Backoff between retries (with jitter). |
| `retry_on_burst` | `RetryOnBurst::Once` | `Once` or `Off` for 429 burst answers. |
| `burst_wait` | 10 s | Wait before the burst retry when the 429 has no `Retry-After`. |
| `max_pages` | 10 | Page limit of `search_all`. |
| `max_response_bytes` | 32 MiB | Largest REST response body. |
| `company_cache_size` | 256 | Records in the ETag cache of `companies().get()`; 0 turns it off. |
| `ping_interval` / `watchdog_timeout` / `rotation_interval` | 25 s / 60 s / 115 min | Stream ping, pong watchdog and rotation. |
| `reconnect_base_delay` / `reconnect_max_delay` | 1 s / 30 s | Stream reconnect backoff. |
| `long_backoff` | 60 s | Wait after "too many connections" and 429 dial refusals. |
| `quota_reconnect` / `quota_reconnect_interval` | `true` / 1 h | Streams only: when the WebSocket quota is used up, retry every hour (`false` ends the stream). REST never waits for a new quota. |
| `search_failed_threshold` | 5 | After this many `SEARCH_FAILED` in a row an error goes to `on_error`. |
| `dedupe_cache_size` | 1000 | Entries of the duplicate filter per stream. |
| `max_message_bytes` | 32 MiB (min. 16 MiB) | Largest incoming stream message. |
| `connect_timeout` | 30 s | WebSocket dial and handshake timeout. |
| `http_client` | - | Your own `reqwest::Client` for REST, e.g. for a proxy. Its own timeouts then apply and `timeout` is not used; switch off its own retries. The SDK never closes it. |
| `ws_connector` | - | Your own `WsConnector` that opens the byte stream for stream dials (e.g. a proxy tunnel); the SDK adds TLS (operating system trust store) and the handshake. |

Tests use tokio's paused clock (`tokio` feature `test-util`); there is no separate clock option.

## Logging

The SDK logs through `tracing` with the target `finlight`. Without a subscriber nothing is printed; it never writes to stdout or stderr itself and installs no global handlers. Retries, reconnects and quota waits are warnings; per-message details are debug. The API key, the webhook secret, signatures and full bodies are never logged, and `Debug` output of `Config`, `Client` and errors leaves them out. The SDK speaks the WebSocket protocol itself, so no transport library logs stream frames, at any level.

## Data notes

- Some old articles have `published_at` `1970-01-01T00:00:00.000Z`. The SDK delivers it as it is; `summary` and `content` arrive as `None`/`Field::Null` when there is no text.
- On articles tagged before roles existed, `companies[].role` and `relevance` are `None`, and `events` is `Some([])`.
- Articles indexed before the id rollout answer 404 on `articles().get()`; `get_by_link` finds them.
- `available_at` is always `None` on raw, and `None` for articles from before its rollout.
- `source.type` is `None` when the source is not classified or the article was indexed before source types existed. It can lag `type` in `sources().list()` after a reclassification.

## Migrating from 0.1

The full guide is on [docs.finlight.me](https://docs.finlight.me). The main changes:

| 0.1 (API v2) | 0.2 (API v3) |
|---|---|
| `use finlight_client::...` | `use finlight::...` (the crate is still `finlight-client`) |
| `client.articles.fetch_articles(&GetArticlesParams {..})` | `client.articles().search(&SearchParams::new()...)`, `search_all(...)` |
| `client.articles.fetch_article_by_link(&GetArticleByLinkParams {..})` | `client.articles().get_by_link(link, &options)`, `get(id, &options)` |
| `client.sources.get_sources()` | `client.sources().list()` |
| `client.websocket.stream(GetArticlesWebSocketParams {..})` | `client.stream(StreamParams::new()..., StreamOptions::new()).await?` |
| `client.raw_websocket.stream(..)` | `client.raw_stream(RawStreamParams::new()..., options).await?` |
| `WebSocketClient::new(config, WebSocketOptions {..})` | stream settings in `Config` (`ping_interval`, `watchdog_timeout`, ...) and hooks in `StreamOptions` |
| `construct_webhook_event(body, signature, secret, timestamp)` | `webhooks::construct_event(body, &headers, secret, &options)` or `construct_event_from_values(...)` |
| `Error::Api { status, body }`, `Error::Blocked` | `Error::Api(ApiError)` with `status()`, `code()`, `api_kind()`; `Error::Preempted` |
| `publish_date: DateTime<Utc>` | `published_at: Option<String>` + `published_at_time()` |
| `created_at` | `first_seen_at` |
| `revised_date` | `revised_at` |
| `images: Vec<String>` | `media: Vec<Media>` |
| `source: String` | `source.domain` |
| `sentiment`, `confidence` | `sentiment: Field<Sentiment>` (`label`, `confidence`) |
| `include_content`, `include_entities` | `include([IncludeBlock::CONTENT, IncludeBlock::COMPANIES, IncludeBlock::EVENTS])` |
| `page`, `page_size` | `limit`, `cursor` (`search_all` pages for you) |
| `language` | `languages(["en"])` |
| `is_update` | `StreamEventKind::Updated` |
| webhook body = article | `event.data` + `event.event_id` |

Switching a webhook to v3, in four steps:

1. Deploy the new SDK.
2. Answer `WrongPayloadVersion` with 2xx and keep its `raw` JSON if you need the data.
3. Switch the webhook to `payloadVersion` 3 in the finlight dashboard.
4. Wait about 2 h. Retries that were already queued still arrive in the old format.

## Development

```bash
cargo test                                   # unit, rule and conformance tests; no network, no keys
cargo test --test conformance -- S-18        # one kit case
scripts/sync-conformance.sh <monorepo>/shared/contracts/api-v3
FINLIGHT_API_KEY=... FINLIGHT_BASE_URL=https://api.dev.finlight.me \
  FINLIGHT_WSS_URL=wss://wss.dev.finlight.me cargo test --test live -- --ignored
```

The contract kit lives in `conformance/` (`CONTRACT_VERSION`, sha256 `MANIFEST`) and is not published. Every kit case runs as its own test in `tests/conformance.rs`; `CONFORMANCE.md` maps cases to tests, and the suite fails when the copy does not match the MANIFEST or a case has no test. Live tests run only against dev and need all three variables; production hosts are refused.

## License

MIT
