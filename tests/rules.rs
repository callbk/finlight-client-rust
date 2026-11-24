//! Rule tests beyond the kit cases (api-v3-client-rules.md): scope, request shape, models,
//! retries and cancellation, configuration, logging, stream lifecycle and webhooks.

mod common;

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// Every name of the public surface (sdk-style.md), so a rename fails to compile here.
#[allow(unused_imports)]
use finlight::{
    AdmittedInfo, ApiError, ApiErrorKind, Article, ArticlePage, ArticleSource, Articles, CacheInfo,
    Client, Companies, Company, CompanyList, CompanyRecord, CompanyRole, CompanySearchParams,
    Config, ConfigBuilder, ConnectionErrorCode, DateInput, DecodeError, Error, EventDirection,
    EventTag, EventType, Field, FieldIssue, FinlightStream, GetArticleOptions, Importance,
    IncludeBlock, Listing, Media, NetworkError, OrderBy, PAYLOAD_VERSION, Pagination,
    PreemptReason, PreemptedError, ProblemCode, QuotaWaitInfo, RateLimitType, RawStreamParams,
    ReconnectInfo, ReconnectReason, Result as FinlightResult, RetryOnBurst, SearchAllOptions,
    SearchAllResult, SearchParams, Sentiment, SentimentLabel, SkippedEntry, SortOrder, Source,
    SourceList, SourceType, Sources, StreamEvent, StreamEventKind, StreamEventName, StreamOptions,
    StreamParams, TimeoutError, VERSION, ValidationError, WebhookEvent, WebhookEventName,
    WebhookPayload, WebhookVerificationError, WebhookVerificationReason, WrongPayloadVersion,
    WsConnectFuture, WsConnector, WsIo,
    webhooks::{
        DEFAULT_TOLERANCE, WebhookHeaders, WebhookVerifyOptions, construct_event,
        construct_event_from_values, verify_signature,
    },
};
use futures_util::StreamExt;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

use common::http_mock::{HttpMock, Scripted};
use common::ws_mock::{DialScript, WsMock};
use common::{KIT_API_KEY, load, run_paused, settle, spawn_spinner, test_http_client, wait_for};

fn client(base_url: &str) -> Client {
    Client::new(
        Config::builder(KIT_API_KEY)
            .base_url(base_url)
            .http_client(test_http_client())
            .build()
            .unwrap(),
    )
    .unwrap()
}

fn article_json() -> Value {
    load("fixtures/decode/full-article.json")["wire"].clone()
}

fn page_of(articles: Vec<Value>) -> Value {
    json!({"articles": articles, "pagination": {"limit": 20, "returned": 1, "hasMore": false, "nextCursor": null}})
}

fn full_search_params() -> SearchParams {
    SearchParams::new()
        .query("ticker:XMPL")
        .sources(["a.example"])
        .opt_in_sources(["b.example"])
        .exclude_sources(["c.example"])
        .source_types([SourceType::NEWSWIRE, "wire_service"])
        .exclude_source_types([SourceType::PRESS_RELEASE])
        .from(DateInput::date(2026, 11, 1))
        .to("2026-11-23")
        .languages(["en"])
        .tickers(["XMPL"])
        .countries(["US"])
        .categories(["markets"])
        .has_content(false)
        .roles([CompanyRole::ACQUIRER])
        .min_relevance(0.0)
        .event_types([EventType::MNA])
        .event_directions([EventDirection::POSITIVE])
        .min_importance(Importance::HIGH)
        .sectors(["Industrials"])
        .company_countries(["US"])
        .exchanges(["XNAS"])
        .order_by(OrderBy::AVAILABLE_AT)
        .order(SortOrder::ASC)
        .limit(1)
        .cursor("c")
        .include([IncludeBlock::CONTENT])
}

const FORBIDDEN_KEYS: [&str; 6] = [
    "includeContent",
    "includeEntities",
    "extended",
    "pageSize",
    "publishDate",
    "isUpdate",
];

// ---- R-SCOPE-1 -----------------------------------------------------------------------

#[test]
fn scope_requests_carry_no_v2_keys() {
    run_paused(async {
        let mock = HttpMock::start().await;
        mock.always(Scripted::json(200, &page_of(vec![])));
        client(&mock.base_url)
            .articles()
            .search(&full_search_params())
            .await
            .unwrap();
        let body = mock.requests()[0].body_json();
        assert_eq!(
            body.as_object().unwrap().len(),
            26,
            "every search key is sent: {body}"
        );
        assert_eq!(
            body["sourceTypes"],
            json!(["newswire", "wire_service"]),
            "an unknown source type is sent unchanged"
        );
        assert_eq!(body["excludeSourceTypes"], json!(["press_release"]));
        assert_eq!(body["hasContent"], false, "an explicit false is sent");
        assert_eq!(body["minRelevance"], 0.0, "an explicit 0 is sent");
        for k in FORBIDDEN_KEYS {
            assert!(body.get(k).is_none());
        }

        let ws = WsMock::start().await;
        let spinner = spawn_spinner();
        let c = Client::new(
            Config::builder(KIT_API_KEY)
                .wss_url(ws.url.clone())
                .build()
                .unwrap(),
        )
        .unwrap();
        let params = StreamParams::new()
            .query("q")
            .sources(["a"])
            .exclude_sources(["b"])
            .opt_in_sources(["c"])
            .language("en")
            .tickers(["XMPL"])
            .countries(["US"])
            .categories(["markets"])
            .source_types([SourceType::NEWSWIRE, "wire_service"])
            .exclude_source_types([SourceType::BLOG])
            .exclude_empty_content(false)
            .include_updates(true)
            .include([IncludeBlock::COMPANIES]);
        let mut s = c.stream(params, StreamOptions::new()).await.unwrap();
        let frame = ws.wait_dial(0).await.conn.unwrap().next("subscribe").await;
        assert_eq!(frame.as_object().unwrap().len(), 15, "{frame}");
        // Unknown values go out unchanged; the server rejects them (S-29).
        assert_eq!(frame["sourceTypes"], json!(["newswire", "wire_service"]));
        assert_eq!(frame["excludeSourceTypes"], json!(["blog"]));
        for k in FORBIDDEN_KEYS {
            assert!(frame.get(k).is_none());
        }
        s.close().await;
        let raw = RawStreamParams::new()
            .query("q")
            .sources(["a"])
            .exclude_sources(["b"])
            .source_types([SourceType::PRESS_RELEASE])
            .exclude_source_types([SourceType::AGGREGATOR])
            .language("en")
            .include_updates(true);
        let mut s = c.raw_stream(raw, StreamOptions::new()).await.unwrap();
        let dial = ws.wait_dial(1).await;
        assert_eq!(dial.path, "/raw");
        let frame = dial.conn.unwrap().next("subscribe").await;
        assert_eq!(frame.as_object().unwrap().len(), 9, "{frame}");
        assert!(frame.get("include").is_none());
        assert_eq!(frame["sourceTypes"], json!(["press_release"]));
        assert_eq!(frame["excludeSourceTypes"], json!(["aggregator"]));
        // Both stream kinds take the source-type filters, also from JSON.
        for key in ["sourceTypes", "excludeSourceTypes"] {
            let json = format!(r#"{{"{key}":["blog"]}}"#);
            assert!(StreamParams::from_json(&json).is_ok(), "{key}");
            assert!(RawStreamParams::from_json(&json).is_ok(), "{key}");
        }
        s.close().await;
        spinner.abort();
    });
}

#[test]
fn scope_sources_have_no_v1_v2_paths_or_keys() {
    // Allowed only in the frame decoder (R-WS-1) and the wrong-version check (R-HOOK-2).
    let tokens = [
        "/v1/",
        "/v2/",
        "sendarticle",
        "admin_kick",
        "includecontent",
        "include_content",
        "includeentities",
        "include_entities",
        "extended",
        "pagesize",
        "page_size",
        "publishdate",
        "publish_date",
        "isupdate",
        "is_update",
    ];
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let mut hits = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&path).unwrap().to_ascii_lowercase();
        for t in tokens {
            if text.contains(t) && !(name == "frames.rs" && t == "sendarticle") {
                hits.push(format!("{name}: {t}"));
            }
        }
    }
    assert!(hits.is_empty(), "{hits:?}");
}

// ---- R-REQ-1 .. R-REQ-3 ----------------------------------------------------------------

#[test]
fn requests_format_dates_and_lists() {
    run_paused(async {
        let mock = HttpMock::start().await;
        mock.always(Scripted::json(200, &page_of(vec![])));
        let c = client(&mock.base_url);
        let t = UNIX_EPOCH + Duration::from_millis(1_795_467_845_123);
        let params = SearchParams::new()
            .from(t)
            .to(DateInput::date(2026, 1, 2))
            .include(Vec::<IncludeBlock>::new());
        c.articles().search(&params).await.unwrap();
        let body = mock.requests()[0].body_json();
        assert_eq!(
            body,
            json!({"from": "2026-11-23T21:04:05.123Z", "to": "2026-01-02"}),
            "empty include is left out"
        );
        c.articles()
            .search(&SearchParams::new().from("yesterday-ish"))
            .await
            .unwrap();
        assert_eq!(
            mock.requests()[1].body_json(),
            json!({"from": "yesterday-ish"}),
            "a string goes out unchanged"
        );
        // GET: include is one comma-joined parameter; the link is encoded exactly once.
        mock.reset();
        mock.always(Scripted::json(200, &json!({"article": article_json()})));
        let link = "https://x.example/a b?c=d&e=%41";
        c.articles()
            .get_by_link(
                link,
                &GetArticleOptions::new().include(["content", "events"]),
            )
            .await
            .unwrap();
        let req = &mock.requests()[0];
        let q = req.query.clone().unwrap();
        assert!(q.contains("include=content%2Cevents"));
        assert_eq!(common::query_pairs(Some(&q))[0].1, link);
    });
}

#[test]
fn requests_shape_is_checked_locally() {
    run_paused(async {
        let mock = HttpMock::start().await;
        mock.always(Scripted::json(200, &page_of(vec![])));
        let c = client(&mock.base_url);
        let bad = [
            SearchParams::new().limit(0),
            SearchParams::new().limit(101),
            SearchParams::new().languages(Vec::<String>::new()),
            SearchParams::new().min_relevance(1.5),
            SearchParams::new().min_relevance(f64::NAN),
            SearchParams::new().include(["content", "companies", "events", "content"]),
            SearchParams::new().cursor(""),
        ];
        for p in bad {
            let err = c.articles().search(&p).await.unwrap_err();
            assert!(
                matches!(&err, Error::Validation(v) if v.param().is_some()),
                "{err:?}"
            );
            assert!(c.articles().search_all(p, SearchAllOptions::new()).is_err());
        }
        let long_q = "x".repeat(201);
        assert!(
            c.companies()
                .search(&CompanySearchParams::new(long_q))
                .await
                .is_err()
        );
        assert!(
            c.companies()
                .search(&CompanySearchParams::new("ok").mic("M".repeat(33)))
                .await
                .is_err()
        );
        assert!(
            c.companies()
                .get_many(&(1..=11).collect::<Vec<_>>())
                .await
                .is_err()
        );
        assert!(c.companies().get_many(&[0]).await.is_err());
        assert!(c.companies().get(0).await.is_err());
        assert!(
            c.articles()
                .get("", &GetArticleOptions::new())
                .await
                .is_err()
        );
        assert!(
            c.articles()
                .get_by_link("", &GetArticleOptions::new())
                .await
                .is_err()
        );
        let ok_q = "ü".repeat(200);
        mock.always(Scripted::json(200, &json!({"companies": []})));
        assert!(
            c.companies()
                .search(&CompanySearchParams::new(ok_q))
                .await
                .is_ok(),
            "200 characters, not bytes"
        );
        assert_eq!(mock.count(), 1, "nothing invalid reached the server");
        // An unknown vocabulary value goes out unchanged.
        mock.reset();
        mock.always(Scripted::json(200, &page_of(vec![])));
        c.articles()
            .search(&SearchParams::new().event_types(["brand_new_type"]))
            .await
            .unwrap();
        assert_eq!(
            mock.requests()[0].body_json(),
            json!({"eventTypes": ["brand_new_type"]})
        );
        // Unknown keys in JSON params are named.
        let err = StreamParams::from_json(r#"{"includeContent":true}"#).unwrap_err();
        assert!(matches!(&err, Error::Validation(v) if v.param() == Some("includeContent")));
    });
}

// ---- R-MODELS-1 .. R-MODELS-3 ---------------------------------------------------------

#[test]
fn models_skip_broken_entries_and_keep_values() {
    run_paused(async {
        let mock = HttpMock::start().await;
        let mut broken = article_json();
        broken.as_object_mut().unwrap().remove("link");
        let mut odd = article_json();
        odd["publishedAt"] = json!("now");
        odd["summary"] = json!("");
        odd["sentiment"]["confidence"] = json!(0.912345678);
        let mut wrong_type = article_json();
        wrong_type["sentiment"]["confidence"] = json!("0.95");
        mock.enqueue(Scripted::json(
            200,
            &page_of(vec![
                article_json(),
                broken.clone(),
                odd,
                wrong_type,
                json!(null),
            ]),
        ));
        let c = client(&mock.base_url);
        let page = c.articles().search(&SearchParams::new()).await.unwrap();
        assert_eq!(page.articles.len(), 2);
        assert_eq!(
            page.skipped.iter().map(|s| s.index).collect::<Vec<_>>(),
            vec![1, 3, 4]
        );
        assert_eq!(
            serde_json::from_str::<Value>(&page.skipped[0].raw).unwrap(),
            broken
        );
        assert!(
            page.skipped[1].reason.contains("confidence")
                || page.skipped[1].reason.contains("string")
        );
        let odd = &page.articles[1];
        assert_eq!(odd.published_at.as_deref(), Some("now"));
        assert_eq!(odd.published_at_time(), None);
        assert_eq!(odd.summary.as_deref(), Some(""));
        assert_eq!(odd.sentiment.value().unwrap().confidence, Some(0.912345678));
        // A single lookup without `link` is a decode error.
        mock.enqueue(Scripted::json(200, &json!({"article": broken})));
        let err = c
            .articles()
            .get("x", &GetArticleOptions::new())
            .await
            .unwrap_err();
        match err {
            Error::Decode(d) => {
                assert!(d.path().unwrap().starts_with("article"));
                assert!(d.raw().is_some());
            }
            other => panic!("expected a decode error, got {other:?}"),
        }
        // Unknown keys on the envelope and a missing optional key are fine; null lists read as [].
        let mut sparse = article_json();
        let o = sparse.as_object_mut().unwrap();
        o.remove("revisedAt");
        o.insert("media".into(), Value::Null);
        mock.enqueue(Scripted::json(
            200,
            &json!({"article": sparse, "extra": {"a": 1}}),
        ));
        let a = c
            .articles()
            .get("x", &GetArticleOptions::new())
            .await
            .unwrap();
        assert!(a.revised_at.is_none() && a.media.is_empty());
    });
}

#[test]
fn models_round_trip_keeps_absent_absent() {
    let wire = load("fixtures/decode/raw-skeleton.json")["wire"].clone();
    let a: Article = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(&a).unwrap(), wire);
    let mut with_null = wire.clone();
    with_null["content"] = Value::Null;
    let a: Article = serde_json::from_value(with_null.clone()).unwrap();
    assert!(a.content.is_null() && a.content_present());
    assert_eq!(serde_json::to_value(&a).unwrap(), with_null);
}

// ---- R-ERR-1, R-RETRY-1 -----------------------------------------------------------------

#[test]
fn errors_classify_429_from_detail_and_never_wait_on_unknown() {
    run_paused(async {
        let mock = HttpMock::start().await;
        let c = client(&mock.base_url);
        for (detail, want) in [
            (
                "Throttling Exception: Exceeded token limit of 5 for the current period.",
                "monthly",
            ),
            (
                "Throttling Exception: Exceeded rate limit of 5 requests per 10 seconds.",
                "burst",
            ),
            ("Something else", "unknown"),
        ] {
            mock.reset();
            mock.always(Scripted::json(
                429,
                &json!({"statusCode": 429, "message": detail}),
            ));
            let start = tokio::time::Instant::now();
            let err = c.articles().search(&SearchParams::new()).await.unwrap_err();
            let e = err.as_api().unwrap();
            assert_eq!(e.limit_type().unwrap().as_str(), want);
            assert_eq!(e.api_kind(), ApiErrorKind::RateLimited);
            let waited = start.elapsed();
            if want == "burst" {
                assert_eq!(mock.count(), 2);
                assert_eq!(waited, Duration::from_secs(10));
            } else {
                assert_eq!(mock.count(), 1);
                assert_eq!(waited, Duration::ZERO);
            }
        }
        // NestJS form with a message array.
        mock.reset();
        mock.always(Scripted::json(
            400,
            &json!({"statusCode": 400, "message": ["a is bad", "b is bad"]}),
        ));
        let err = c.articles().search(&SearchParams::new()).await.unwrap_err();
        assert_eq!(err.as_api().unwrap().detail(), Some("a is bad; b is bad"));
        assert_eq!(
            err.as_api().unwrap().api_kind(),
            ApiErrorKind::ValidationFailed
        );
    });
}

#[test]
fn retry_cancel_during_burst_wait_and_total_timeout() {
    run_paused(async {
        let mock = HttpMock::start().await;
        let burst = load("fixtures/errors/rate-limited-burst.json")["response"].clone();
        mock.always(Scripted::from_fixture(&burst));
        // Cancelling (dropping the future) ends the wait at once; nothing is retried.
        let c = client(&mock.base_url);
        let spinner = spawn_spinner();
        let task = tokio::spawn(async move { c.articles().search(&SearchParams::new()).await });
        wait_for("first attempt", || mock.count() == 1).await;
        settle().await;
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(!task.is_finished(), "waiting for Retry-After");
        task.abort();
        settle().await;
        assert!(task.is_finished(), "the cancelled call ends at once");
        tokio::time::advance(Duration::from_secs(30)).await;
        settle().await;
        assert_eq!(mock.count(), 1, "no retry after cancelling");
        spinner.abort();
        // total_timeout bounds retries and the burst wait.
        mock.reset();
        mock.always(Scripted::from_fixture(&burst));
        let c = Client::new(
            Config::builder(KIT_API_KEY)
                .base_url(&mock.base_url)
                .http_client(test_http_client())
                .total_timeout(Duration::from_secs(3))
                .build()
                .unwrap(),
        )
        .unwrap();
        let spinner = spawn_spinner();
        let task = tokio::spawn(async move { c.articles().search(&SearchParams::new()).await });
        wait_for("first attempt", || mock.count() == 1).await;
        settle().await;
        tokio::time::advance(Duration::from_secs(3)).await;
        settle().await;
        assert!(task.is_finished(), "total_timeout ends the burst wait");
        let err = task.await.unwrap().unwrap_err();
        assert_eq!(err.kind(), "timeout");
        assert_eq!(mock.count(), 1);
        spinner.abort();
    });
}

#[test]
fn network_errors_are_retried_and_wrapped() {
    run_paused(async {
        // Nothing listens on this port: connection refused, retried 3 times, then a network error.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let c = client(&format!("http://127.0.0.1:{port}"));
        let err = c.articles().search(&SearchParams::new()).await.unwrap_err();
        assert_eq!(err.kind(), "network");
        assert!(
            std::error::Error::source(&err).is_some(),
            "the cause stays reachable"
        );
    });
}

#[test]
fn companies_get_keeps_no_cache() {
    let ok = load("fixtures/requests/companies-get.json");
    let not_modified = load("fixtures/requests/companies-get-not-modified.json");
    run_paused(async move {
        let mock = HttpMock::start().await;
        let c = client(&mock.base_url);
        // A record from get_if_none_match is not remembered: get() still sends no If-None-Match.
        mock.enqueue(Scripted::from_fixture(&ok["response"]));
        mock.enqueue(Scripted::from_fixture(&ok["response"]));
        let fresh = c
            .companies()
            .get_if_none_match(900001, "\"stale\"")
            .await
            .unwrap()
            .unwrap();
        let etag = fresh.cache_info().unwrap().etag.clone();
        assert_eq!(etag.as_deref(), ok["response"]["headers"]["etag"].as_str());
        c.companies().get(900001).await.unwrap();
        let reqs = mock.requests();
        assert_eq!(reqs[0].header("if-none-match"), Some("\"stale\""));
        assert_eq!(reqs[1].header("if-none-match"), None);
        // A 304 that get() never asked for is a decode error, not an empty record.
        mock.enqueue(Scripted::from_fixture(&not_modified["response"]));
        let err = c.companies().get(900001).await.unwrap_err();
        assert_eq!(err.kind(), "decode");
        assert!(c.companies().get_if_none_match(900001, "").await.is_err());
    });
}

// ---- R-CONFIG-1 -----------------------------------------------------------------------

#[test]
fn config_defaults_and_overrides() {
    let c = Config::new("k");
    assert_eq!(c.base_url, "https://api.finlight.me");
    assert_eq!(c.wss_url, "wss://wss.finlight.me");
    assert_eq!(c.timeout, Duration::from_secs(30));
    assert_eq!(c.total_timeout, None);
    assert_eq!(c.max_retries, 3);
    assert_eq!(c.retry_base_delay, Duration::from_millis(500));
    assert_eq!(c.retry_max_delay, Duration::from_secs(8));
    assert_eq!(c.retry_on_burst, RetryOnBurst::Once);
    assert_eq!(c.burst_wait, Duration::from_secs(10));
    assert_eq!(c.max_pages, 10);
    assert_eq!(c.max_response_bytes, 32 * 1024 * 1024);
    assert_eq!(c.ping_interval, Duration::from_secs(25));
    assert_eq!(c.watchdog_timeout, Duration::from_secs(60));
    assert_eq!(c.rotation_interval, Duration::from_secs(115 * 60));
    assert_eq!(c.reconnect_base_delay, Duration::from_secs(1));
    assert_eq!(c.reconnect_max_delay, Duration::from_secs(30));
    assert_eq!(c.long_backoff, Duration::from_secs(60));
    assert!(c.quota_reconnect);
    assert_eq!(c.quota_reconnect_interval, Duration::from_secs(3600));
    assert_eq!(c.search_failed_threshold, 5);
    assert_eq!(c.dedupe_cache_size, 1000);
    assert_eq!(c.max_message_bytes, 32 * 1024 * 1024);
    assert_eq!(c.connect_timeout, Duration::from_secs(30));
    assert_eq!(DEFAULT_TOLERANCE, Duration::from_secs(300));
    assert_eq!(PAYLOAD_VERSION, 3);
    assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
    assert!(matches!(
        Client::new(Config::new("")),
        Err(Error::Validation(_))
    ));
    assert!(matches!(
        Client::new(Config::new("   ")),
        Err(Error::Validation(_))
    ));
    assert!(Config::builder("k").base_url("ftp://x").build().is_err());
    assert!(Config::builder("k").wss_url("https://x").build().is_err());
    assert!(
        Config::builder("k")
            .max_message_bytes(1024)
            .build()
            .is_err()
    );
}

#[test]
fn config_trailing_slash_and_headers() {
    run_paused(async {
        let mock = HttpMock::start().await;
        mock.always(Scripted::json(200, &json!({"sources": []})));
        let c = Client::new(
            Config::builder(KIT_API_KEY)
                .base_url(format!("{}/", mock.base_url))
                .http_client(test_http_client())
                .build()
                .unwrap(),
        )
        .unwrap();
        c.sources().list().await.unwrap();
        let req = &mock.requests()[0];
        assert_eq!(req.path, "/v3/sources");
        assert_eq!(
            req.header("user-agent").unwrap(),
            format!("finlight-client-rust/{VERSION}")
        );
        assert_eq!(req.header("x-client-version"), Some(VERSION));
        assert_eq!(req.header("x-api-key"), Some(KIT_API_KEY));
        assert!(!req.head.contains("apiKey="));
    });
    run_paused(async {
        let ws = WsMock::start().await;
        let spinner = spawn_spinner();
        let c = Client::new(
            Config::builder(KIT_API_KEY)
                .wss_url(format!("{}/", ws.url))
                .build()
                .unwrap(),
        )
        .unwrap();
        let mut s = c
            .raw_stream(RawStreamParams::new(), StreamOptions::new())
            .await
            .unwrap();
        let dial = ws.wait_dial(0).await;
        assert_eq!(dial.path, "/raw");
        assert_eq!(
            dial.header("user-agent").unwrap(),
            format!("finlight-client-rust/{VERSION}")
        );
        assert_eq!(dial.header("x-client-version"), Some(VERSION));
        assert!(
            dial.header("x-takeover").is_none(),
            "takeover is off by default"
        );
        s.close().await;
        spinner.abort();
    });
}

// ---- R-LOG-1 --------------------------------------------------------------------------

#[derive(Clone, Default)]
struct Buf(Arc<Mutex<Vec<u8>>>);

impl Write for Buf {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn logging_never_shows_secrets_and_installs_nothing_global() {
    const KEY: &str = "sk_test_never_log_this_key_0123456789";
    const SECRET: &str = "whsec_never_log_this_secret";
    let _ = tracing_log::LogTracer::init();
    let buf = Buf::default();
    let writer = buf.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .finish();
    let mut texts = Vec::new();
    tracing::subscriber::with_default(subscriber, || {
        run_paused(async {
            let mock = HttpMock::start().await;
            let unauthorized =
                load("fixtures/errors/unauthorized-invalid-key.json")["response"].clone();
            mock.always(Scripted::from_fixture(&unauthorized));
            let config = Config::builder(KEY)
                .base_url(&mock.base_url)
                .http_client(test_http_client())
                .build()
                .unwrap();
            texts.push(format!("{config:?}"));
            let c = Client::new(config).unwrap();
            texts.push(format!("{c:?}"));
            let err = c.articles().search(&SearchParams::new()).await.unwrap_err();
            texts.push(format!("{err} {err:?}"));

            let ws = WsMock::start().await;
            ws.script(DialScript::Refuse {
                status: 401,
                body: Some(r#"{"message":"Invalid API key"}"#.into()),
            });
            let spinner = spawn_spinner();
            let c = Client::new(
                Config::builder(KEY)
                    .wss_url(ws.url.clone())
                    .build()
                    .unwrap(),
            )
            .unwrap();
            let mut s = c
                .stream(StreamParams::new(), StreamOptions::new())
                .await
                .unwrap();
            let item = s.next().await.unwrap();
            let err = item.unwrap_err();
            texts.push(format!("{err} {err:?} {s:?}"));

            // A streamed article and the subscribe frame at TRACE. The server here writes raw
            // bytes, so any frame log would come from the SDK side.
            let mut article = article_json();
            article["title"] = json!(ARTICLE_MARKER);
            let frames = vec![
                json!({"event": "connection.admitted", "payloadVersion": 3,
                    "data": {"leaseId": "l", "serverNow": 1763932800123_i64, "clientNonce": null}}),
                json!({"event": "article.created", "payloadVersion": 3, "data": article}),
            ];
            let url = raw_ws_server(frames).await;
            let c = Client::new(Config::builder(KEY).wss_url(url).build().unwrap()).unwrap();
            let mut params = StreamParams::new();
            params.query = Some(QUERY_MARKER.to_owned());
            let mut s = c.stream(params, StreamOptions::new()).await.unwrap();
            let event = s.next().await.unwrap().unwrap();
            assert_eq!(event.data.unwrap().title, ARTICLE_MARKER);
            s.close().await;
            spinner.abort();

            let body = r#"{"payloadVersion":3}"#;
            let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(SECRET.as_bytes()).unwrap();
            mac.update(b"2026-11-23T21:04:05.123Z.x");
            let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
            let err = construct_event_from_values(
                body,
                Some(&sig),
                Some("2026-11-23T21:04:05.123Z"),
                SECRET,
                &WebhookVerifyOptions::new(),
            )
            .unwrap_err();
            texts.push(format!("{err} {err:?}"));
            assert!(!texts.last().unwrap().contains(&sig[7..]));
        });
    });
    let log = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    assert!(!log.is_empty(), "debug logging was on");
    for text in texts.iter().chain(std::iter::once(&log)) {
        assert!(!text.contains(KEY), "the API key leaked: {text}");
        assert!(!text.contains(SECRET), "the secret leaked: {text}");
    }
    // No frame payload in the log, as text or as hex, and no transport library logging frames.
    for marker in [ARTICLE_MARKER, QUERY_MARKER] {
        assert!(!log.contains(marker), "a frame body was logged: {log}");
        assert!(
            !log.contains(&hex::encode(marker)),
            "a frame body was logged as hex: {log}"
        );
    }
    assert!(
        !log.contains("tungstenite"),
        "the WebSocket transport logged: {log}"
    );
}

const ARTICLE_MARKER: &str = "ZEBRA payload marker 4711";
const QUERY_MARKER: &str = "QUAGGA_query_marker_0815";

/// A one-connection WebSocket server that writes raw frames (no WebSocket library), then
/// keeps the socket open until the client closes it. Returns its ws:// URL.
async fn raw_ws_server(frames: Vec<Value>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            sock.read_exact(&mut byte).await.unwrap();
            head.push(byte[0]);
        }
        let head = String::from_utf8(head).unwrap();
        let key = head
            .lines()
            .find_map(|l| l.strip_prefix("sec-websocket-key: "))
            .unwrap()
            .trim()
            .to_owned();
        let accept = tokio_tungstenite::tungstenite::handshake::derive_accept_key(key.as_bytes());
        let mut out = format!(
            "HTTP/1.1 101 Switching Protocols\r\nupgrade: websocket\r\nconnection: Upgrade\r\nsec-websocket-accept: {accept}\r\n\r\n"
        )
        .into_bytes();
        for frame in frames {
            let payload = frame.to_string().into_bytes();
            out.push(0x81);
            if payload.len() < 126 {
                out.push(payload.len() as u8);
            } else if payload.len() <= 0xffff {
                out.push(126);
                out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
            } else {
                out.push(127);
                out.extend_from_slice(&(payload.len() as u64).to_be_bytes());
            }
            out.extend_from_slice(&payload);
        }
        sock.write_all(&out).await.unwrap();
        let mut sink = [0u8; 1024];
        while sock.read(&mut sink).await.unwrap_or(0) > 0 {}
    });
    format!("ws://{addr}")
}

// ---- R-WS-6, R-WS-2 extras -------------------------------------------------------------

#[test]
fn stream_hook_panic_does_not_end_the_stream_and_close_cleans_up() {
    run_paused(async {
        let ws = WsMock::start().await;
        let spinner = spawn_spinner();
        let c = Client::new(
            Config::builder(KIT_API_KEY)
                .wss_url(ws.url.clone())
                .build()
                .unwrap(),
        )
        .unwrap();
        let options = StreamOptions::new().on_admitted(|_| panic!("user code fails"));
        let mut s = c.stream(StreamParams::new(), options).await.unwrap();
        let conn = ws.wait_dial(0).await.conn.unwrap();
        conn.next("subscribe").await;
        conn.send(&json!({"event": "connection.admitted", "payloadVersion": 3,
            "data": {"leaseId": "l", "serverNow": 1763932800123_i64, "clientNonce": null}}));
        conn.send(
            &json!({"event": "article.created", "payloadVersion": 3, "data": article_json()}),
        );
        let item = s.next().await.unwrap().unwrap();
        assert_eq!(item.kind, StreamEventKind::Created);
        assert!(!s.closed());
        s.close().await;
        assert!(s.closed());
        assert!(s.next().await.is_none());
        wait_for("close 1000", || conn.closed_code().is_some()).await;
        assert_eq!(conn.closed_code(), Some(Some(1000)));
        spinner.abort();
    });
}

#[test]
fn stream_drop_closes_with_1000_and_policy_close_is_terminal() {
    run_paused(async {
        let ws = WsMock::start().await;
        let spinner = spawn_spinner();
        let c = Client::new(
            Config::builder(KIT_API_KEY)
                .wss_url(ws.url.clone())
                .build()
                .unwrap(),
        )
        .unwrap();
        let s = c
            .stream(StreamParams::new(), StreamOptions::new())
            .await
            .unwrap();
        let conn = ws.wait_dial(0).await.conn.unwrap();
        conn.next("subscribe").await;
        drop(s);
        wait_for("close after drop", || conn.closed_code().is_some()).await;
        assert_eq!(conn.closed_code(), Some(Some(1000)));

        let mut s = c
            .stream(StreamParams::new(), StreamOptions::new())
            .await
            .unwrap();
        let conn = ws.wait_dial(1).await.conn.unwrap();
        conn.close(1008);
        let err = s.next().await.unwrap().unwrap_err();
        assert_eq!(err.kind(), "api");
        assert_eq!(err.as_api().unwrap().status(), 1008);
        assert!(s.next().await.is_none());
        settle().await;
        assert_eq!(ws.dial_count(), 2, "no reconnect after 1008");
        spinner.abort();
    });
}

#[test]
fn stream_refusal_without_body_falls_back_to_status() {
    run_paused(async {
        let ws = WsMock::start().await;
        let spinner = spawn_spinner();
        ws.script(DialScript::Refuse {
            status: 403,
            body: None,
        });
        let errors = Arc::new(Mutex::new(Vec::new()));
        let e2 = errors.clone();
        let c = Client::new(
            Config::builder(KIT_API_KEY)
                .wss_url(ws.url.clone())
                .build()
                .unwrap(),
        )
        .unwrap();
        let mut s = c
            .stream(
                StreamParams::new(),
                StreamOptions::new().on_error(move |e| e2.lock().unwrap().push(e.kind())),
            )
            .await
            .unwrap();
        ws.wait_dial(0).await;
        settle().await;
        assert_eq!(
            errors.lock().unwrap().as_slice(),
            ["api"],
            "403 without a body is reported"
        );
        common::advance(59_000).await;
        assert_eq!(ws.dial_count(), 1, "long wait");
        common::advance(13_000).await;
        assert_eq!(ws.dial_count(), 2);
        s.close().await;

        ws.script(DialScript::Refuse {
            status: 401,
            body: None,
        });
        let mut s = c
            .stream(StreamParams::new(), StreamOptions::new())
            .await
            .unwrap();
        let err = s.next().await.unwrap().unwrap_err();
        assert_eq!(
            err.as_api().unwrap().api_kind(),
            ApiErrorKind::Unauthorized,
            "401 is terminal"
        );
        spinner.abort();
    });
}

#[test]
fn stream_refusal_body_chunked_or_endless_is_read_in_time() {
    run_paused(async {
        let ws = WsMock::start().await;
        let spinner = spawn_spinner();
        let c = Client::new(
            Config::builder(KIT_API_KEY)
                .wss_url(ws.url.clone())
                .build()
                .unwrap(),
        )
        .unwrap();

        // Chunked body on a kept-alive connection: read in full, so the refusal is terminal.
        let body = r#"{"message":"Your subscription does not include WebSockets"}"#;
        ws.script(DialScript::Raw(format!(
            "HTTP/1.1 403 Forbidden\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\n\
             connection: keep-alive\r\n\r\n{:x}\r\n{}\r\n{:x}\r\n{}\r\n0\r\n\r\n",
            20,
            &body[..20],
            body.len() - 20,
            &body[20..]
        )));
        let mut s = c
            .stream(StreamParams::new(), StreamOptions::new())
            .await
            .unwrap();
        let err = s.next().await.unwrap().unwrap_err();
        assert_eq!(err.as_api().unwrap().status(), 403);
        assert!(
            err.to_string().contains("does not include WebSockets"),
            "{err}"
        );
        assert!(s.next().await.is_none(), "terminal");
        assert_eq!(ws.dial_count(), 1);

        // A body that never ends counts as missing after 5 s: 403 falls back to the long wait.
        ws.script(DialScript::Raw(
            "HTTP/1.1 403 Forbidden\r\nconnection: keep-alive\r\n\r\n{\"message\":".to_owned(),
        ));
        let errors = Arc::new(Mutex::new(Vec::new()));
        let e2 = errors.clone();
        let mut s = c
            .stream(
                StreamParams::new(),
                StreamOptions::new().on_error(move |e| e2.lock().unwrap().push(e.to_string())),
            )
            .await
            .unwrap();
        ws.wait_dial(1).await;
        common::advance(7_000).await;
        wait_for("the refusal is reported", || {
            !errors.lock().unwrap().is_empty()
        })
        .await;
        assert!(
            errors.lock().unwrap()[0].contains("HTTP 403"),
            "{:?}",
            errors.lock().unwrap()
        );
        assert!(!s.closed());
        common::advance(50_000).await;
        assert_eq!(ws.dial_count(), 2, "long wait, not a timeout backoff");
        s.close().await;
        spinner.abort();
    });
}

#[test]
fn stream_rotation_refused_redials_at_once() {
    run_paused(async {
        let ws = WsMock::start().await;
        let spinner = spawn_spinner();
        let too_many =
            load("ws-sessions/dial-too-many-connections.json")["steps"][0]["server"]["body"]
                .as_str()
                .unwrap()
                .to_owned();
        ws.script(DialScript::Accept);
        // The renewal dial meets the old connection at the plan's limit of 1.
        ws.script(DialScript::Refuse {
            status: 403,
            body: Some(too_many.clone()),
        });
        ws.script(DialScript::Accept);
        let errors = Arc::new(Mutex::new(Vec::new()));
        let reconnects = Arc::new(Mutex::new(Vec::new()));
        let (e2, r2) = (errors.clone(), reconnects.clone());
        let c = Client::new(
            Config::builder(KIT_API_KEY)
                .wss_url(ws.url.clone())
                .build()
                .unwrap(),
        )
        .unwrap();
        let options = StreamOptions::new()
            .on_error(move |e| e2.lock().unwrap().push(e.to_string()))
            .on_reconnect(move |r| r2.lock().unwrap().push((r.reason.clone(), r.connection)));
        let mut s = c.stream(StreamParams::new(), options).await.unwrap();
        let first = ws.wait_dial(0).await.conn.unwrap();
        first.next("subscribe").await;
        first.send(&json!({"event": "connection.admitted", "payloadVersion": 3,
            "data": {"leaseId": "l1", "serverNow": 1763932800123_i64, "clientNonce": null}}));
        common::advance(115 * 60 * 1000).await;
        ws.wait_dial(1).await;
        // No clock movement: the next dial follows at once.
        let third = ws.wait_dial(2).await.conn.expect("accepted");
        third.next("re-subscribe").await;
        assert_eq!(
            first.closed_code(),
            Some(Some(1000)),
            "the old connection was closed"
        );
        assert!(
            errors.lock().unwrap().is_empty(),
            "the expected refusal is not an error"
        );
        wait_for("on_reconnect", || !reconnects.lock().unwrap().is_empty()).await;
        assert_eq!(
            reconnects.lock().unwrap().as_slice(),
            [(ReconnectReason::Rotation, 2)],
            "refused dials are not counted as connections"
        );
        assert!(!s.closed());
        s.close().await;

        // When the immediate redial is refused as well, the long wait applies.
        ws.script(DialScript::Accept);
        ws.script(DialScript::Refuse {
            status: 403,
            body: Some(too_many.clone()),
        });
        ws.script(DialScript::Refuse {
            status: 403,
            body: Some(too_many),
        });
        let errors = Arc::new(Mutex::new(Vec::new()));
        let e2 = errors.clone();
        let mut s = c
            .stream(
                StreamParams::new(),
                StreamOptions::new().on_error(move |e| e2.lock().unwrap().push(e.to_string())),
            )
            .await
            .unwrap();
        let conn = ws.wait_dial(3).await.conn.unwrap();
        conn.next("subscribe").await;
        common::advance(115 * 60 * 1000).await;
        ws.wait_dial(5).await;
        wait_for("the second refusal is reported", || {
            errors.lock().unwrap().len() == 1
        })
        .await;
        common::advance(59_000).await;
        assert_eq!(ws.dial_count(), 6, "long wait after the second refusal");
        common::advance(13_000).await;
        assert_eq!(ws.dial_count(), 7);
        s.close().await;
        spinner.abort();
    });
}

// ---- R-HOOK-1, R-HOOK-2 ----------------------------------------------------------------

fn signed(body: &str, secret: &str, ts: &str) -> String {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(ts.as_bytes());
    mac.update(b".");
    mac.update(body.as_bytes());
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

#[test]
fn webhooks_reject_bad_headers_and_read_versions() {
    let ts = "2026-11-23T21:04:05.123Z";
    let now = UNIX_EPOCH + Duration::from_millis(1_795_467_845_123);
    let opts = WebhookVerifyOptions::new().now(now);
    let reason = |r: FinlightResult<WebhookPayload>| match r {
        Err(Error::WebhookVerification(v)) => v.reason().as_str(),
        other => panic!("expected a verification error, got {other:?}"),
    };
    let body = r#"{"payloadVersion":3}"#;
    let sig = signed(body, "s", ts);
    // Duplicate header, non-ASCII header, empty secret.
    let dup = vec![
        ("X-Webhook-Signature", sig.as_str()),
        ("x-webhook-signature", sig.as_str()),
        ("X-Webhook-Timestamp", ts),
    ];
    assert_eq!(
        reason(construct_event(body, &dup, "s", &opts)),
        "duplicate_header"
    );
    let joined = format!("{sig}, {sig}");
    let dup2 = [
        ("X-Webhook-Signature", joined.as_str()),
        ("X-Webhook-Timestamp", ts),
    ];
    assert_eq!(
        reason(construct_event(body, &dup2, "s", &opts)),
        "duplicate_header"
    );
    let non_ascii = [
        ("X-Webhook-Signature", sig.as_str()),
        ("X-Webhook-Timestamp", "2026-11-23T21:04:05.123Ž"),
    ];
    assert_eq!(
        reason(construct_event(body, &non_ascii, "s", &opts)),
        "non_ascii_header"
    );
    let ok = [
        ("X-Webhook-Signature", sig.as_str()),
        ("X-Webhook-Timestamp", ts),
    ];
    assert_eq!(
        reason(construct_event(body, &ok, "", &opts)),
        "invalid_secret"
    );
    // CGI-style names (Rack, PHP) and http::HeaderMap.
    let cgi: BTreeMap<&str, &str> = [
        ("HTTP_X_WEBHOOK_SIGNATURE", sig.as_str()),
        ("HTTP_X_WEBHOOK_TIMESTAMP", ts),
    ]
    .into();
    assert!(matches!(
        construct_event(body, &cgi, "s", &opts),
        Err(Error::Decode(_)) // signed, but not a v3 envelope
    ));
    let mut map = http::HeaderMap::new();
    map.insert("x-webhook-signature", sig.parse().unwrap());
    map.insert("x-webhook-timestamp", ts.parse().unwrap());
    assert!(matches!(
        construct_event(body, &map, "s", &opts),
        Err(Error::Decode(_))
    ));
    map.insert(
        "x-webhook-timestamp",
        http::HeaderValue::from_bytes(b"2026\xff").unwrap(),
    );
    assert_eq!(
        reason(construct_event(body, &map, "s", &opts)),
        "non_ascii_header"
    );

    // payloadVersion: "3" (string) is not v3; 3.0 is.
    for (pv, v3) in [
        (r#""3""#, false),
        ("3.0", true),
        ("true", false),
        ("2", false),
    ] {
        let body = format!(r#"{{"payloadVersion":{pv},"link":"https://x.example/a"}}"#);
        let sig = signed(&body, "s", ts);
        let result = construct_event_from_values(body.as_bytes(), Some(&sig), Some(ts), "s", &opts);
        if v3 {
            assert!(
                matches!(result, Err(Error::Decode(_))),
                "3.0 counts as v3 (and this body is no envelope)"
            );
        } else {
            let Ok(WebhookPayload::WrongPayloadVersion(w)) = result else {
                panic!("{pv}: expected WrongPayloadVersion")
            };
            assert_eq!(w.detected_version, 2);
            assert_eq!(w.payload_version.as_deref(), Some(pv));
            assert_eq!(w.link.as_deref(), Some("https://x.example/a"));
        }
    }
    // `raw` is the received text: key order and big integers stay as they were.
    let body =
        r#"{"payloadVersion":2,"link":"https://x.example/a","n":12345678901234567890123,"a":1}"#;
    let sig = signed(body, "s", ts);
    let Ok(WebhookPayload::WrongPayloadVersion(w)) =
        construct_event_from_values(body, Some(&sig), Some(ts), "s", &opts)
    else {
        panic!("expected WrongPayloadVersion")
    };
    assert_eq!(w.raw, body);
    assert_eq!(w.raw_json(), body);
    // A verified body that is not a JSON object.
    let body = "[1,2]";
    let sig = signed(body, "s", ts);
    assert!(matches!(
        construct_event_from_values(body, Some(&sig), Some(ts), "s", &opts),
        Err(Error::Decode(_))
    ));
    assert!(verify_signature(body, &sig, ts, "s", &opts).is_ok());
    // Offsets are accepted; a zone-less time is not.
    let off = "2026-11-23T22:04:05.123+01:00";
    let sig = signed(body, "s", off);
    assert!(verify_signature(body, &sig, off, "s", &opts).is_ok());
    let naive = "2026-11-23T21:04:05.123";
    let sig = signed(body, "s", naive);
    assert!(verify_signature(body, &sig, naive, "s", &opts).is_err());
    let _ = SystemTime::now();
}
