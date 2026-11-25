//! Live articles over WebSocket. Pings, reconnects and rotation are handled by the SDK.
//!
//! Run: `FINLIGHT_API_KEY=... cargo run --example stream` (optional `FINLIGHT_WSS_URL`).

use finlight::{Client, Config, Error, IncludeBlock, StreamEventKind, StreamOptions, StreamParams};
use futures_util::StreamExt;

pub fn client_from_env() -> Result<Client, Box<dyn std::error::Error>> {
    let key = std::env::var("FINLIGHT_API_KEY").map_err(|_| "set FINLIGHT_API_KEY")?;
    let mut config = Config::builder(key);
    if let Ok(url) = std::env::var("FINLIGHT_WSS_URL") {
        config = config.wss_url(url);
    }
    Ok(Client::new(config.build()?)?)
}

pub async fn run(client: &Client, max_items: usize) -> Result<usize, Error> {
    let params = StreamParams::new()
        .query("ticker:NVDA OR ticker:AMD")
        .language("en")
        .include([IncludeBlock::COMPANIES])
        .include_updates(true);
    let options = StreamOptions::new()
        .on_admitted(|a| println!("admitted, lease {}", a.lease_id.as_deref().unwrap_or("-")))
        // The SDK does not backfill: fill gaps after a reconnect through REST if you need them.
        .on_reconnect(|r| {
            println!(
                "reconnected ({}), last seen {}",
                r.reason,
                r.last_seen_at.as_deref().unwrap_or("nothing yet")
            )
        })
        .on_error(|e| eprintln!("stream problem: {e}"));
    let mut stream = client.stream(params, options).await?;
    let mut seen = 0;
    while let Some(item) = stream.next().await {
        match item {
            Ok(event) if event.kind == StreamEventKind::Other => continue, // unknown to this SDK version
            Ok(event) => {
                if let Some(article) = &event.data {
                    println!("{}: {}", event.kind.as_str(), article.title);
                }
                seen += 1;
                if seen >= max_items {
                    break; // dropping the stream closes the socket
                }
            }
            Err(Error::Preempted(p)) => {
                eprintln!("preempted ({})", p.reason());
                break;
            }
            Err(Error::Api(e)) => {
                eprintln!(
                    "stream refused: {} {}",
                    e.status(),
                    e.detail().unwrap_or_default()
                );
                break;
            }
            Err(e) => return Err(e),
        }
    }
    stream.close().await;
    Ok(seen)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    run(&client_from_env()?, 5).await?;
    Ok(())
}
