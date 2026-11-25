# finlight 클라이언트 (Rust)

*[English](README.md) | [简体中文](README.zh-CN.md) | [日本語](README.ja.md) | 한국어*

> **Breaking release.** 0.2는 finlight API v3만 지원하며(REST `/v3`, WebSocket `payloadVersion` 3, v3 웹훅) 0.1 코드는 동작하지 않습니다. 먼저 [0.1에서 마이그레이션](#01에서-마이그레이션)과 [변경 기록](CHANGELOG.md)을 읽어 주세요. API v2를 계속 쓰려면 `finlight-client = "0.1"`을 유지하세요.

*이 문서는 영어 README를 번역한 것이며 영어판에 맞춰 계속 갱신됩니다. 내용이 다르면 영어판이 우선합니다.*

[finlight.me](https://finlight.me)용 비동기 클라이언트입니다. 금융 뉴스 검색, 기사·기업·출처 조회, WebSocket으로 실시간 기사 수신, 웹훅 검증을 지원합니다. crate 이름은 `finlight-client`이고 라이브러리는 `finlight`로 import합니다.

## 버전

| SDK version | API version | Status |
|---|---|---|
| 0.2.x | v3 (`/v3`, WebSocket `payloadVersion` 3, 웹훅 v3) | 현재 |
| 0.1.x | v2 | 추가 릴리스 없음 |

SDK 버전과 API 버전은 서로 독립적입니다. 새 API 필드와 열거 값은 0.2.x 패치 릴리스로 추가됩니다. API v3를 지원하는 SDK: TypeScript 4.x, Python 3.x, PHP 2.x, Go `/v2`, .NET 2.x, Java 1.x, Ruby 1.x, Rust 0.2.

Cargo는 `"0.1"`을 0.2로 올리지 않습니다. `"*"` 같은 와일드카드는 올리므로 마이너 버전을 고정하세요.

## 요구 사항

- Rust 1.85 이상 (edition 2024)
- tokio 1.47 이상 (스트림마다 태스크를 하나 실행하므로 tokio 런타임 필요)
- reqwest 0.13 (rustls), serde 1, serde_json 1.0.142 이상. WebSocket 프로토콜은 SDK에 내장
- TLS는 rustls(aws-lc-rs 프로바이더) 사용. REST와 스트림 모두 운영체제 신뢰 저장소(rustls-platform-verifier)로 인증서를 검증하므로, 거기에 설치한 사내 CA는 둘 다에 적용됩니다. OpenSSL 불필요

## 설치

```bash
cargo add finlight-client@0.2 tokio --features tokio/full
cargo add futures-util   # for StreamExt on streams
```

## 빠른 시작

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

SDK는 지정한 파라미터만 보냅니다. 기본값을 추가하지 않습니다. `limit`(20), `languages`(`["en"]`), 기간은 서버가 정합니다. 파라미터 이름은 구조체 필드이므로 철자가 틀리면 컴파일되지 않습니다. `SearchParams::from_json`은 알 수 없는 키를 요청 전에 `Error::Validation`으로 거부합니다.

## 기사 검색

`articles().search()`는 한 페이지를 반환합니다. `articles().search_all()`은 커서를 대신 따라갑니다.

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

- `search_all`은 호출 시점에 요청 본문을 고정합니다. 이후 페이지에서는 `cursor`만 바뀌므로 커서가 계속 유효합니다.
- `has_more: false`, `next_cursor: None`, 또는 `max_pages`(기본 10)에서 멈춥니다. 한도에 도달하면 경고를 남기고 `truncated()`가 true가 되며 `next_cursor()`로 이어서 가져올 수 있습니다.
- 각 페이지는 과금되는 호출입니다. 결과는 한 번만 순회할 수 있습니다. 기사 단위(`next()` 또는 `futures`의 `Stream`)나 페이지 단위(`next_page()`) 중 하나를 쓰며, 다른 방식은 `Error::Validation`을 반환합니다.
- `from`과 `to`는 `SystemTime`(밀리초 포함 UTC로 전송), `DateInput::date(y, m, d)`(`YYYY-MM-DD`로 전송), 문자열(그대로 전송)을 받습니다. `order_by` 타임스탬프에 적용됩니다.
- `source_types`와 `exclude_source_types`는 소스 유형으로 거릅니다: `SourceType::NEWSWIRE`, `PRESS_RELEASE`, `FINANCIAL_MEDIA`, `GENERAL_MEDIA`, `AGGREGATOR`, `BLOG`, `GOVERNMENT`. 분류되지 않은 소스의 기사는 `source_types`와 일치하지 않고 `exclude_source_types`에서는 남습니다. 둘 다 소스 풀 안에서만 거르며 소스를 추가하지 않습니다. `StreamParams`와 `RawStreamParams`에서도 쓸 수 있습니다.
- 디코딩할 수 없는 항목은 건너뛰고 로그에 남기며 `page.skipped`에 담깁니다. 페이지의 나머지는 그대로 쓸 수 있습니다.

## 기사, 기업, 출처 조회

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
    let record = client.companies().get(4821).await?; // no SDK cache: keep it yourself
    println!("{} {:?} {}", found.len(), record.cache_info(), many.skipped().len());

    // With your own ETag: Ok(None) means "not modified"
    let fresh = client.companies().get_if_none_match(4821, "\"80b88d6b\"").await?;
    println!("changed: {}", fresh.is_some());

    let sources = client.sources().list().await?;
    println!("{} sources", sources.len());
    Ok(())
}
```

- 404는 빈 결과가 아니라 `api_kind() == ApiErrorKind::NotFound`인 `Error::Api`입니다.
- ID 도입 전에 색인된 기사는 ID로 조회할 수 없습니다. `get_by_link`를 쓰세요.
- 기업 엔드포인트에는 `entityAccess` 권한이 필요하며, 없으면 403(`ApiErrorKind::Forbidden`)을 반환합니다.
- SDK는 기업 레코드를 캐시하지 않습니다. `companies().get()`은 항상 일반 요청을 보냅니다. 주요 기업 정보는 기사 페이로드에 이미 들어 있습니다. `cache_info()`로 레코드의 `ETag`와 `Cache-Control`을 얻을 수 있습니다. 크레딧을 아끼려면 레코드를 직접 보관하고 `max-age` 동안 다시 호출하지 마세요. 보관한 레코드를 확인하려면 그 ETag를 `get_if_none_match`에 넘기세요. 304이면 `Ok(None)`을 반환합니다. 304도 1크레딧입니다.
- 조회도 기사 호출처럼 크레딧을 씁니다. `companies()`(`search`, `get_many`, `get`, `get_if_none_match`)와 `sources().list()`는 요청마다 1크레딧이며, ID가 여러 개인 `get_many`도 1크레딧입니다. 304 재검증과 오류도 1크레딧입니다.
- `CompanyList`와 `SourceList`는 슬라이스처럼 쓸 수 있고, 디코딩하지 못한 항목은 `skipped()`에 있습니다.

## 키 없음, null, 빈 목록

일부 키는 요금제와 `include`에 따라 달라집니다. 키가 없는 것, `null`, `[]`은 의미가 다르며 SDK는 이를 구분합니다.

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

- 키 없음: 요청하지 않았거나, 요금제에 없거나, raw 채널입니다.
- `null`: 요청했고 권한도 있지만 값이 없습니다(예: `content`에 본문 없음).
- `[]`: 실제로 빈 목록입니다(예: 태깅했지만 기업을 찾지 못함).
- `content`와 `sentiment`는 `Field<T>`(`Absent`, `Null`, `Value`)입니다. `companies`, `events`, `events[].company_ids`, `Source::has_content`는 `Option<_>`이며 `None`은 "키 없음"이지 "비어 있음"이 아닙니다.
- `content_present()`, `sentiment_present()`, `companies_present()`, `events_present()`는 키가 JSON에 있었는지(`null` 포함) 알려 줍니다.
- 새 서버의 알 수 없는 키는 무시됩니다. `raw_json()`은 받은 그대로의 JSON을 반환합니다. serde로 모델을 다시 쓰면 없던 키는 계속 없습니다.
- 어휘는 열려 있습니다. SDK가 모르는 값은 `CompanyRole::Other("...")`로 보존됩니다. `role == CompanyRole::ACQUIRER`처럼 상수와 비교하세요.

## 실시간 스트림

`client.stream()`은 보강된 기사를, `client.raw_stream()`은 보강 전 raw 골격을 받습니다. 백그라운드 태스크가 소켓을 관리하며 25초마다 ping을 보내고, 60초 동안 pong이 없으면 다시 연결하고, 115분 뒤 연결을 교체하고, 재연결 때마다 다시 구독하며, 중복을 걸러냅니다.

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

알아 둘 점:

- **연결 수 한도.** 요금제의 WebSocket 한도는 보강 스트림, raw 스트림, 구버전 SDK가 함께 씁니다. 신구 스트림을 동시에 실행하지 마세요. 한도에 도달하면 SDK는 최소 60초 기다린 뒤 다시 시도합니다. `StreamOptions::takeover(true)`는 가장 오래된 연결을 대신 교체합니다.
- **2시간 한도.** 게이트웨이는 2시간 뒤 모든 연결을 끊습니다. SDK는 그 전에 새 연결을 열어 다시 구독하고, 새 연결이 admitted되면 이전 연결을 닫습니다. 요금제가 연결 한도에 도달해 있으면(예: 한도 1) 이전 연결이 아직 계산되어 새 연결이 거부됩니다. 이때 SDK는 이전 연결을 닫고 바로 다시 연결하므로 끊김은 연결 한 번 정도입니다. 그 연결마저 거부될 때만 평소처럼 최소 60초를 기다립니다.
- **초기 푸시.** 구독할 때마다 보강 스트림은 조건에 맞는 최신 기사 하나를 보냅니다. 오래된 기사일 수 있습니다. 이 스트림에서 이미 전달된 기사면 SDK가 버립니다.
- **큰 프레임.** 32,000바이트를 넘는 실시간 프레임은 서버가 알림 없이 버립니다. 긴 기사에 `include: content`를 쓰면 생길 수 있습니다. 본문은 `articles().get(id, ...)`로 가져오세요.
- `on_admitted`는 서버가 구독을 검사하기 전에 호출됩니다. 구독이 수락되었다는 뜻이 아닙니다.
- SDK는 빠진 데이터를 자동으로 채우지 않습니다. `on_reconnect`의 `last_seen_at`으로 REST를 통해 채우세요.
- 스트림을 drop하거나 `stream.close().await`를 호출하면 소켓을 1000으로 닫고 모든 백그라운드 작업을 멈춥니다.
- 키 거부, WebSocket이 없는 요금제, 잘못된 구독, 종료 코드 1008, `connection_limit` 선점(`Error::Preempted`)에서는 스트림이 `Err`로 끝납니다. 그 밖의 문제는 `on_error`로 전달되고 스트림은 계속됩니다. 훅의 panic은 잡아서 로그에 남깁니다.
- **WebSocket 할당량 소진.** 기간 할당량을 다 쓰면(다이얼 거부 또는 `quota_exceeded` 선점) 스트림은 끝나지 않고, 할당량이 허용할 때까지 1시간마다 다시 시도합니다. 대기할 때마다 경고를 남기고 `on_quota_wait`에 알립니다. `Config::quota_reconnect`를 `false`로 두면 오류로 끝납니다. `quota_reconnect_interval`로 간격을 바꿀 수 있습니다. REST 호출은 기다리지 않습니다. 월간 429는 즉시 실패합니다.
- `raw_stream`에는 `include`도 보강 필터도 없습니다. `source_types`와 `exclude_source_types`는 쓸 수 있습니다.

## 웹훅

`webhooks::construct_event(raw_body, headers, secret, &options)`는 서명과 타임스탬프(기본 허용 오차 300초)를 확인한 뒤 본문을 읽습니다. `WebhookPayload::Event`를 반환하거나, 서명이 올바른 구형식 본문에는 `WebhookPayload::WrongPayloadVersion`을 반환합니다.

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

서명은 원시 바이트에 대해 계산됩니다. JSON 파서가 실행되기 전에 받으세요:

- axum: `Json<T>`가 아니라 `axum::body::Bytes`와 `HeaderMap`을 받습니다.
- actix-web: `web::Bytes`를 받고 `req.headers()` 값을 `construct_event_from_values`에 넘깁니다.
- warp: `warp::body::bytes()`와 `warp::header::headers_cloned()`.
- hyper: 본문을 `Bytes`로 모으고 `parts.headers`를 넘깁니다.

- `headers`는 `http::HeaderMap`, 문자열 `HashMap`이나 `BTreeMap`, `(name, value)` 쌍을 받습니다. 이름은 대소문자를 구분하지 않으며 CGI 형식(`HTTP_X_WEBHOOK_TIMESTAMP`)도 지원합니다.
- 전달은 중복될 수 있습니다. `event_id`로 중복을 제거하세요.
- 5초 안에 2xx로 응답하고 오래 걸리는 작업은 그 뒤에 하세요.
- 410으로 응답하지 마세요. 웹훅이 즉시 비활성화됩니다.
- 대시보드의 테스트 전달에서는 `is_test()`가 true입니다.
- `webhooks::verify_signature(raw_body, signature, timestamp, secret, &options)`는 서명만 확인합니다. `construct_event_from_values`는 두 헤더 값을 직접 받습니다.
- SDK는 웹훅을 관리하지 않습니다. 생성과 변경은 finlight 대시보드에서 하세요.

## 오류와 재시도

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

모든 오류는 `finlight::Error`의 한 변형이며 `kind()`가 종류를 문자열로 알려 줍니다. API 오류에는 `status()`, `code()`, `title()`, `detail()`, `request_id()`, `errors()`(`FieldIssue`), `api_kind()`, `raw()`(받은 본문)가 있습니다. 논리적 호출마다 고유한 `X-Request-Id`를 보내며 재시도에서는 같은 값을 씁니다. 원인은 `std::error::Error::source`로 볼 수 있습니다.

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

재시도:

| 응답 | 재시도 |
|---|---|
| 400, 401, 403, 404 | 안 함 |
| 429 monthly / unknown | 안 함 |
| 429 burst | `Retry-After`만큼 정확히 기다린 뒤 한 번(헤더가 없으면 10초). 기본으로 켜져 있으며 `RetryOnBurst::Off`로 끔 |
| 5xx, 네트워크 오류, 타임아웃 | 최대 3번, 대기 시간을 늘리며(지터 포함) |

호출을 취소하려면 future를 drop하세요(예: `tokio::time::timeout`, `tokio::select!`). 대기는 즉시 끝나고 그 뒤로 재시도하지 않습니다. `Config::total_timeout`은 재시도와 burst 대기를 포함한 호출 전체를 제한합니다.

## 설정

`Config::new(key)`는 기본값을 씁니다. `Config::builder(key)....build()?`는 값을 검사합니다. 시간은 `std::time::Duration`입니다.

| 필드 | 기본값 | 의미 |
|---|---|---|
| `api_key` | - | 필수, 비어 있으면 안 됨. |
| `base_url` | `https://api.finlight.me` | REST 기본 URL. |
| `wss_url` | `wss://wss.finlight.me` | WebSocket URL. raw는 `<wss_url>/raw`. |
| `timeout` | 30초 | HTTP 시도 한 번의 타임아웃. |
| `total_timeout` | 없음 | 선택: 재시도와 대기를 포함한 호출 전체의 상한. |
| `max_retries` | 3 | 5xx, 네트워크 오류, 타임아웃의 재시도 횟수. |
| `retry_base_delay` / `retry_max_delay` | 0.5초 / 8초 | 재시도 사이의 백오프(지터 포함). |
| `retry_on_burst` | `RetryOnBurst::Once` | 429 burst 처리(`Once` 또는 `Off`). |
| `burst_wait` | 10초 | 429에 `Retry-After`가 없을 때의 대기 시간. |
| `max_pages` | 10 | `search_all`의 페이지 한도. |
| `max_response_bytes` | 32 MiB | REST 응답 본문의 최대 크기. |
| `ping_interval` / `watchdog_timeout` / `rotation_interval` | 25초 / 60초 / 115분 | 스트림 ping, pong 감시, 연결 교체. |
| `reconnect_base_delay` / `reconnect_max_delay` | 1초 / 30초 | 스트림 재연결 백오프. |
| `long_backoff` | 60초 | "연결 과다"와 429 다이얼 거부 후 대기 시간. |
| `quota_reconnect` / `quota_reconnect_interval` | `true` / 1시간 | 스트림 전용: WebSocket 할당량을 다 쓰면 1시간마다 재시도(`false`면 종료). REST는 새 할당량을 기다리지 않습니다. |
| `search_failed_threshold` | 5 | `SEARCH_FAILED`가 이만큼 연속되면 `on_error`에 알림. |
| `dedupe_cache_size` | 1000 | 스트림별 중복 필터 항목 수. |
| `max_message_bytes` | 32 MiB(최소 16 MiB) | 수신 스트림 메시지의 최대 크기. |
| `connect_timeout` | 30초 | WebSocket 다이얼과 핸드셰이크 타임아웃. |
| `http_client` | - | REST용 자체 `reqwest::Client`(예: 프록시). 이 경우 그 클라이언트의 타임아웃이 적용되고 `timeout`은 쓰이지 않습니다. 자체 재시도는 끄세요. SDK는 이 클라이언트를 닫지 않습니다. |
| `ws_connector` | - | 스트림 연결용 바이트 스트림을 여는 자체 `WsConnector`(예: 프록시 터널). TLS(운영체제 신뢰 저장소)와 핸드셰이크는 SDK가 합니다. |

테스트는 tokio의 일시 정지 시계(`tokio`의 `test-util` 기능)를 씁니다. 별도의 시계 옵션은 없습니다.

## 로깅

SDK는 `tracing`으로 대상 `finlight`에 로그를 남깁니다. 구독자가 없으면 아무것도 출력하지 않습니다. stdout이나 stderr에 직접 쓰지 않고 전역 핸들러도 설치하지 않습니다. 재시도, 재연결, 할당량 대기는 경고이고 메시지별 세부 내용은 debug입니다. API 키, 웹훅 시크릿, 서명, 전체 본문은 로그에 남지 않으며 `Config`, `Client`, 오류의 `Debug` 출력에도 포함되지 않습니다. WebSocket 프로토콜은 SDK가 직접 구현하므로 어떤 수준에서도 전송 라이브러리가 스트림 프레임 내용을 로그에 남기지 않습니다.

## 데이터 참고 사항

- 일부 오래된 기사는 `published_at`이 `1970-01-01T00:00:00.000Z`입니다. SDK는 그대로 전달합니다. 텍스트가 없으면 `summary`와 `content`는 `None` / `Field::Null`입니다.
- 역할 기능 이전에 태깅된 기사에서는 `companies[].role`과 `relevance`가 `None`이고 `events`는 `Some([])`입니다.
- ID 도입 전에 색인된 기사는 `articles().get()`에서 404를 반환합니다. `get_by_link`로는 찾을 수 있습니다.
- `available_at`은 raw에서 항상 `None`이며 도입 이전 기사에서도 `None`입니다.
- 소스가 분류되지 않았거나 소스 유형이 도입되기 전에 인덱싱된 기사라면 `source.type`은 `None`입니다. 재분류 후에는 `sources().list()`의 `type`보다 늦게 반영될 수 있습니다.

## 0.1에서 마이그레이션

전체 가이드는 [docs.finlight.me](https://docs.finlight.me)에 있습니다. 주요 변경 사항:

| 0.1 (API v2) | 0.2 (API v3) |
|---|---|
| `use finlight_client::...` | `use finlight::...` (crate 이름은 그대로 `finlight-client`) |
| `client.articles.fetch_articles(&GetArticlesParams {..})` | `client.articles().search(&SearchParams::new()...)`, `search_all(...)` |
| `client.articles.fetch_article_by_link(&GetArticleByLinkParams {..})` | `client.articles().get_by_link(link, &options)`, `get(id, &options)` |
| `client.sources.get_sources()` | `client.sources().list()` |
| `client.websocket.stream(GetArticlesWebSocketParams {..})` | `client.stream(StreamParams::new()..., StreamOptions::new()).await?` |
| `client.raw_websocket.stream(..)` | `client.raw_stream(RawStreamParams::new()..., options).await?` |
| `WebSocketClient::new(config, WebSocketOptions {..})` | 스트림 설정은 `Config`(`ping_interval`, `watchdog_timeout` 등), 훅은 `StreamOptions` |
| `construct_webhook_event(body, signature, secret, timestamp)` | `webhooks::construct_event(body, &headers, secret, &options)` 또는 `construct_event_from_values(...)` |
| `Error::Api { status, body }`, `Error::Blocked` | `status()`, `code()`, `api_kind()`가 있는 `Error::Api(ApiError)`, `Error::Preempted` |
| `publish_date: DateTime<Utc>` | `published_at: Option<String>` + `published_at_time()` |
| `created_at` | `first_seen_at` |
| `revised_date` | `revised_at` |
| `images: Vec<String>` | `media: Vec<Media>` |
| `source: String` | `source.domain` |
| `sentiment`, `confidence` | `sentiment: Field<Sentiment>` (`label`, `confidence`) |
| `include_content`, `include_entities` | `include([IncludeBlock::CONTENT, IncludeBlock::COMPANIES, IncludeBlock::EVENTS])` |
| `page`, `page_size` | `limit`, `cursor` (페이지 넘김은 `search_all`) |
| `language` | `languages(["en"])` |
| `is_update` | `StreamEventKind::Updated` |
| 웹훅 본문 = 기사 | `event.data` + `event.event_id` |

웹훅을 v3로 전환하는 네 단계:

1. 새 SDK를 배포합니다.
2. `WrongPayloadVersion`에 2xx로 응답하고, 데이터가 필요하면 `raw` JSON을 보관합니다.
3. finlight 대시보드에서 웹훅을 `payloadVersion` 3으로 전환합니다.
4. 약 2시간 기다립니다. 이미 대기열에 있는 재시도는 여전히 구형식으로 도착합니다.

## 개발

```bash
cargo test                                   # unit, rule and conformance tests; no network, no keys
cargo test --test conformance -- S-18        # one kit case
scripts/sync-conformance.sh <monorepo>/shared/contracts/api-v3
FINLIGHT_API_KEY=... FINLIGHT_BASE_URL=https://api.dev.finlight.me \
  FINLIGHT_WSS_URL=wss://wss.dev.finlight.me cargo test --test live -- --ignored
```

계약 키트는 `conformance/`(`CONTRACT_VERSION`, sha256 `MANIFEST`)에 있으며 배포되지 않습니다. 키트의 각 사례는 `tests/conformance.rs`에서 개별 테스트로 실행됩니다. `CONFORMANCE.md`가 사례와 테스트를 연결합니다. 사본이 MANIFEST와 다르거나 테스트가 없는 사례가 있으면 테스트가 실패합니다. 라이브 테스트는 dev에서만 실행하며 세 변수가 모두 필요합니다. 운영 호스트는 거부됩니다.

## 라이선스

MIT
