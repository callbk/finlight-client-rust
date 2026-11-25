//! Every page of a search, up to 3 pages. Each page is one call and costs quota.
//!
//! Run: `FINLIGHT_API_KEY=... cargo run --example search_all`

use finlight::{Client, Config, Error, SearchAllOptions, SearchParams};

pub fn client_from_env() -> Result<Client, Box<dyn std::error::Error>> {
    let key = std::env::var("FINLIGHT_API_KEY").map_err(|_| "set FINLIGHT_API_KEY")?;
    let mut config = Config::builder(key);
    if let Ok(url) = std::env::var("FINLIGHT_BASE_URL") {
        config = config.base_url(url);
    }
    Ok(Client::new(config.build()?)?)
}

pub async fn run(client: &Client) -> Result<usize, Error> {
    let params = SearchParams::new()
        .query("ticker:AAPL")
        .languages(["en"])
        .limit(20);
    // The body is frozen here; later pages only add the cursor.
    let mut all = client
        .articles()
        .search_all(params, SearchAllOptions::new().max_pages(3))?;
    let mut count = 0;
    while let Some(article) = all.next().await {
        let article = article?; // errors arrive during iteration; a monthly 429 ends it at once
        count += 1;
        if count <= 3 {
            println!("{}", article.title);
        }
    }
    println!("{count} articles on {} pages", all.pages_fetched());
    if all.truncated() {
        // Continue later with SearchParams::cursor(...) and the same filters.
        println!(
            "stopped at max_pages; continue with cursor {}",
            all.next_cursor().unwrap_or_default()
        );
    }
    Ok(count)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run(&client_from_env()?).await?;
    Ok(())
}
