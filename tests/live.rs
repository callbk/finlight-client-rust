//! Live smoke test against the finlight **dev** API. Not part of `cargo test`: it is ignored
//! and runs only with `cargo test --test live -- --ignored`, the owner's okay and all three of
//! `FINLIGHT_API_KEY`, `FINLIGHT_BASE_URL` and `FINLIGHT_WSS_URL`. A missing variable or a
//! production host fails at once. The load is small: a handful of calls and two short streams.

use std::time::Duration;

use finlight::{
    ApiErrorKind, Client, CompanySearchParams, Config, Error, GetArticleOptions, IncludeBlock,
    RawStreamParams, SearchAllOptions, SearchParams, StreamOptions, StreamParams,
};
use futures_util::StreamExt;

fn env(name: &str) -> String {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => panic!(
            "{name} is not set; live tests need FINLIGHT_API_KEY, FINLIGHT_BASE_URL and FINLIGHT_WSS_URL"
        ),
    }
}

fn dev_client() -> Client {
    let key = env("FINLIGHT_API_KEY");
    let base = env("FINLIGHT_BASE_URL");
    let wss = env("FINLIGHT_WSS_URL");
    for url in [&base, &wss] {
        let host = url
            .split("://")
            .nth(1)
            .unwrap_or_default()
            .split(['/', ':'])
            .next()
            .unwrap_or_default();
        assert!(
            host.contains(".dev.") || host == "127.0.0.1" || host == "localhost",
            "live tests run against dev only, not {host}"
        );
    }
    Client::new(
        Config::builder(key)
            .base_url(base)
            .wss_url(wss)
            .build()
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
#[ignore = "live: needs dev credentials and the owner's okay"]
async fn live_dev_smoke() {
    let client = dev_client();
    let articles = client.articles();

    let page = articles
        .search(
            &SearchParams::new()
                .query("ticker:NVDA")
                .languages(["en"])
                .include([IncludeBlock::COMPANIES, IncludeBlock::EVENTS])
                .limit(3),
        )
        .await
        .expect("search");
    println!(
        "search: {} articles, has_more {}",
        page.articles.len(),
        page.pagination.has_more
    );
    assert!(page.skipped.is_empty(), "skipped: {:?}", page.skipped);
    let first = page.articles.first().expect("dev has NVDA articles");
    assert!(first.published_at_time().is_some());

    let by_link = articles
        .get_by_link(&first.link, &GetArticleOptions::new().include(["content"]))
        .await
        .expect("get_by_link");
    assert_eq!(by_link.id, first.id);
    println!("get_by_link: content present {}", by_link.content_present());
    let by_id = articles
        .get(&first.id, &GetArticleOptions::new())
        .await
        .expect("get");
    assert_eq!(by_id.link, first.link);
    match articles
        .get("AAAAAAAAAAAAAAAAAAAAAA", &GetArticleOptions::new())
        .await
    {
        Err(Error::Api(e)) => {
            assert_eq!(e.api_kind(), ApiErrorKind::NotFound);
            assert!(e.request_id().is_some());
        }
        other => panic!("expected not_found, got {other:?}"),
    }

    let mut all = articles
        .search_all(
            SearchParams::new()
                .query("ticker:AAPL")
                .languages(["en"])
                .limit(2),
            SearchAllOptions::new().max_pages(2),
        )
        .unwrap();
    let mut n = 0;
    while let Some(a) = all.next().await {
        a.expect("search_all page");
        n += 1;
    }
    println!(
        "search_all: {n} articles, {} pages, truncated {}",
        all.pages_fetched(),
        all.truncated()
    );

    match client
        .companies()
        .search(&CompanySearchParams::new("NVDA").mic("XNAS"))
        .await
    {
        Ok(list) => {
            println!("companies.search: {} results", list.len());
            if let Some(c) = list.first() {
                let record = client
                    .companies()
                    .get(c.company_id)
                    .await
                    .expect("companies.get");
                let etag = record
                    .cache_info()
                    .and_then(|i| i.etag.clone())
                    .expect("etag");
                let again = client
                    .companies()
                    .get_if_none_match(c.company_id, &etag)
                    .await
                    .expect("304");
                println!(
                    "companies.get: {}, revalidated: {}",
                    record.name,
                    again.is_none()
                );
                let many = client
                    .companies()
                    .get_many(&[c.company_id])
                    .await
                    .expect("get_many");
                assert_eq!(many.len(), 1);
            }
        }
        Err(Error::Api(e)) if e.api_kind() == ApiErrorKind::Forbidden => {
            println!("companies: no entityAccess")
        }
        Err(e) => panic!("companies.search: {e}"),
    }

    let sources = client.sources().list().await.expect("sources");
    assert!(!sources.is_empty());
    println!("sources: {}", sources.len());

    // Enriched stream: admitted, then the initial push (one article), then close.
    let admitted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let a2 = admitted.clone();
    let mut stream = client
        .stream(
            StreamParams::new()
                .query("ticker:NVDA")
                .language("en")
                .include([IncludeBlock::COMPANIES]),
            StreamOptions::new()
                .on_admitted(move |_| a2.store(true, std::sync::atomic::Ordering::SeqCst)),
        )
        .await
        .unwrap();
    match tokio::time::timeout(Duration::from_secs(20), stream.next()).await {
        Ok(Some(Ok(event))) => println!("stream: {} {}", event.kind.as_str(), event.event),
        Ok(Some(Err(e))) => panic!("stream ended: {e}"),
        Ok(None) => panic!("stream ended without an item"),
        Err(_) => println!("stream: no item within 20 s"),
    }
    assert!(
        admitted.load(std::sync::atomic::Ordering::SeqCst),
        "connection.admitted"
    );
    stream.close().await;

    // Raw stream: admitted only (no initial push on raw).
    let admitted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let a2 = admitted.clone();
    let mut raw = client
        .raw_stream(
            RawStreamParams::new().language("en"),
            StreamOptions::new()
                .on_admitted(move |_| a2.store(true, std::sync::atomic::Ordering::SeqCst)),
        )
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while !admitted.load(std::sync::atomic::Ordering::SeqCst)
        && tokio::time::Instant::now() < deadline
    {
        tokio::select! {
            item = raw.next() => if let Some(Err(e)) = item { panic!("raw stream ended: {e}") },
            _ = tokio::time::sleep(Duration::from_millis(200)) => {}
        }
    }
    assert!(
        admitted.load(std::sync::atomic::Ordering::SeqCst),
        "raw connection.admitted"
    );
    raw.close().await;
}
