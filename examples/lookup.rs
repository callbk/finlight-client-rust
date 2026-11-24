//! Lookups: an article by link and by id, a 404, companies (entityAccess) and the source list.
//!
//! Run: `FINLIGHT_API_KEY=... cargo run --example lookup`

use finlight::{
    ApiErrorKind, Client, CompanySearchParams, Config, Error, Field, GetArticleOptions,
    SearchParams,
};

pub fn client_from_env() -> Result<Client, Box<dyn std::error::Error>> {
    let key = std::env::var("FINLIGHT_API_KEY").map_err(|_| "set FINLIGHT_API_KEY")?;
    let mut config = Config::builder(key);
    if let Ok(url) = std::env::var("FINLIGHT_BASE_URL") {
        config = config.base_url(url);
    }
    Ok(Client::new(config.build()?)?)
}

pub async fn run(client: &Client) -> Result<(), Error> {
    let articles = client.articles();
    // A link to look up: the newest article of a small search.
    let page = articles
        .search(&SearchParams::new().query("ticker:NVDA").limit(1))
        .await?;
    let Some(first) = page.articles.first() else {
        println!("no article found");
        return Ok(());
    };

    let with_content = GetArticleOptions::new().include(["content"]);
    let article = articles.get_by_link(&first.link, &with_content).await?;
    match &article.content {
        Field::Value(text) => println!("content: {} characters", text.chars().count()),
        Field::Null => println!("content: none for this article"),
        Field::Absent => println!("content: not in your plan"),
    }
    println!("content key present: {}", article.content_present());

    let by_id = articles.get(&article.id, &GetArticleOptions::new()).await?;
    println!("by id: {}", by_id.title);

    // A miss is an error, not an empty result. Articles from before the id rollout are only
    // reachable by link.
    match articles
        .get("AAAAAAAAAAAAAAAAAAAAAA", &GetArticleOptions::new())
        .await
    {
        Err(Error::Api(e)) if e.api_kind() == ApiErrorKind::NotFound => {
            println!("unknown id: not found")
        }
        Err(e) => return Err(e),
        Ok(_) => println!("unexpected hit"),
    }

    // Company master data needs the entityAccess entitlement.
    match client
        .companies()
        .search(&CompanySearchParams::new("NVDA").mic("XNAS"))
        .await
    {
        Ok(companies) => {
            if let Some(company) = companies.first() {
                let record = client.companies().get(company.company_id).await?;
                let etag = record
                    .cache_info()
                    .and_then(|c| c.etag.clone())
                    .unwrap_or_default();
                println!(
                    "{} ({} listings), etag {etag}",
                    record.name,
                    record.listings.len()
                );
            }
        }
        Err(Error::Api(e)) if e.api_kind() == ApiErrorKind::Forbidden => {
            println!("companies: not in your plan")
        }
        Err(e) => return Err(e),
    }

    let sources = client.sources().list().await?;
    let with_content = sources
        .iter()
        .filter(|s| s.has_content == Some(true))
        .count();
    println!(
        "{} sources, {with_content} with content (None means: not in your plan)",
        sources.len()
    );
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run(&client_from_env()?).await?;
    Ok(())
}
