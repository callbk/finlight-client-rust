use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One exchange listing of a company.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    /// Ticker symbol on this exchange.
    pub ticker: String,
    /// Exchange code, e.g. `XNAS`.
    pub exchange_code: String,
    /// Country of the exchange.
    pub exchange_country: String,
}

/// An entity recognized in an article.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Company {
    /// finlight's internal company id.
    pub company_id: i64,
    /// Recognition confidence in `[0, 1]`. The API delivers this both as a
    /// number and as a string; both are accepted.
    #[serde(
        default,
        with = "flex::opt_f64",
        skip_serializing_if = "Option::is_none"
    )]
    pub confidence: Option<f64>,
    /// Country of the company.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    /// Primary exchange.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exchange: Option<String>,
    /// Industry classification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub industry: Option<String>,
    /// Sector classification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sector: Option<String>,
    /// Company name.
    pub name: String,
    /// Primary ticker symbol.
    pub ticker: String,
    /// Primary ISIN.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isin: Option<String>,
    /// OpenFIGI identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openfigi: Option<String>,
    /// Primary exchange listing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_listing: Option<Listing>,
    /// All known ISINs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub isins: Vec<String>,
    /// Listings on other exchanges.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub other_listings: Vec<Listing>,
}

/// An enriched news article as returned by the REST API, the enhanced
/// WebSocket stream, and webhooks.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Article {
    /// Canonical URL of the article.
    pub link: String,
    /// Headline.
    pub title: String,
    /// Publication timestamp. The API's timestamp formats (RFC 3339 with or
    /// without zone, space-separated, date-only) are all accepted.
    #[serde(with = "flex::datetime")]
    pub publish_date: DateTime<Utc>,
    /// Source domain, e.g. `www.reuters.com`.
    pub source: String,
    /// ISO 639-1 language code.
    pub language: String,
    /// Article summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Image URLs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    /// When finlight first stored the article.
    #[serde(
        default,
        with = "flex::opt_datetime",
        skip_serializing_if = "Option::is_none"
    )]
    pub created_at: Option<DateTime<Utc>>,
    /// When the article was last revised by its publisher.
    #[serde(
        default,
        with = "flex::opt_datetime",
        skip_serializing_if = "Option::is_none"
    )]
    pub revised_date: Option<DateTime<Utc>>,
    /// Whether this delivery is an update to a previously seen article.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_update: Option<bool>,
    /// Categories assigned by finlight's classification.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    /// Sentiment label (`positive`, `negative`, `neutral`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sentiment: Option<String>,
    /// Sentiment confidence in `[0, 1]`; accepted both as number and string.
    #[serde(
        default,
        with = "flex::opt_f64",
        skip_serializing_if = "Option::is_none"
    )]
    pub confidence: Option<f64>,
    /// Full article content (when requested and available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Companies recognized in the article (when requested).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub companies: Vec<Company>,
    /// Countries the article relates to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub countries: Vec<String>,
}

/// An unenriched article as delivered by the raw WebSocket stream (no
/// sentiment, entities, or content — lower latency).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RawArticle {
    /// Canonical URL of the article.
    pub link: String,
    /// Headline.
    pub title: String,
    /// Publication timestamp.
    #[serde(with = "flex::datetime")]
    pub publish_date: DateTime<Utc>,
    /// Source domain.
    pub source: String,
    /// ISO 639-1 language code.
    pub language: String,
    /// Article summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Image URLs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    /// When finlight first stored the article.
    #[serde(
        default,
        with = "flex::opt_datetime",
        skip_serializing_if = "Option::is_none"
    )]
    pub created_at: Option<DateTime<Utc>>,
    /// When the article was last revised by its publisher.
    #[serde(
        default,
        with = "flex::opt_datetime",
        skip_serializing_if = "Option::is_none"
    )]
    pub revised_date: Option<DateTime<Utc>>,
    /// Whether this delivery is an update to a previously seen article.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_update: Option<bool>,
    /// Categories assigned by finlight's classification.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
}

/// Paginated result of [`ArticleService::fetch_articles`](crate::ArticleService::fetch_articles).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArticleResponse {
    /// Request status reported by the server.
    pub status: String,
    /// Result page number.
    pub page: u32,
    /// Page size used for this result.
    pub page_size: u32,
    /// The articles of this page.
    pub articles: Vec<Article>,
}

/// A news source available through the API.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    /// Source domain, e.g. `www.reuters.com`.
    pub domain: String,
    /// Whether full article content is available for this source.
    pub is_content_available: bool,
    /// Whether the source is queried by default (without opt-in).
    pub is_default_source: bool,
}

/// Serde adapters for the flexible value representations used by the API:
/// confidence as number or string, timestamps in several formats.
pub(crate) mod flex {
    use chrono::{DateTime, NaiveDate, NaiveDateTime, SecondsFormat, Utc};
    use serde::de::{Deserializer, Error as _};
    use serde::{Deserialize, Serializer};

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum NumOrStr {
        Num(f64),
        Str(String),
    }

    fn num_or_str_to_f64<E: serde::de::Error>(v: NumOrStr) -> Result<f64, E> {
        match v {
            NumOrStr::Num(n) => Ok(n),
            NumOrStr::Str(s) => s
                .trim()
                .parse::<f64>()
                .map_err(|_| E::custom(format!("cannot parse {s:?} as float"))),
        }
    }

    /// `Option<f64>` from a JSON number, string, or null.
    pub(crate) mod opt_f64 {
        use super::*;

        pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
            de: D,
        ) -> Result<Option<f64>, D::Error> {
            Option::<NumOrStr>::deserialize(de)?
                .map(num_or_str_to_f64)
                .transpose()
        }

        pub(crate) fn serialize<S: Serializer>(v: &Option<f64>, ser: S) -> Result<S::Ok, S::Error> {
            match v {
                Some(f) => ser.serialize_f64(*f),
                None => ser.serialize_none(),
            }
        }
    }

    pub(crate) fn parse_datetime(s: &str) -> Option<DateTime<Utc>> {
        let s = s.trim();
        if let Ok(t) = DateTime::parse_from_rfc3339(s) {
            return Some(t.with_timezone(&Utc));
        }
        for format in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"] {
            if let Ok(t) = NaiveDateTime::parse_from_str(s, format) {
                return Some(t.and_utc());
            }
        }
        if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
            return Some(d.and_hms_opt(0, 0, 0)?.and_utc());
        }
        None
    }

    /// `DateTime<Utc>` from the timestamp formats used by the finlight API
    /// (RFC 3339 with or without zone, space-separated, date-only).
    pub(crate) mod datetime {
        use super::*;

        pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
            de: D,
        ) -> Result<DateTime<Utc>, D::Error> {
            let s = String::deserialize(de)?;
            parse_datetime(&s)
                .ok_or_else(|| D::Error::custom(format!("cannot parse {s:?} as timestamp")))
        }

        pub(crate) fn serialize<S: Serializer>(
            t: &DateTime<Utc>,
            ser: S,
        ) -> Result<S::Ok, S::Error> {
            ser.serialize_str(&t.to_rfc3339_opts(SecondsFormat::AutoSi, true))
        }
    }

    /// `Option<DateTime<Utc>>` variant of [`datetime`].
    pub(crate) mod opt_datetime {
        use super::*;

        pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
            de: D,
        ) -> Result<Option<DateTime<Utc>>, D::Error> {
            Option::<String>::deserialize(de)?
                .map(|s| {
                    parse_datetime(&s)
                        .ok_or_else(|| D::Error::custom(format!("cannot parse {s:?} as timestamp")))
                })
                .transpose()
        }

        pub(crate) fn serialize<S: Serializer>(
            t: &Option<DateTime<Utc>>,
            ser: S,
        ) -> Result<S::Ok, S::Error> {
            match t {
                Some(t) => datetime::serialize(t, ser),
                None => ser.serialize_none(),
            }
        }
    }
}
