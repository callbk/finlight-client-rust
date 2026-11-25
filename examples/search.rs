//! One page of a search: companies and events included, times through the parsed accessor.
//!
//! Run: `FINLIGHT_API_KEY=... cargo run --example search`
//! (optional `FINLIGHT_BASE_URL`, e.g. the dev API).

use std::time::SystemTime;

use finlight::{Client, Config, Error, IncludeBlock, SearchParams};

/// Builds a client from `FINLIGHT_API_KEY` and the optional `FINLIGHT_BASE_URL` / `FINLIGHT_WSS_URL`.
pub fn client_from_env() -> Result<Client, Box<dyn std::error::Error>> {
    let key = std::env::var("FINLIGHT_API_KEY").map_err(|_| "set FINLIGHT_API_KEY")?;
    let mut config = Config::builder(key);
    if let Ok(url) = std::env::var("FINLIGHT_BASE_URL") {
        config = config.base_url(url);
    }
    if let Ok(url) = std::env::var("FINLIGHT_WSS_URL") {
        config = config.wss_url(url);
    }
    Ok(Client::new(config.build()?)?)
}

pub async fn run(client: &Client) -> Result<(), Error> {
    let params = SearchParams::new()
        .query("ticker:NVDA")
        .languages(["en"])
        .include([IncludeBlock::COMPANIES, IncludeBlock::EVENTS])
        .limit(10);
    let page = match client.articles().search(&params).await {
        Ok(page) => page,
        Err(Error::Api(e)) => {
            eprintln!(
                "API error: status {} code {} request {}",
                e.status(),
                e.code().map(|c| c.as_str()).unwrap_or("-"),
                e.request_id().unwrap_or("-")
            );
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    for article in &page.articles {
        let age = article
            .published_at_time()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .map(|d| format!("{} min ago", d.as_secs() / 60))
            .unwrap_or_else(|| "time unknown".to_owned());
        println!("{age:>16}  {:<24} {}", article.source.domain, article.title);
        if article.companies_present() {
            let names: Vec<&str> = article
                .companies
                .iter()
                .flatten()
                .map(|c| c.name.as_str())
                .collect();
            println!(
                "{:>16}  companies: {}",
                "",
                if names.is_empty() {
                    "none tagged".to_owned()
                } else {
                    names.join(", ")
                }
            );
        }
    }
    println!("has more: {}", page.pagination.has_more);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run(&client_from_env()?).await?;
    Ok(())
}
