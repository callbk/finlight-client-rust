//! Wire models. Field names follow the wire keys in `snake_case`; serde writes the wire keys back.
//!
//! Readers are tolerant: unknown keys are ignored at every level, missing optional keys read as
//! `None` (lists as empty), and only the required keys (`id`, `link`, `title`, `source.domain`;
//! `companyId` and `name`; `domain`; event `type`) fail an entry. Timestamps keep the exact
//! string the server sent; the `*_time()` accessors parse it strictly (RFC 3339) into a
//! `SystemTime` and give `None` when it is not readable.

use std::sync::Arc;
use std::time::SystemTime;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::value::RawValue;

use crate::error::Error;
use crate::field::Field;
use crate::time::parse_opt;
use crate::vocab::{
    CompanyRole, EventDirection, EventType, Importance, SentimentLabel, SourceType, StreamEventName,
};

fn null_as_empty<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

/// A news article (REST, streams and webhooks share this shape).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Article {
    /// Stable id (22 characters, base64url).
    pub id: String,
    /// Article URL.
    pub link: String,
    /// Where the article comes from.
    pub source: ArticleSource,
    /// Title in the original language.
    pub title: String,
    /// Summary; `None` when there is none (never an empty string from the server).
    #[serde(default)]
    pub summary: Option<String>,
    /// ISO 639-1 language.
    #[serde(default)]
    pub language: Option<String>,
    /// Images, at most 3.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub media: Vec<Media>,
    /// Publication time claimed by the source, as sent (legacy data can carry 1970-01-01).
    #[serde(default)]
    pub published_at: Option<String>,
    /// Ingest time, as sent.
    #[serde(default)]
    pub first_seen_at: Option<String>,
    /// The source's revision time, as sent.
    #[serde(default)]
    pub revised_at: Option<String>,
    /// When the enriched article first became available, as sent (always `None` on raw).
    #[serde(default)]
    pub available_at: Option<String>,
    /// Geographic focus of the story (ISO alpha-2).
    #[serde(default, deserialize_with = "null_as_empty")]
    pub countries: Vec<String>,
    /// Categories (open vocabulary).
    #[serde(default, deserialize_with = "null_as_empty")]
    pub categories: Vec<String>,
    /// Full text: absent unless requested with `include` and entitled; `null` when there is none.
    #[serde(default, skip_serializing_if = "Field::is_absent")]
    pub content: Field<String>,
    /// Sentiment: absent unless entitled; `null` when not computed.
    #[serde(default, skip_serializing_if = "Field::is_absent")]
    pub sentiment: Field<Sentiment>,
    /// Tagged companies: `None` when absent (not requested or not entitled); `Some([])` means
    /// tagged, no company.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub companies: Option<Vec<Company>>,
    /// Tagged events: `None` when absent; `Some([])` means classified, no event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<Vec<EventTag>>,
    #[serde(skip)]
    raw: Option<Arc<str>>,
}

impl Article {
    /// `true` when the `content` key was in the JSON (`null` included).
    pub fn content_present(&self) -> bool {
        self.content.is_present()
    }

    /// `true` when the `sentiment` key was in the JSON (`null` included).
    pub fn sentiment_present(&self) -> bool {
        self.sentiment.is_present()
    }

    /// `true` when the `companies` key was in the JSON.
    pub fn companies_present(&self) -> bool {
        self.companies.is_some()
    }

    /// `true` when the `events` key was in the JSON.
    pub fn events_present(&self) -> bool {
        self.events.is_some()
    }

    /// `published_at` parsed strictly; `None` when absent or not RFC 3339.
    pub fn published_at_time(&self) -> Option<SystemTime> {
        parse_opt(self.published_at.as_deref())
    }

    /// `first_seen_at` parsed strictly.
    pub fn first_seen_at_time(&self) -> Option<SystemTime> {
        parse_opt(self.first_seen_at.as_deref())
    }

    /// `revised_at` parsed strictly.
    pub fn revised_at_time(&self) -> Option<SystemTime> {
        parse_opt(self.revised_at.as_deref())
    }

    /// `available_at` parsed strictly.
    pub fn available_at_time(&self) -> Option<SystemTime> {
        parse_opt(self.available_at.as_deref())
    }

    /// The article JSON exactly as received, unknown keys included.
    pub fn raw_json(&self) -> Option<&str> {
        self.raw.as_deref()
    }
}

/// Source of an article.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ArticleSource {
    /// Domain, e.g. `www.reuters.com`; the join key to `sources().list()`.
    pub domain: String,
    /// Source type when the article was indexed (open). `None` when the source is not
    /// classified or the article is older than source types. Can lag `Source::type` after a
    /// reclassification.
    #[serde(default)]
    pub r#type: Option<SourceType>,
}

/// An image of an article.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Media {
    /// Image URL.
    #[serde(default)]
    pub url: Option<String>,
}

/// Sentiment of an article.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Sentiment {
    /// Label (open), `None` when not set.
    #[serde(default)]
    pub label: Option<SentimentLabel>,
    /// Confidence, full precision.
    #[serde(default)]
    pub confidence: Option<f64>,
}

/// A company tagged in an article.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Company {
    /// Company id; join key to `companies().get()` and `EventTag::company_ids`.
    pub company_id: i64,
    /// Name.
    pub name: String,
    /// ISIN.
    #[serde(default)]
    pub isin: Option<String>,
    /// Primary listing; `None` for private companies.
    #[serde(default)]
    pub primary_listing: Option<Listing>,
    /// Tagging confidence.
    #[serde(default)]
    pub confidence: Option<f64>,
    /// Position in the story (open); `None` for articles tagged without roles.
    #[serde(default)]
    pub role: Option<CompanyRole>,
    /// Aboutness, 0 to 1; `None` for articles tagged without roles.
    #[serde(default)]
    pub relevance: Option<f64>,
}

/// A listing of a company.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Listing {
    /// Ticker symbol.
    #[serde(default)]
    pub ticker: Option<String>,
    /// Exchange (MIC).
    #[serde(default)]
    pub mic: Option<String>,
    /// Exchange country.
    #[serde(default)]
    pub country: Option<String>,
    /// Whether the listing is active; only set in `CompanyRecord::listings`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
}

/// An event tagged in an article.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct EventTag {
    /// Event type (open).
    pub r#type: EventType,
    /// Direction (open).
    #[serde(default)]
    pub direction: Option<EventDirection>,
    /// Importance (open, ordered).
    #[serde(default)]
    pub importance: Option<Importance>,
    /// Confidence.
    #[serde(default)]
    pub confidence: Option<f64>,
    /// Ids of the companies the event is about. `None` when absent: it is only sent when the
    /// `companies` block is in the same payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub company_ids: Option<Vec<i64>>,
}

/// Company master data from `companies()`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct CompanyRecord {
    /// Company id.
    pub company_id: i64,
    /// Name.
    pub name: String,
    /// Country of domicile.
    #[serde(default)]
    pub country: Option<String>,
    /// Sector.
    #[serde(default)]
    pub sector: Option<String>,
    /// Industry.
    #[serde(default)]
    pub industry: Option<String>,
    /// ISINs.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub isins: Vec<String>,
    /// Primary listing; `None` for private companies.
    #[serde(default)]
    pub primary_listing: Option<Listing>,
    /// One listing per exchange, the primary first.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub listings: Vec<Listing>,
    /// Last update, as sent.
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(skip)]
    raw: Option<Arc<str>>,
    #[serde(skip)]
    cache: Option<CacheInfo>,
}

impl CompanyRecord {
    /// `updated_at` parsed strictly.
    pub fn updated_at_time(&self) -> Option<SystemTime> {
        parse_opt(self.updated_at.as_deref())
    }

    /// The JSON exactly as received.
    pub fn raw_json(&self) -> Option<&str> {
        self.raw.as_deref()
    }

    /// ETag data of a record from `companies().get()`; `None` for other records.
    pub fn cache_info(&self) -> Option<&CacheInfo> {
        self.cache.as_ref()
    }

    pub(crate) fn set_cache_info(&mut self, info: CacheInfo) {
        self.cache = Some(info);
    }
}

/// ETag data of a record from `companies().get()`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct CacheInfo {
    /// `ETag` header; pass it to `companies().get_if_none_match()` later.
    pub etag: Option<String>,
    /// `Cache-Control` header.
    pub cache_control: Option<String>,
    /// `true` when the server answered 304 and the SDK returned its cached copy.
    pub not_modified: bool,
}

/// A source from `sources().list()`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Source {
    /// Domain.
    pub domain: String,
    /// Display name; `None` when the source has none.
    #[serde(default)]
    pub name: Option<String>,
    /// Source type (open); `None` when the source is not classified.
    #[serde(default)]
    pub r#type: Option<SourceType>,
    /// Part of the default pool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_default: Option<bool>,
    /// Whether the source has scraped content. `None` means unknown (your plan has no
    /// `fullArticleAccess`), never "no content".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_content: Option<bool>,
    /// A custom source granted to your subscription.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_custom: Option<bool>,
    /// Country.
    #[serde(default)]
    pub country: Option<String>,
    /// Languages, most common first.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub languages: Vec<String>,
    #[serde(skip)]
    raw: Option<Arc<str>>,
}

impl Source {
    /// The JSON exactly as received.
    pub fn raw_json(&self) -> Option<&str> {
        self.raw.as_deref()
    }
}

/// Pagination of a search page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Pagination {
    /// Page size used.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Articles on this page.
    #[serde(default)]
    pub returned: Option<u32>,
    /// More pages follow.
    pub has_more: bool,
    /// Cursor for the next page; `None` at the end.
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// An entry of a list that could not be decoded. It is left out of the list, logged as a
/// warning and kept here.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SkippedEntry {
    /// Position in the list as received.
    pub index: usize,
    /// The entry's JSON as received.
    pub raw: String,
    /// Why it was skipped.
    pub reason: String,
}

/// One page of `articles().search()`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ArticlePage {
    /// Articles of this page.
    pub articles: Vec<Article>,
    /// Pagination.
    pub pagination: Pagination,
    /// Entries that could not be decoded.
    pub skipped: Vec<SkippedEntry>,
}

macro_rules! item_list {
    ($(#[$meta:meta])* $name:ident, $item:ty) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq)]
        pub struct $name {
            items: Vec<$item>,
            skipped: Vec<SkippedEntry>,
        }

        impl $name {
            pub(crate) fn new(items: Vec<$item>, skipped: Vec<SkippedEntry>) -> Self {
                Self { items, skipped }
            }

            /// Entries that could not be decoded.
            pub fn skipped(&self) -> &[SkippedEntry] {
                &self.skipped
            }

            /// The entries as a `Vec`.
            pub fn into_vec(self) -> Vec<$item> {
                self.items
            }
        }

        impl std::ops::Deref for $name {
            type Target = [$item];
            fn deref(&self) -> &[$item] {
                &self.items
            }
        }

        impl IntoIterator for $name {
            type Item = $item;
            type IntoIter = std::vec::IntoIter<$item>;
            fn into_iter(self) -> Self::IntoIter {
                self.items.into_iter()
            }
        }

        impl<'a> IntoIterator for &'a $name {
            type Item = &'a $item;
            type IntoIter = std::slice::Iter<'a, $item>;
            fn into_iter(self) -> Self::IntoIter {
                self.items.iter()
            }
        }
    };
}

item_list!(
    /// Result of `companies().search()` and `companies().get_many()`: the records plus `skipped()`.
    CompanyList,
    CompanyRecord
);
item_list!(
    /// Result of `sources().list()`: the sources plus `skipped()`.
    SourceList,
    Source
);

/// Kind of a stream element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StreamEventKind {
    /// `article.created`.
    Created,
    /// `article.updated`.
    Updated,
    /// Any other event; `data` is `None` and the frame is reachable through `raw_json()`.
    Other,
}

impl StreamEventKind {
    /// `created`, `updated` or `other`.
    pub fn as_str(&self) -> &'static str {
        match self {
            StreamEventKind::Created => "created",
            StreamEventKind::Updated => "updated",
            StreamEventKind::Other => "other",
        }
    }
}

/// One element of a stream.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct StreamEvent {
    /// `Created`, `Updated` or `Other`.
    pub kind: StreamEventKind,
    /// The wire event name (open).
    pub event: StreamEventName,
    /// The article; `None` for `Other`.
    pub data: Option<Article>,
    raw: Arc<str>,
}

impl StreamEvent {
    pub(crate) fn new(
        kind: StreamEventKind,
        event: StreamEventName,
        data: Option<Article>,
        raw: Arc<str>,
    ) -> Self {
        StreamEvent {
            kind,
            event,
            data,
            raw,
        }
    }

    /// The whole frame exactly as received.
    pub fn raw_json(&self) -> &str {
        &self.raw
    }
}

// ---- decoding -------------------------------------------------------------------------

/// Decodes one JSON value with a path for errors.
pub(crate) fn decode_value<T: DeserializeOwned>(json: &str, path: &str) -> Result<T, Error> {
    let mut de = serde_json::Deserializer::from_str(json);
    match serde_path_to_error::deserialize::<_, T>(&mut de) {
        Ok(v) => Ok(v),
        Err(e) => {
            let inner = e.path().to_string();
            let full = match (path.is_empty(), inner.as_str()) {
                (true, _) => inner.clone(),
                (false, ".") => path.to_owned(),
                (false, _) => format!("{path}.{inner}"),
            };
            let message = format!(
                "could not decode {}: {}",
                if full.is_empty() {
                    "the response"
                } else {
                    &full
                },
                e.inner()
            );
            Err(Error::decode(
                message,
                Some(full),
                Some(json.to_owned()),
                Some(Box::new(e.into_inner())),
            ))
        }
    }
}

pub(crate) fn decode_article(raw: &RawValue, path: &str) -> Result<Article, Error> {
    let mut article: Article = decode_value(raw.get(), path)?;
    article.raw = Some(Arc::from(raw.get()));
    Ok(article)
}

pub(crate) fn decode_company(raw: &RawValue, path: &str) -> Result<CompanyRecord, Error> {
    let mut record: CompanyRecord = decode_value(raw.get(), path)?;
    record.raw = Some(Arc::from(raw.get()));
    Ok(record)
}

pub(crate) fn decode_source(raw: &RawValue, path: &str) -> Result<Source, Error> {
    let mut source: Source = decode_value(raw.get(), path)?;
    source.raw = Some(Arc::from(raw.get()));
    Ok(source)
}

/// Decodes every entry of a list; broken entries are skipped, logged and returned apart.
pub(crate) fn decode_entries<T>(
    entries: &[Box<RawValue>],
    list: &str,
    decode: impl Fn(&RawValue, &str) -> Result<T, Error>,
) -> (Vec<T>, Vec<SkippedEntry>) {
    let mut items = Vec::with_capacity(entries.len());
    let mut skipped = Vec::new();
    for (index, raw) in entries.iter().enumerate() {
        match decode(raw, &format!("{list}[{index}]")) {
            Ok(item) => items.push(item),
            Err(err) => {
                tracing::warn!(target: "finlight", list, index, "skipped an entry that could not be decoded: {err}");
                skipped.push(SkippedEntry {
                    index,
                    raw: raw.get().to_owned(),
                    reason: err.to_string(),
                });
            }
        }
    }
    (items, skipped)
}

#[derive(Deserialize)]
struct SearchEnvelope {
    articles: Vec<Box<RawValue>>,
    pagination: Pagination,
}

#[derive(Deserialize)]
struct ArticleEnvelope {
    article: Box<RawValue>,
}

#[derive(Deserialize)]
struct CompaniesEnvelope {
    companies: Vec<Box<RawValue>>,
}

#[derive(Deserialize)]
struct CompanyEnvelope {
    company: Box<RawValue>,
}

#[derive(Deserialize)]
struct SourcesEnvelope {
    sources: Vec<Box<RawValue>>,
}

pub(crate) fn decode_search_page(body: &str) -> Result<ArticlePage, Error> {
    let env: SearchEnvelope = decode_value(body, "")?;
    let (articles, skipped) = decode_entries(&env.articles, "articles", decode_article);
    Ok(ArticlePage {
        articles,
        pagination: env.pagination,
        skipped,
    })
}

pub(crate) fn decode_article_envelope(body: &str) -> Result<Article, Error> {
    let env: ArticleEnvelope = decode_value(body, "")?;
    decode_article(&env.article, "article")
}

pub(crate) fn decode_companies(body: &str) -> Result<CompanyList, Error> {
    let env: CompaniesEnvelope = decode_value(body, "")?;
    let (items, skipped) = decode_entries(&env.companies, "companies", decode_company);
    Ok(CompanyList::new(items, skipped))
}

pub(crate) fn decode_company_envelope(body: &str) -> Result<CompanyRecord, Error> {
    let env: CompanyEnvelope = decode_value(body, "")?;
    decode_company(&env.company, "company")
}

pub(crate) fn decode_sources(body: &str) -> Result<SourceList, Error> {
    let env: SourcesEnvelope = decode_value(body, "")?;
    let (items, skipped) = decode_entries(&env.sources, "sources", decode_source);
    Ok(SourceList::new(items, skipped))
}
