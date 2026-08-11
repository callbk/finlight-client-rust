//! REST client tests against a local axum server, mirroring the sibling
//! clients' suites: auth headers, request encoding, envelope unwrapping, and
//! retry behavior.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use finlight_client::{
    Client, Config, Error, GetArticleByLinkParams, GetArticlesParams, OrderBy, SortOrder,
};
use serde_json::{Value, json};
use tokio::sync::Mutex;

fn test_config(addr: &str) -> Config {
    let mut cfg = Config::new("test-key");
    cfg.base_url = format!("http://{addr}");
    cfg
}

fn article_json(link: &str) -> Value {
    json!({
        "link": link,
        "title": format!("Title {link}"),
        "publishDate": "2024-01-01T00:00:00Z",
        "source": "example.com",
        "language": "en",
    })
}

#[tokio::test]
async fn new_requires_api_key() {
    assert!(matches!(
        Client::new(Config::new("")),
        Err(Error::MissingApiKey)
    ));
}

#[tokio::test]
async fn fetch_articles_sends_auth_and_body() {
    let captured: Arc<Mutex<Option<(HeaderMap, Value)>>> = Arc::default();
    let state = captured.clone();
    let app = Router::new().route(
        "/v2/articles",
        post(
            async move |headers: HeaderMap, Json(body): Json<Value>| -> Json<Value> {
                *state.lock().await = Some((headers, body));
                let mut article = article_json("https://example.com/a");
                article["confidence"] = json!("0.9");
                Json(json!({"status": "ok", "page": 1, "pageSize": 10, "articles": [article]}))
            },
        ),
    );
    let addr = common::serve(app).await;

    let client = Client::new(test_config(&addr)).unwrap();
    let resp = client
        .articles
        .fetch_articles(&GetArticlesParams {
            query: Some("nvidia".into()),
            from: Some("2024-01-01".into()),
            page_size: Some(10),
            order_by: Some(OrderBy::PublishDate),
            order: Some(SortOrder::Desc),
            ..Default::default()
        })
        .await
        .unwrap();

    let (headers, body) = captured.lock().await.take().unwrap();
    assert_eq!(headers.get("x-api-key").unwrap(), "test-key");
    assert!(
        headers
            .get("user-agent")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("rust/finlight-client-rust@")
    );
    assert_eq!(body["query"], "nvidia");
    assert_eq!(body["from"], "2024-01-01");
    assert_eq!(body["pageSize"], 10);
    assert_eq!(body["orderBy"], "publishDate");
    assert_eq!(body["order"], "DESC");
    assert!(
        body.get("includeContent").is_none(),
        "unset fields must be omitted: {body}"
    );

    assert_eq!(resp.articles.len(), 1);
    assert_eq!(resp.articles[0].confidence, Some(0.9));
}

#[tokio::test]
async fn fetch_article_by_link_unwraps_envelope() {
    let captured: Arc<Mutex<Option<Value>>> = Arc::default();
    let state = captured.clone();
    let app = Router::new().route(
        "/v2/articles/by-link",
        get(async move |Query(query): Query<Value>| -> Json<Value> {
            *state.lock().await = Some(query);
            Json(json!({"article": article_json("https://example.com/a")}))
        }),
    );
    let addr = common::serve(app).await;

    let client = Client::new(test_config(&addr)).unwrap();
    let article = client
        .articles
        .fetch_article_by_link(&GetArticleByLinkParams {
            link: "https://example.com/a".into(),
            include_content: true,
            include_entities: false,
        })
        .await
        .unwrap();

    let query = captured.lock().await.take().unwrap();
    assert_eq!(query["link"], "https://example.com/a");
    assert_eq!(query["includeContent"], "true");
    assert!(
        query.get("includeEntities").is_none(),
        "false include_entities must be omitted: {query}"
    );
    assert_eq!(article.title, "Title https://example.com/a");
}

#[tokio::test]
async fn get_sources() {
    let app = Router::new().route(
        "/v2/sources",
        get(async || -> Json<Value> {
            Json(json!([
                {"domain": "example.com", "isContentAvailable": true, "isDefaultSource": false}
            ]))
        }),
    );
    let addr = common::serve(app).await;

    let client = Client::new(test_config(&addr)).unwrap();
    let sources = client.sources.get_sources().await.unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].domain, "example.com");
    assert!(sources[0].is_content_available);
    assert!(!sources[0].is_default_source);
}

fn counting_sources_app(fail_first: u32) -> (Router, Arc<AtomicU32>) {
    let attempts = Arc::new(AtomicU32::new(0));
    let state = attempts.clone();
    let app = Router::new()
        .route(
            "/v2/sources",
            get(
                async move |State(state): State<Arc<AtomicU32>>| -> Result<Json<Value>, StatusCode> {
                    if state.fetch_add(1, Ordering::SeqCst) < fail_first {
                        Err(StatusCode::SERVICE_UNAVAILABLE)
                    } else {
                        Ok(Json(json!([])))
                    }
                },
            ),
        )
        .with_state(state);
    (app, attempts)
}

#[tokio::test]
async fn retry_on_503_then_success() {
    let (app, attempts) = counting_sources_app(1);
    let addr = common::serve(app).await;

    let client = Client::new(test_config(&addr)).unwrap();
    client.sources.get_sources().await.unwrap();
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn no_retry_on_400() {
    let attempts = Arc::new(AtomicU32::new(0));
    let state = attempts.clone();
    let app = Router::new()
        .route(
            "/v2/sources",
            get(async |State(state): State<Arc<AtomicU32>>| {
                state.fetch_add(1, Ordering::SeqCst);
                (StatusCode::BAD_REQUEST, r#"{"message":"bad request"}"#)
            }),
        )
        .with_state(state);
    let addr = common::serve(app).await;

    let client = Client::new(test_config(&addr)).unwrap();
    let err = client.sources.get_sources().await.unwrap_err();
    match err {
        Error::Api { status, body } => {
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert!(body.contains("bad request"));
        }
        other => panic!("expected Error::Api, got {other:?}"),
    }
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn retries_exhausted() {
    let (app, attempts) = counting_sources_app(u32::MAX);
    let addr = common::serve(app).await;

    let mut cfg = test_config(&addr);
    cfg.retry_count = 2;
    let client = Client::new(cfg).unwrap();
    let err = client.sources.get_sources().await.unwrap_err();
    assert!(
        matches!(err, Error::Api { status, .. } if status == StatusCode::SERVICE_UNAVAILABLE),
        "expected 503 Error::Api, got {err:?}"
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}
