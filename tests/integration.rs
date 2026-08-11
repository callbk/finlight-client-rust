//! Integration tests against the real API. Ignored by default; run with a
//! real key:
//!
//! ```sh
//! FINLIGHT_API_KEY=sk_... cargo test --test integration -- --ignored
//! ```

use std::time::Duration;

use finlight_client::{
    Client, Config, GetArticleByLinkParams, GetArticlesParams, GetArticlesWebSocketParams,
};
use futures_util::StreamExt;

fn client_from_env() -> Client {
    let api_key = std::env::var("FINLIGHT_API_KEY").expect("FINLIGHT_API_KEY not set");
    let mut cfg = Config::new(api_key);
    if let Ok(base_url) = std::env::var("FINLIGHT_BASE_URL") {
        cfg.base_url = base_url;
    }
    if let Ok(wss_url) = std::env::var("FINLIGHT_WSS_URL") {
        cfg.wss_url = wss_url;
    }
    Client::new(cfg).unwrap()
}

#[tokio::test]
#[ignore = "requires FINLIGHT_API_KEY"]
async fn rest_endpoints() {
    let client = client_from_env();

    let resp = client
        .articles
        .fetch_articles(&GetArticlesParams {
            query: Some("nvidia".into()),
            page_size: Some(5),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(resp.status, "ok");
    assert!(!resp.articles.is_empty(), "expected articles for 'nvidia'");

    let article = client
        .articles
        .fetch_article_by_link(&GetArticleByLinkParams {
            link: resp.articles[0].link.clone(),
            include_content: true,
            include_entities: false,
        })
        .await
        .unwrap();
    assert_eq!(article.link, resp.articles[0].link);

    let sources = client.sources.get_sources().await.unwrap();
    assert!(!sources.is_empty());
}

#[tokio::test]
#[ignore = "requires FINLIGHT_API_KEY"]
async fn websocket_stream_delivers_articles() {
    let client = client_from_env();

    let mut stream = client
        .websocket
        .stream(GetArticlesWebSocketParams::default());
    let first = tokio::time::timeout(Duration::from_secs(60), stream.next())
        .await
        .expect("no article within 60s")
        .expect("stream ended unexpectedly")
        .expect("terminal stream error");
    assert!(!first.link.is_empty());
}
