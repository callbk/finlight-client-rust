use serde::{Deserialize, Serialize};

/// An article category assigned by finlight's classification.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Category {
    /// Financial markets.
    Markets,
    /// Macroeconomics.
    Economy,
    /// Company and business news.
    Business,
    /// Domestic politics.
    Politics,
    /// International relations.
    Geopolitics,
    /// Regulation and policy.
    Regulation,
    /// Technology.
    Technology,
    /// Energy.
    Energy,
    /// Commodities.
    Commodities,
    /// Cryptocurrencies.
    Crypto,
    /// Health and pharma.
    Health,
    /// Climate.
    Climate,
    /// Security and defense.
    Security,
}

/// Sort field for article queries.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OrderBy {
    /// Sort by publication date.
    PublishDate,
    /// Sort by the time finlight stored the article.
    CreatedAt,
    /// Sort by the publisher's revision date.
    RevisedDate,
}

/// Sort direction for article queries.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum SortOrder {
    /// Ascending.
    #[serde(rename = "ASC")]
    Asc,
    /// Descending.
    #[serde(rename = "DESC")]
    Desc,
}

/// Search parameters for [`ArticleService::fetch_articles`](crate::ArticleService::fetch_articles).
/// `None` fields are omitted from the request; the server applies its
/// documented defaults.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetArticlesParams {
    /// Query in the finlight query language, e.g.
    /// `(ticker:AAPL OR ticker:NVDA) AND NOT source:www.reuters.com AND "Elon Musk"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// Deprecated: use `sources`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Restrict results to these source domains.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<String>>,
    /// Exclude these source domains.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_sources: Option<Vec<String>>,
    /// Include these non-default sources.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opt_in_sources: Option<Vec<String>>,
    /// Earliest publish date, `YYYY-MM-DD` or ISO 8601.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Latest publish date, `YYYY-MM-DD` or ISO 8601.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// ISO 639-1 language code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Restrict results to articles mentioning these tickers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tickers: Option<Vec<String>>,
    /// Include recognized entities in the response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_entities: Option<bool>,
    /// Skip articles without content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_empty_content: Option<bool>,
    /// Include full article content in the response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_content: Option<bool>,
    /// Sort field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_by: Option<OrderBy>,
    /// Sort direction.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<SortOrder>,
    /// Page size, 1–1000.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_size: Option<u32>,
    /// Page number, starting at 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    /// Restrict results to these countries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub countries: Option<Vec<String>>,
    /// Restrict results to these categories.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<Category>>,
}

/// Parameters for [`ArticleService::fetch_article_by_link`](crate::ArticleService::fetch_article_by_link).
#[derive(Clone, Debug, Default)]
pub struct GetArticleByLinkParams {
    /// URL of the article (required).
    pub link: String,
    /// Include full article content in the response.
    pub include_content: bool,
    /// Include recognized entities in the response.
    pub include_entities: bool,
}

/// Filters for the enhanced WebSocket stream.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetArticlesWebSocketParams {
    /// Query in the finlight query language.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// Restrict the stream to these source domains.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<String>>,
    /// Exclude these source domains.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_sources: Option<Vec<String>>,
    /// Include these non-default sources.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opt_in_sources: Option<Vec<String>>,
    /// ISO 639-1 language code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Deprecated: use `include_content`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extended: Option<bool>,
    /// Restrict the stream to articles mentioning these tickers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tickers: Option<Vec<String>>,
    /// Include recognized entities.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_entities: Option<bool>,
    /// Skip articles without content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_empty_content: Option<bool>,
    /// Include full article content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_content: Option<bool>,
    /// Restrict the stream to these countries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub countries: Option<Vec<String>>,
    /// Restrict the stream to these categories.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<Category>>,
    /// Also deliver updates to previously seen articles.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_updates: Option<bool>,
}

/// Filters for the raw WebSocket stream.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetRawArticlesWebSocketParams {
    /// Query in the finlight query language.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// Restrict the stream to these source domains.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<String>>,
    /// Exclude these source domains.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_sources: Option<Vec<String>>,
    /// Include these non-default sources.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opt_in_sources: Option<Vec<String>>,
    /// ISO 639-1 language code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Also deliver updates to previously seen articles.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_updates: Option<bool>,
}
