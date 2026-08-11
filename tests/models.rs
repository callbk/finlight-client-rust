//! Model (de)serialization tests: flexible confidence and timestamp formats,
//! camelCase field names, and omission of unset parameters.

use chrono::{TimeZone, Utc};
use finlight_client::{Article, Category, GetArticlesParams, OrderBy, SortOrder};
use serde_json::{Value, json};

fn minimal_article(extra: Value) -> Value {
    let mut v = json!({
        "link": "https://example.com/a",
        "title": "T",
        "publishDate": "2024-01-01T00:00:00Z",
        "source": "example.com",
        "language": "en",
    });
    v.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    v
}

#[test]
fn confidence_accepts_number_and_string() {
    let a: Article = serde_json::from_value(minimal_article(json!({"confidence": 0.95}))).unwrap();
    assert_eq!(a.confidence, Some(0.95));

    let a: Article =
        serde_json::from_value(minimal_article(json!({"confidence": "0.95"}))).unwrap();
    assert_eq!(a.confidence, Some(0.95));

    let a: Article = serde_json::from_value(minimal_article(json!({"confidence": null}))).unwrap();
    assert_eq!(a.confidence, None);

    let a: Article = serde_json::from_value(minimal_article(json!({}))).unwrap();
    assert_eq!(a.confidence, None);

    assert!(
        serde_json::from_value::<Article>(minimal_article(json!({"confidence": "abc"}))).is_err()
    );
}

#[test]
fn publish_date_accepts_all_api_formats() {
    let cases = [
        ("2024-01-02T03:04:05Z", (2024, 1, 2, 3, 4, 5), 0),
        (
            "2024-01-02T03:04:05.123456789Z",
            (2024, 1, 2, 3, 4, 5),
            123_456_789,
        ),
        ("2024-01-02T03:04:05+02:00", (2024, 1, 2, 1, 4, 5), 0),
        ("2024-01-02T03:04:05", (2024, 1, 2, 3, 4, 5), 0),
        (
            "2024-01-02 03:04:05.123",
            (2024, 1, 2, 3, 4, 5),
            123_000_000,
        ),
        ("2024-01-02", (2024, 1, 2, 0, 0, 0), 0),
    ];
    for (input, (y, mo, d, h, mi, s), nanos) in cases {
        let a: Article = serde_json::from_value(minimal_article(json!({"publishDate": input})))
            .unwrap_or_else(|e| panic!("cannot parse {input:?}: {e}"));
        let expected = Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap()
            + chrono::Duration::nanoseconds(nanos);
        assert_eq!(a.publish_date, expected, "input {input:?}");
    }

    assert!(
        serde_json::from_value::<Article>(minimal_article(json!({"publishDate": "not a date"})))
            .is_err()
    );
}

#[test]
fn optional_timestamps_and_flags() {
    let a: Article = serde_json::from_value(minimal_article(json!({
        "createdAt": "2024-01-02 03:04:05",
        "revisedDate": "2024-01-03",
        "isUpdate": true,
    })))
    .unwrap();
    assert_eq!(
        a.created_at,
        Some(Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap())
    );
    assert_eq!(
        a.revised_date,
        Some(Utc.with_ymd_and_hms(2024, 1, 3, 0, 0, 0).unwrap())
    );
    assert_eq!(a.is_update, Some(true));
}

#[test]
fn company_fields_use_api_names() {
    let a: Article = serde_json::from_value(minimal_article(json!({
        "companies": [{
            "companyId": 42,
            "confidence": "0.8",
            "name": "NVIDIA Corp",
            "ticker": "NVDA",
            "openfigi": "BBG000BBJQV0",
            "primaryListing": {
                "ticker": "NVDA",
                "exchangeCode": "XNAS",
                "exchangeCountry": "US",
            },
            "isins": ["US67066G1040"],
            "otherListings": [],
        }],
    })))
    .unwrap();
    let company = &a.companies[0];
    assert_eq!(company.company_id, 42);
    assert_eq!(company.confidence, Some(0.8));
    assert_eq!(company.openfigi.as_deref(), Some("BBG000BBJQV0"));
    assert_eq!(
        company.primary_listing.as_ref().unwrap().exchange_code,
        "XNAS"
    );
    assert_eq!(company.isins, ["US67066G1040"]);
}

#[test]
fn article_serializes_back_to_camel_case() {
    let a: Article = serde_json::from_value(minimal_article(json!({"confidence": "0.9"}))).unwrap();
    let v = serde_json::to_value(&a).unwrap();
    assert_eq!(v["publishDate"], "2024-01-01T00:00:00Z");
    assert_eq!(v["confidence"], 0.9);
    assert!(v.get("createdAt").is_none(), "unset fields must be omitted");
}

#[test]
fn params_serialize_camel_case_and_omit_unset() {
    let v = serde_json::to_value(GetArticlesParams {
        query: Some("nvidia".into()),
        exclude_sources: Some(vec!["www.reuters.com".into()]),
        page_size: Some(25),
        order_by: Some(OrderBy::CreatedAt),
        order: Some(SortOrder::Asc),
        categories: Some(vec![Category::Markets, Category::Crypto]),
        ..Default::default()
    })
    .unwrap();

    assert_eq!(
        v,
        json!({
            "query": "nvidia",
            "excludeSources": ["www.reuters.com"],
            "pageSize": 25,
            "orderBy": "createdAt",
            "order": "ASC",
            "categories": ["markets", "crypto"],
        })
    );
}
