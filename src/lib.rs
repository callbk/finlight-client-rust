//! Official Rust client for the [finlight.me](https://finlight.me) API v3: financial news
//! search, article, company and source lookups, real-time streams and webhook verification.
//!
//! This crate (0.2) speaks API v3 only. Stay on `finlight-client = "0.1"` for API v2.
//! Full API documentation: <https://docs.finlight.me>
//!
//! ```no_run
//! use finlight::{Client, Config, SearchParams};
//!
//! # async fn run() -> Result<(), finlight::Error> {
//! let client = Client::new(Config::new(std::env::var("FINLIGHT_API_KEY").unwrap_or_default()))?;
//! let page = client
//!     .articles()
//!     .search(&SearchParams::new().query("ticker:NVDA").limit(10))
//!     .await?;
//! for article in &page.articles {
//!     println!("{}  {}", article.source.domain, article.title);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Logging goes through `tracing` with the target `finlight`; nothing is printed without a
//! subscriber. The API key, webhook secrets, signatures and full bodies are never logged.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

mod articles;
mod client;
mod companies;
mod config;
mod error;
mod field;
mod frames;
mod http;
mod models;
mod params;
mod sources;
mod stream;
mod time;
mod version;
mod vocab;
pub mod webhooks;
mod ws;

pub use articles::{Articles, SearchAllResult};
pub use client::Client;
pub use companies::Companies;
pub use config::{Config, ConfigBuilder, RetryOnBurst};
pub use error::{
    ApiError, ApiErrorKind, DecodeError, Error, FieldIssue, NetworkError, PreemptedError, Result,
    TimeoutError, ValidationError, WebhookVerificationError, WebhookVerificationReason,
};
pub use field::Field;
pub use models::{
    Article, ArticlePage, ArticleSource, CacheInfo, Company, CompanyList, CompanyRecord, EventTag,
    Listing, Media, Pagination, Sentiment, SkippedEntry, Source, SourceList, StreamEvent,
    StreamEventKind,
};
pub use params::{
    CompanySearchParams, DateInput, GetArticleOptions, RawStreamParams, SearchAllOptions,
    SearchParams, StreamParams,
};
pub use sources::Sources;
pub use stream::{AdmittedInfo, FinlightStream, QuotaWaitInfo, ReconnectInfo, StreamOptions};
pub use version::{PAYLOAD_VERSION, VERSION};
pub use vocab::{
    CompanyRole, ConnectionErrorCode, EventDirection, EventType, Importance, IncludeBlock, OrderBy,
    PreemptReason, ProblemCode, RateLimitType, ReconnectReason, SentimentLabel, SortOrder,
    SourceType, StreamEventName, WebhookEventName,
};
pub use webhooks::{WebhookEvent, WebhookPayload, WrongPayloadVersion};
pub use ws::{WsConnectFuture, WsConnector, WsIo};

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
