//! One-shot smoke run against the real API: REST endpoints + 30s article
//! stream. Mirrors the sibling clients' smoke runners; keep credentials in
//! the environment, never in code.
//!
//! ```sh
//! FINLIGHT_API_KEY=sk_... cargo run --example smoke
//! ```

use std::time::Duration;

use finlight_client::{
    Client, Config, GetArticleByLinkParams, GetArticlesParams, GetArticlesWebSocketParams,
};
use futures_util::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let mut cfg = Config::new(std::env::var("FINLIGHT_API_KEY").expect("FINLIGHT_API_KEY not set"));
    if let Ok(base_url) = std::env::var("FINLIGHT_BASE_URL") {
        cfg.base_url = base_url;
    }
    if let Ok(wss_url) = std::env::var("FINLIGHT_WSS_URL") {
        cfg.wss_url = wss_url;
    }
    let client = Client::new(cfg)?;

    println!("== fetch_articles(query: nvidia) ==");
    let resp = client
        .articles
        .fetch_articles(&GetArticlesParams {
            query: Some("nvidia".into()),
            page_size: Some(5),
            ..Default::default()
        })
        .await?;
    for article in &resp.articles {
        println!("  [{}] {}", article.source, article.title);
    }

    if let Some(first) = resp.articles.first() {
        println!("== fetch_article_by_link ==");
        let article = client
            .articles
            .fetch_article_by_link(&GetArticleByLinkParams {
                link: first.link.clone(),
                include_content: true,
                include_entities: false,
            })
            .await?;
        println!(
            "  {} (content: {} chars)",
            article.title,
            article.content.map(|c| c.len()).unwrap_or(0)
        );
    }

    println!("== get_sources ==");
    let sources = client.sources.get_sources().await?;
    println!("  {} sources", sources.len());

    println!("== websocket stream (30s) ==");
    let mut stream = client
        .websocket
        .stream(GetArticlesWebSocketParams::default());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut count = 0u32;
    loop {
        match tokio::time::timeout_at(deadline, stream.next()).await {
            Err(_) => break, // 30s over
            Ok(None) => break,
            Ok(Some(Err(e))) => return Err(e.into()),
            Ok(Some(Ok(article))) => {
                count += 1;
                println!("  [{}] {}", article.source, article.title);
            }
        }
    }
    println!("  {count} articles in 30s");

    Ok(())
}
