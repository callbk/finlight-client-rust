//! Official Rust client for the [finlight.me](https://finlight.me) API —
//! financial news with sentiment analysis, entity recognition, and real-time
//! streaming.
//!
//! Full API documentation: <https://docs.finlight.me>
//!
//! # Quick start
//!
//! ```no_run
//! use finlight_client::{Client, Config, GetArticlesParams};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let client = Client::new(Config::new(std::env::var("FINLIGHT_API_KEY")?))?;
//!
//!     let resp = client
//!         .articles
//!         .fetch_articles(&GetArticlesParams {
//!             query: Some("nvidia".into()),
//!             page_size: Some(10),
//!             ..Default::default()
//!         })
//!         .await?;
//!     for article in resp.articles {
//!         println!("[{}] {}", article.source, article.title);
//!     }
//!     Ok(())
//! }
//! ```
//!
//! # Streaming
//!
//! ```no_run
//! use finlight_client::{Client, Config, GetArticlesWebSocketParams};
//! use futures_util::StreamExt;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let client = Client::new(Config::new(std::env::var("FINLIGHT_API_KEY")?))?;
//!
//!     let mut stream = client.websocket.stream(GetArticlesWebSocketParams {
//!         query: Some("nvidia".into()),
//!         ..Default::default()
//!     });
//!     while let Some(article) = stream.next().await {
//!         let article = article?; // Err is terminal (e.g. blocked by server)
//!         println!("[{}] {}", article.source, article.title);
//!     }
//!     Ok(())
//! }
//! ```

#![warn(missing_docs)]

mod client;
mod config;
mod error;
mod http;
mod models;
mod params;
mod services;
mod version;
mod webhook;
mod websocket;

pub use client::Client;
pub use config::Config;
pub use error::{Error, WebhookVerificationError};
pub use models::{Article, ArticleResponse, Company, Listing, RawArticle, Source};
pub use params::{
    Category, GetArticleByLinkParams, GetArticlesParams, GetArticlesWebSocketParams,
    GetRawArticlesWebSocketParams, OrderBy, SortOrder,
};
pub use services::{ArticleService, SourceService};
pub use webhook::construct_webhook_event;
pub use websocket::{ArticleStream, RawWebSocketClient, WebSocketClient, WebSocketOptions};
