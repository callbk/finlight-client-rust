//! V-01, D-01 .. D-11, F-01 .. F-03 (REST and webhook parts) through the public API.

use std::time::UNIX_EPOCH;

use finlight::{
    Article, Client, CompanyRole, Config, ConnectionErrorCode, EventDirection, EventType, Field,
    GetArticleOptions, Importance, IncludeBlock, OrderBy, PAYLOAD_VERSION, PreemptReason,
    ProblemCode, RateLimitType, SearchParams, SentimentLabel, SortOrder, SourceType,
    StreamEventName, WebhookEventName,
};
use serde_json::{Value, json};

use crate::common::http_mock::{HttpMock, Scripted};
use crate::common::{KIT_API_KEY, load, run_paused, test_http_client};

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_owned())
        .collect()
}

fn consts(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

fn v01() {
    let vocab = load("vocab.json");
    assert_eq!(vocab["payloadVersion"], json!(PAYLOAD_VERSION));
    let req = &vocab["request"];
    let resp = &vocab["response"];
    let ws = &vocab["websocket"];
    assert_eq!(
        consts(IncludeBlock::ALL),
        strings(&req["include"]["values"])
    );
    assert_eq!(consts(OrderBy::ALL), strings(&req["orderBy"]["values"]));
    assert_eq!(consts(SortOrder::ALL), strings(&req["order"]["values"]));
    assert_eq!(consts(CompanyRole::ALL), strings(&req["roles"]["values"]));
    assert_eq!(consts(CompanyRole::ALL), strings(&resp["roles"]["values"]));
    assert_eq!(
        consts(EventType::ALL),
        strings(&req["eventTypes"]["values"])
    );
    assert_eq!(
        consts(EventType::ALL),
        strings(&resp["eventTypes"]["values"])
    );
    assert_eq!(
        consts(EventDirection::ALL),
        strings(&req["eventDirections"]["values"])
    );
    assert_eq!(
        consts(EventDirection::ALL),
        strings(&resp["eventDirections"]["values"])
    );
    assert_eq!(
        consts(Importance::ALL),
        strings(&req["minImportance"]["values"])
    );
    assert_eq!(
        consts(Importance::ALL),
        strings(&resp["importance"]["values"])
    );
    assert_eq!(
        consts(SentimentLabel::ALL),
        strings(&resp["sentimentLabels"]["values"])
    );
    assert_eq!(
        consts(SourceType::ALL),
        strings(&resp["sourceTypes"]["values"])
    );
    assert_eq!(
        consts(StreamEventName::ALL),
        strings(&ws["events"]["values"])
    );
    assert_eq!(
        consts(StreamEventName::ARTICLE_EVENTS),
        strings(&ws["articleEvents"]["values"])
    );
    assert_eq!(
        consts(StreamEventName::CONNECTION_EVENTS),
        strings(&ws["connectionEvents"]["values"])
    );
    assert_eq!(
        consts(ConnectionErrorCode::ALL),
        strings(&ws["connectionErrorCodes"]["values"])
    );
    assert_eq!(
        consts(PreemptReason::ALL),
        strings(&ws["preemptReasons"]["values"])
    );
    assert_eq!(
        consts(WebhookEventName::ALL),
        strings(&vocab["webhook"]["events"]["values"])
    );
    let codes: Vec<String> = vocab["problemCodes"]["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["code"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(consts(ProblemCode::ALL), codes);
    let mut limits = strings(&vocab["limitTypes"]["values"]);
    limits.push(RateLimitType::UNKNOWN.to_owned());
    assert_eq!(consts(RateLimitType::ALL), limits);
    assert_eq!(ws["heartbeatMs"], 25000);
    assert_eq!(ws["staleMs"], 60000);

    // Every type stays open: an unknown value is kept as received and written back unchanged.
    let role = CompanyRole::from("guarantor");
    assert!(!role.is_known());
    assert_eq!(role.as_str(), "guarantor");
    assert_eq!(serde_json::to_value(&role).unwrap(), json!("guarantor"));
    let back: CompanyRole = serde_json::from_value(json!("guarantor")).unwrap();
    assert_eq!(back, CompanyRole::Other("guarantor".into()));
    let known: CompanyRole = serde_json::from_value(json!("acquirer")).unwrap();
    assert_eq!(known, CompanyRole::Acquirer);
    assert!(known == CompanyRole::ACQUIRER);
    // Exact comparison: case is not folded.
    assert!(!CompanyRole::from("Acquirer").is_known());
}

fn decode(wire: &Value) -> Article {
    let page = json!({"articles": [wire], "pagination": {"limit": 1, "returned": 1, "hasMore": false, "nextCursor": null}});
    let mut articles = decode_page(&page);
    assert_eq!(articles.len(), 1, "the article decodes");
    articles.remove(0)
}

/// Decodes a search page through the public API (served by the local HTTP server).
pub fn decode_page(page: &Value) -> Vec<Article> {
    let mut out = None;
    let page = page.clone();
    run_paused(async {
        let mock = HttpMock::start().await;
        mock.enqueue(Scripted::json(200, &page));
        let client = client(&mock.base_url);
        let result = client
            .articles()
            .search(&SearchParams::new())
            .await
            .unwrap();
        assert!(
            result.skipped.is_empty(),
            "nothing skipped: {:?}",
            result.skipped
        );
        out = Some(result.articles);
    });
    out.unwrap()
}

pub fn client(base_url: &str) -> Client {
    let config = Config::builder(KIT_API_KEY)
        .base_url(base_url)
        .http_client(test_http_client())
        .build()
        .unwrap();
    Client::new(config).unwrap()
}

fn opt<T>(v: &Option<T>) -> &'static str {
    if v.is_some() { "value" } else { "null" }
}

fn list<T>(v: &[T]) -> &'static str {
    if v.is_empty() { "empty" } else { "value" }
}

fn gated<T>(v: &Option<Vec<T>>) -> &'static str {
    match v {
        None => "absent",
        Some(l) if l.is_empty() => "empty",
        Some(_) => "value",
    }
}

fn field<T>(v: &Field<T>) -> &'static str {
    match v {
        Field::Absent => "absent",
        Field::Null => "null",
        Field::Value(_) => "value",
    }
}

fn index(part: &str) -> (&str, Option<usize>) {
    match part.split_once('[') {
        Some((name, rest)) => (name, rest.trim_end_matches(']').parse().ok()),
        None => (part, None),
    }
}

/// The presence state of a key path in the decoded model: absent, null, empty or value.
fn presence(a: &Article, path: &str) -> &'static str {
    let parts: Vec<&str> = path.split('.').collect();
    match parts.as_slice() {
        ["id"] | ["link"] | ["title"] | ["source"] | ["source", "domain"] => "value",
        ["source", "type"] => opt(&a.source.r#type),
        ["summary"] => opt(&a.summary),
        ["language"] => opt(&a.language),
        ["publishedAt"] => opt(&a.published_at),
        ["firstSeenAt"] => opt(&a.first_seen_at),
        ["revisedAt"] => opt(&a.revised_at),
        ["availableAt"] => opt(&a.available_at),
        ["media"] => list(&a.media),
        ["countries"] => list(&a.countries),
        ["categories"] => list(&a.categories),
        ["content"] => field(&a.content),
        ["sentiment"] => field(&a.sentiment),
        ["sentiment", "label"] => opt(&a.sentiment.value().unwrap().label),
        ["sentiment", "confidence"] => opt(&a.sentiment.value().unwrap().confidence),
        ["companies"] => gated(&a.companies),
        ["events"] => gated(&a.events),
        [head, rest @ ..] => {
            let (name, i) = index(head);
            let i = i.unwrap();
            match name {
                "companies" => {
                    let c = &a.companies.as_ref().unwrap()[i];
                    match rest {
                        ["companyId"] | ["name"] => "value",
                        ["isin"] => opt(&c.isin),
                        ["primaryListing"] => opt(&c.primary_listing),
                        ["primaryListing", "ticker"] => {
                            opt(&c.primary_listing.as_ref().unwrap().ticker)
                        }
                        ["primaryListing", "mic"] => opt(&c.primary_listing.as_ref().unwrap().mic),
                        ["primaryListing", "country"] => {
                            opt(&c.primary_listing.as_ref().unwrap().country)
                        }
                        ["confidence"] => opt(&c.confidence),
                        ["role"] => opt(&c.role),
                        ["relevance"] => opt(&c.relevance),
                        other => panic!("no presence mapping for companies[].{other:?}"),
                    }
                }
                "events" => {
                    let e = &a.events.as_ref().unwrap()[i];
                    match rest {
                        ["type"] => "value",
                        ["direction"] => opt(&e.direction),
                        ["importance"] => opt(&e.importance),
                        ["confidence"] => opt(&e.confidence),
                        ["companyIds"] => gated(&e.company_ids),
                        other => panic!("no presence mapping for events[].{other:?}"),
                    }
                }
                other => panic!("no presence mapping for {other}"),
            }
        }
        _ => panic!("empty path"),
    }
}

/// Checks the presence view and that writing the model back gives the wire JSON.
fn check_presence(a: &Article, expect: &Value, wire: &Value, case: &str) {
    for (path, want) in expect.as_object().unwrap() {
        assert_eq!(
            presence(a, path),
            want.as_str().unwrap(),
            "{case}: presence of {path}"
        );
    }
    assert_eq!(
        &serde_json::to_value(a).unwrap(),
        wire,
        "{case}: written back unchanged"
    );
    assert_eq!(
        serde_json::from_str::<Value>(a.raw_json().unwrap()).unwrap(),
        *wire
    );
}

fn decode_case(file: &'static str) -> impl FnOnce() {
    move || {
        let f = load(&format!("fixtures/decode/{file}"));
        let a = decode(&f["wire"]);
        let id = f["id"].as_str().unwrap();
        check_presence(&a, &f["expect"]["presence"], &f["wire"], id);
        match id {
            "D-01" => {
                assert!(a.media.len() <= 3);
                assert!(a.revised_at.is_some() && a.revised_at_time().is_some());
                let companies = a.companies.as_ref().unwrap();
                assert!(companies.iter().any(|c| c.primary_listing.is_none()));
                assert!(a.published_at_time().is_some());
            }
            "D-02" => {
                assert!(a.content.is_absent() && a.sentiment.is_absent());
                assert!(!a.companies_present() && !a.events_present());
                assert!(a.available_at.is_none());
            }
            "D-04" => {
                assert!(!a.content_present() && !a.sentiment_present());
                assert!(!a.companies_present() && !a.events_present());
            }
            "D-05" | "D-06" => {
                for e in a.events.as_ref().unwrap() {
                    assert!(e.company_ids.is_none(), "companyIds absent, never []");
                }
            }
            "D-07" => {
                assert_eq!(a.published_at.as_deref(), Some("1970-01-01T00:00:00.000Z"));
                assert_eq!(a.published_at_time(), Some(UNIX_EPOCH));
                assert!(a.summary.is_none() && a.first_seen_at.is_none() && a.sentiment.is_null());
                assert!(a.content.is_null());
                let c = &a.companies.as_ref().unwrap()[0];
                assert!(c.role.is_none() && c.relevance.is_none());
                assert!(c.primary_listing.is_some());
                assert_eq!(a.events.as_ref().map(Vec::len), Some(0));
            }
            "D-08" => {
                assert_eq!(a.title, f["wire"]["title"].as_str().unwrap());
                assert_eq!(a.summary.as_deref(), f["wire"]["summary"].as_str());
            }
            "D-09" => {
                let s = a.sentiment.value().unwrap();
                assert!(s.label.is_none() && s.confidence.is_some());
                assert_eq!(a.companies.as_ref().map(Vec::len), Some(0));
            }
            "D-11" => {
                assert!(a.summary.is_none());
                assert!(a.content.is_null());
                assert!(!a.title.is_empty());
            }
            _ => {}
        }
    }
}

fn d10() {
    let f = load("fixtures/decode/matrix.json");
    let cases = f["cases"].as_array().unwrap();
    assert_eq!(cases.len() as u64, f["count"].as_u64().unwrap());
    let page = json!({
        "articles": cases.iter().map(|c| c["wire"].clone()).collect::<Vec<_>>(),
        "pagination": {"limit": 100, "returned": cases.len(), "hasMore": false, "nextCursor": null},
    });
    let articles = decode_page(&page);
    assert_eq!(articles.len(), cases.len());
    for (a, c) in articles.iter().zip(cases) {
        let id = c["id"].as_str().unwrap();
        check_presence(a, &c["expect"]["presence"], &c["wire"], id);
        if c["channel"] == "raw" {
            assert!(a.available_at.is_none(), "{id}: raw availableAt is null");
        }
    }
}

fn f01() {
    let f = load("fixtures/forward-compat/unknown-fields.json");
    let a = decode(&f["wire"]);
    assert_eq!(
        serde_json::to_value(&a).unwrap(),
        f["known"],
        "every known field equals `known`"
    );
    let raw: Value = serde_json::from_str(a.raw_json().unwrap()).unwrap();
    assert_eq!(raw["wordCount"], 512);
    assert_eq!(raw["sentiment"]["score"], 0.73);
    assert_eq!(raw["events"][0]["subtype"], "tender_offer");
}

fn f02() {
    let f = load("fixtures/forward-compat/unknown-enum-values.json");
    let a = decode(&f["wire"]);
    assert_eq!(
        a.source.r#type.as_ref().unwrap(),
        &SourceType::Other("wire_service".into())
    );
    assert_eq!(
        a.sentiment
            .value()
            .unwrap()
            .label
            .as_ref()
            .unwrap()
            .as_str(),
        "mixed"
    );
    let c = &a.companies.as_ref().unwrap()[0];
    assert_eq!(
        c.role.as_ref().unwrap(),
        &CompanyRole::Other("guarantor".into())
    );
    let e = &a.events.as_ref().unwrap()[0];
    assert_eq!(e.r#type.as_str(), "esg_controversy");
    assert!(!e.r#type.is_known());
    assert_eq!(e.direction.as_ref().unwrap().as_str(), "mixed");
    assert_eq!(e.importance.as_ref().unwrap().as_str(), "extreme");
    assert_eq!(e.importance.as_ref().unwrap().rank(), None);
    assert!(a.categories.contains(&"synthetic-new-category".to_owned()));
    assert_eq!(
        serde_json::to_value(&a).unwrap(),
        f["wire"],
        "unknown values are written back unchanged"
    );
}

/// F-03, REST and webhook envelopes; the WebSocket envelopes run in `f03_ws`.
fn f03() {
    let f = load("fixtures/forward-compat/envelope-keys.json");
    let envelopes = f["envelopes"].as_array().unwrap();
    let get = |kind: &str| envelopes.iter().find(|e| e["kind"] == kind).unwrap()["wire"].clone();
    let search = get("rest.search");
    let article = get("rest.article");
    let webhook = get("webhook");
    run_paused(async move {
        let mock = HttpMock::start().await;
        let client = client(&mock.base_url);
        mock.enqueue(Scripted::json(200, &search));
        let page = client
            .articles()
            .search(&SearchParams::new())
            .await
            .unwrap();
        assert_eq!(page.articles.len(), 1);
        assert!(!page.pagination.has_more && page.pagination.next_cursor.is_none());
        mock.enqueue(Scripted::json(200, &article));
        let a = client
            .articles()
            .get("EU8JwpsZ8iJehuyPZjMqRz", &GetArticleOptions::new())
            .await
            .unwrap();
        assert_eq!(a.id, "EU8JwpsZ8iJehuyPZjMqRz");
    });
    let body = webhook.to_string();
    let (sig, ts) = crate::webhooks::sign(&body, "whsec_test", "2026-09-18T12:03:06.000Z");
    let event =
        crate::webhooks::verify_ok(&body, &sig, &ts, "whsec_test", "2026-09-18T12:03:06.000Z");
    assert_eq!(event.event_id, "evt_dNUmEJrbL57wBuN8Rc_4aj");
    crate::sessions::f03_ws(envelopes);
}

pub fn cases() -> Vec<crate::common::Case> {
    vec![
        ("V-01", Box::new(v01)),
        ("D-01", Box::new(decode_case("full-article.json"))),
        ("D-02", Box::new(decode_case("raw-skeleton.json"))),
        ("D-03", Box::new(decode_case("no-include-full-access.json"))),
        ("D-04", Box::new(decode_case("all-include-no-access.json"))),
        (
            "D-05",
            Box::new(decode_case("events-without-companies.json")),
        ),
        (
            "D-06",
            Box::new(decode_case("events-companies-not-entitled.json")),
        ),
        ("D-07", Box::new(decode_case("legacy-values.json"))),
        ("D-08", Box::new(decode_case("non-ascii.json"))),
        ("D-09", Box::new(decode_case("partial-values.json"))),
        ("D-10", Box::new(d10)),
        ("D-11", Box::new(decode_case("blank-text.json"))),
        ("F-01", Box::new(f01)),
        ("F-02", Box::new(f02)),
        ("F-03", Box::new(f03)),
    ]
}
