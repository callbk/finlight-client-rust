//! Request parameters.
//!
//! Only what you set is sent: the SDK adds no defaults (no `languages`, no `to`, no `limit`),
//! and an empty search sends exactly `{}`. Values of vocabularies are not checked locally;
//! the server answers unknown values with a `ValidationFailed` error that lists the allowed ones.
//! Only the shape is checked, before any request: `limit` 1 to 100, `languages` not empty,
//! `min_relevance` 0 to 1, at most 3 `include` blocks, `q` at most 200 characters, `mic` at
//! most 32, 1 to 10 company ids of at least 1.

use std::time::SystemTime;

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::error::{Error, Result};
use crate::time::{format_date, format_utc_millis};
use crate::version::PAYLOAD_VERSION;
use crate::vocab::{
    CompanyRole, EventDirection, EventType, Importance, IncludeBlock, OrderBy, SortOrder,
    SourceType,
};

/// A `from` or `to` value.
///
/// A `SystemTime` goes out as UTC ISO 8601 with milliseconds (`2026-11-01T00:00:00.000Z`),
/// a date as `YYYY-MM-DD`, and a string unchanged. A time without a zone cannot be expressed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DateInput {
    /// An instant.
    Time(SystemTime),
    /// A calendar date.
    Date {
        /// Year (0 to 9999).
        year: i32,
        /// Month (1 to 12).
        month: u32,
        /// Day of the month.
        day: u32,
    },
    /// A string sent unchanged.
    Text(String),
}

impl DateInput {
    /// A calendar date, sent as `YYYY-MM-DD`.
    pub fn date(year: i32, month: u32, day: u32) -> Self {
        DateInput::Date { year, month, day }
    }

    fn to_wire(&self, key: &str) -> Result<String> {
        match self {
            DateInput::Time(t) => format_utc_millis(*t).ok_or_else(|| {
                Error::validation(
                    format!("{key} is outside the years 0000 to 9999"),
                    Some(key),
                )
            }),
            DateInput::Date { year, month, day } => format_date(*year, *month, *day)
                .ok_or_else(|| Error::validation(format!("{key} is not a valid date"), Some(key))),
            DateInput::Text(s) => Ok(s.clone()),
        }
    }
}

impl From<SystemTime> for DateInput {
    fn from(value: SystemTime) -> Self {
        DateInput::Time(value)
    }
}

impl From<&str> for DateInput {
    fn from(value: &str) -> Self {
        DateInput::Text(value.to_owned())
    }
}

impl From<String> for DateInput {
    fn from(value: String) -> Self {
        DateInput::Text(value)
    }
}

impl<'de> Deserialize<'de> for DateInput {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        Ok(DateInput::Text(String::deserialize(deserializer)?))
    }
}

fn strings<I, S>(values: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    values.into_iter().map(Into::into).collect()
}

fn vocab<T, I, S>(values: I) -> Vec<T>
where
    I: IntoIterator<Item = S>,
    S: Into<T>,
{
    values.into_iter().map(Into::into).collect()
}

macro_rules! string_setters {
    ($( $(#[$m:meta])* $name:ident ),* $(,)?) => {
        $(
            $(#[$m])*
            pub fn $name(mut self, value: impl Into<String>) -> Self {
                self.$name = Some(value.into());
                self
            }
        )*
    };
}

macro_rules! list_setters {
    ($( $(#[$m:meta])* $name:ident ),* $(,)?) => {
        $(
            $(#[$m])*
            pub fn $name<I, S>(mut self, values: I) -> Self
            where
                I: IntoIterator<Item = S>,
                S: Into<String>,
            {
                self.$name = Some(strings(values));
                self
            }
        )*
    };
}

macro_rules! vocab_list_setters {
    ($( $(#[$m:meta])* $name:ident : $ty:ty ),* $(,)?) => {
        $(
            $(#[$m])*
            pub fn $name<I, S>(mut self, values: I) -> Self
            where
                I: IntoIterator<Item = S>,
                S: Into<$ty>,
            {
                self.$name = Some(vocab(values));
                self
            }
        )*
    };
}

macro_rules! bool_setters {
    ($( $(#[$m:meta])* $name:ident ),* $(,)?) => {
        $(
            $(#[$m])*
            pub fn $name(mut self, value: bool) -> Self {
                self.$name = Some(value);
                self
            }
        )*
    };
}

fn from_json_error(err: serde_json::Error, what: &str) -> Error {
    let text = err.to_string();
    if let Some(rest) = text.strip_prefix("unknown field `") {
        let key = rest.split('`').next().unwrap_or_default();
        return Error::validation(
            format!(
                "unknown {what} parameter \"{key}\"; the server would ignore it silently. \
                 See https://docs.finlight.me for the v3 parameters."
            ),
            Some(key),
        );
    }
    Error::validation(format!("invalid {what} parameters: {text}"), None)
}

fn check_include(include: &Option<Vec<IncludeBlock>>) -> Result<Option<Vec<IncludeBlock>>> {
    match include {
        Some(list) if list.len() > 3 => Err(Error::validation(
            "include takes at most 3 values",
            Some("include"),
        )),
        Some(list) if list.is_empty() => Ok(None),
        other => Ok(other.clone()),
    }
}

fn put_str(map: &mut Map<String, Value>, key: &str, value: &Option<String>) {
    if let Some(v) = value {
        map.insert(key.to_owned(), Value::String(v.clone()));
    }
}

fn put_list<T: AsRef<str>>(map: &mut Map<String, Value>, key: &str, value: &Option<Vec<T>>) {
    if let Some(list) = value {
        map.insert(
            key.to_owned(),
            Value::Array(
                list.iter()
                    .map(|v| Value::String(v.as_ref().to_owned()))
                    .collect(),
            ),
        );
    }
}

fn put_bool(map: &mut Map<String, Value>, key: &str, value: Option<bool>) {
    if let Some(v) = value {
        map.insert(key.to_owned(), Value::Bool(v));
    }
}

/// Parameters of `articles().search()` and `articles().search_all()`.
///
/// ```
/// use finlight::{IncludeBlock, SearchParams};
/// let params = SearchParams::new()
///     .query("ticker:NVDA")
///     .languages(["en"])
///     .include([IncludeBlock::COMPANIES, IncludeBlock::EVENTS])
///     .limit(10);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct SearchParams {
    /// Query language, e.g. `ticker:AAPL AND eventType:earnings`.
    pub query: Option<String>,
    /// Only these source domains.
    pub sources: Option<Vec<String>>,
    /// Non-default sources added to the default pool.
    pub opt_in_sources: Option<Vec<String>>,
    /// Source domains left out.
    pub exclude_sources: Option<Vec<String>>,
    /// Only sources of these types. Articles of unclassified sources never match.
    pub source_types: Option<Vec<SourceType>>,
    /// Source types left out. Articles of unclassified sources are kept.
    pub exclude_source_types: Option<Vec<SourceType>>,
    /// Lower bound of the `order_by` timestamp.
    pub from: Option<DateInput>,
    /// Upper bound of the `order_by` timestamp.
    pub to: Option<DateInput>,
    /// ISO 639-1 codes; not empty. The server default is `["en"]`.
    pub languages: Option<Vec<String>>,
    /// Tickers (`SYMBOL`, `SYMBOL@MIC`, vendor notations).
    pub tickers: Option<Vec<String>>,
    /// Geographic focus of the story (ISO alpha-2).
    pub countries: Option<Vec<String>>,
    /// Categories.
    pub categories: Option<Vec<String>>,
    /// `true` keeps only articles with scraped content.
    pub has_content: Option<bool>,
    /// Company roles (same company as `tickers` and `min_relevance`).
    pub roles: Option<Vec<CompanyRole>>,
    /// Minimum relevance, 0 to 1.
    pub min_relevance: Option<f64>,
    /// Event types (same event as `event_directions` and `min_importance`).
    pub event_types: Option<Vec<EventType>>,
    /// Event directions.
    pub event_directions: Option<Vec<EventDirection>>,
    /// Minimum event importance.
    pub min_importance: Option<Importance>,
    /// GICS sectors.
    pub sectors: Option<Vec<String>>,
    /// Company domicile (ISO alpha-2).
    pub company_countries: Option<Vec<String>>,
    /// Primary-listing MICs.
    pub exchanges: Option<Vec<String>>,
    /// Sort timestamp (and the timestamp `from` and `to` apply to).
    pub order_by: Option<OrderBy>,
    /// Sort direction.
    pub order: Option<SortOrder>,
    /// Page size, 1 to 100. The server default is 20.
    pub limit: Option<u32>,
    /// `pagination.next_cursor` of the previous page.
    pub cursor: Option<String>,
    /// Extra blocks (at most 3).
    pub include: Option<Vec<IncludeBlock>>,
}

impl SearchParams {
    /// Empty parameters (an empty search sends `{}`).
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads parameters from a JSON object with the wire keys (`eventTypes`, `orderBy`, ...).
    /// An unknown key is rejected with `Error::Validation` naming it.
    pub fn from_json(json: &str) -> Result<Self> {
        serde_json::from_str(json).map_err(|e| from_json_error(e, "search"))
    }

    string_setters! {
        /// Query language.
        query,
        /// Cursor of the previous page.
        cursor,
    }

    list_setters! {
        /// Only these source domains.
        sources,
        /// Non-default sources added to the default pool.
        opt_in_sources,
        /// Source domains left out.
        exclude_sources,
        /// ISO 639-1 codes.
        languages,
        /// Tickers.
        tickers,
        /// Story countries.
        countries,
        /// Categories.
        categories,
        /// GICS sectors.
        sectors,
        /// Company domicile countries.
        company_countries,
        /// Primary-listing MICs.
        exchanges,
    }

    vocab_list_setters! {
        /// Only sources of these types (`SourceType::NEWSWIRE`, ...).
        source_types: SourceType,
        /// Source types left out.
        exclude_source_types: SourceType,
        /// Company roles.
        roles: CompanyRole,
        /// Event types.
        event_types: EventType,
        /// Event directions.
        event_directions: EventDirection,
        /// Extra blocks.
        include: IncludeBlock,
    }

    bool_setters! {
        /// Only articles with scraped content.
        has_content,
    }

    /// Lower bound (a `SystemTime`, `DateInput::date(..)` or a string).
    pub fn from(mut self, value: impl Into<DateInput>) -> Self {
        self.from = Some(value.into());
        self
    }

    /// Upper bound.
    pub fn to(mut self, value: impl Into<DateInput>) -> Self {
        self.to = Some(value.into());
        self
    }

    /// Minimum relevance, 0 to 1.
    pub fn min_relevance(mut self, value: f64) -> Self {
        self.min_relevance = Some(value);
        self
    }

    /// Minimum event importance.
    pub fn min_importance(mut self, value: impl Into<Importance>) -> Self {
        self.min_importance = Some(value.into());
        self
    }

    /// Sort timestamp.
    pub fn order_by(mut self, value: impl Into<OrderBy>) -> Self {
        self.order_by = Some(value.into());
        self
    }

    /// Sort direction.
    pub fn order(mut self, value: impl Into<SortOrder>) -> Self {
        self.order = Some(value.into());
        self
    }

    /// Page size, 1 to 100.
    pub fn limit(mut self, value: u32) -> Self {
        self.limit = Some(value);
        self
    }

    /// Checks the shape and builds the JSON body from the fixed key list.
    pub(crate) fn to_body(&self) -> Result<Map<String, Value>> {
        let mut m = Map::new();
        put_str(&mut m, "query", &self.query);
        put_list(&mut m, "sources", &self.sources);
        put_list(&mut m, "optInSources", &self.opt_in_sources);
        put_list(&mut m, "excludeSources", &self.exclude_sources);
        put_list(&mut m, "sourceTypes", &self.source_types);
        put_list(&mut m, "excludeSourceTypes", &self.exclude_source_types);
        if let Some(from) = &self.from {
            m.insert("from".into(), Value::String(from.to_wire("from")?));
        }
        if let Some(to) = &self.to {
            m.insert("to".into(), Value::String(to.to_wire("to")?));
        }
        if let Some(languages) = &self.languages {
            if languages.is_empty() {
                return Err(Error::validation(
                    "languages must not be empty; leave it out for the server default",
                    Some("languages"),
                ));
            }
        }
        put_list(&mut m, "languages", &self.languages);
        put_list(&mut m, "tickers", &self.tickers);
        put_list(&mut m, "countries", &self.countries);
        put_list(&mut m, "categories", &self.categories);
        put_bool(&mut m, "hasContent", self.has_content);
        put_list(&mut m, "roles", &self.roles);
        if let Some(v) = self.min_relevance {
            if !v.is_finite() || !(0.0..=1.0).contains(&v) {
                return Err(Error::validation(
                    "minRelevance must be between 0 and 1",
                    Some("minRelevance"),
                ));
            }
            let number = serde_json::Number::from_f64(v).ok_or_else(|| {
                Error::validation("minRelevance must be a finite number", Some("minRelevance"))
            })?;
            m.insert("minRelevance".into(), Value::Number(number));
        }
        put_list(&mut m, "eventTypes", &self.event_types);
        put_list(&mut m, "eventDirections", &self.event_directions);
        if let Some(v) = &self.min_importance {
            m.insert("minImportance".into(), Value::String(v.as_str().to_owned()));
        }
        put_list(&mut m, "sectors", &self.sectors);
        put_list(&mut m, "companyCountries", &self.company_countries);
        put_list(&mut m, "exchanges", &self.exchanges);
        if let Some(v) = &self.order_by {
            m.insert("orderBy".into(), Value::String(v.as_str().to_owned()));
        }
        if let Some(v) = &self.order {
            m.insert("order".into(), Value::String(v.as_str().to_owned()));
        }
        if let Some(limit) = self.limit {
            if !(1..=100).contains(&limit) {
                return Err(Error::validation(
                    "limit must be between 1 and 100",
                    Some("limit"),
                ));
            }
            m.insert("limit".into(), Value::from(limit));
        }
        if let Some(cursor) = &self.cursor {
            if cursor.is_empty() {
                return Err(Error::validation(
                    "cursor must not be empty",
                    Some("cursor"),
                ));
            }
            m.insert("cursor".into(), Value::String(cursor.clone()));
        }
        put_list(&mut m, "include", &check_include(&self.include)?);
        Ok(m)
    }
}

/// Options of `articles().get()` and `articles().get_by_link()`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct GetArticleOptions {
    /// Extra blocks (at most 3).
    pub include: Option<Vec<IncludeBlock>>,
}

impl GetArticleOptions {
    /// No extra blocks.
    pub fn new() -> Self {
        Self::default()
    }

    /// Extra blocks.
    pub fn include<I, S>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<IncludeBlock>,
    {
        self.include = Some(vocab(values));
        self
    }

    pub(crate) fn include_query(&self) -> Result<Option<String>> {
        Ok(check_include(&self.include)?.map(|list| {
            list.iter()
                .map(IncludeBlock::as_str)
                .collect::<Vec<_>>()
                .join(",")
        }))
    }
}

/// Parameters of `companies().search()`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CompanySearchParams {
    /// Ticker, name, alias, ISIN, `XNAS:AMD` or a vendor notation; at most 200 characters.
    pub q: String,
    /// MIC or alias that narrows the hits; at most 32 characters.
    pub mic: Option<String>,
}

impl CompanySearchParams {
    /// Search for `q`.
    pub fn new(q: impl Into<String>) -> Self {
        CompanySearchParams {
            q: q.into(),
            mic: None,
        }
    }

    /// Narrow the hits to an exchange.
    pub fn mic(mut self, value: impl Into<String>) -> Self {
        self.mic = Some(value.into());
        self
    }

    pub(crate) fn to_query(&self) -> Result<Vec<(&'static str, String)>> {
        let q_len = self.q.chars().count();
        if q_len == 0 {
            return Err(Error::validation("q must not be empty", Some("q")));
        }
        if q_len > 200 {
            return Err(Error::validation(
                "q must be at most 200 characters",
                Some("q"),
            ));
        }
        let mut query = vec![("q", self.q.clone())];
        if let Some(mic) = &self.mic {
            let len = mic.chars().count();
            if len == 0 {
                return Err(Error::validation("mic must not be empty", Some("mic")));
            }
            if len > 32 {
                return Err(Error::validation(
                    "mic must be at most 32 characters",
                    Some("mic"),
                ));
            }
            query.push(("mic", mic.clone()));
        }
        Ok(query)
    }
}

pub(crate) fn check_company_id(id: i64, name: &str) -> Result<i64> {
    if !(1..=(1i64 << 53) - 1).contains(&id) {
        return Err(Error::validation(
            format!("{name} must be an integer from 1 to 2^53-1"),
            Some(name),
        ));
    }
    Ok(id)
}

pub(crate) fn company_ids_query(ids: &[i64]) -> Result<String> {
    if ids.is_empty() || ids.len() > 10 {
        return Err(Error::validation("ids takes 1 to 10 values", Some("ids")));
    }
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        out.push(check_company_id(*id, "ids")?.to_string());
    }
    Ok(out.join(","))
}

/// Options of `articles().search_all()`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SearchAllOptions {
    /// Page limit; default `Config::max_pages` (10).
    pub max_pages: Option<u32>,
}

impl SearchAllOptions {
    /// Default options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Page limit.
    pub fn max_pages(mut self, value: u32) -> Self {
        self.max_pages = Some(value);
        self
    }
}

/// Subscribe parameters of the enriched stream (`client.stream()`).
///
/// Streams keep the subscription vocabulary: one `language`, no flat event or role filters
/// (use `query`, e.g. `eventType:mna`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct StreamParams {
    /// Query (lenient stream grammar).
    pub query: Option<String>,
    /// Only these source domains.
    pub sources: Option<Vec<String>>,
    /// Source domains left out.
    pub exclude_sources: Option<Vec<String>>,
    /// Non-default sources added.
    pub opt_in_sources: Option<Vec<String>>,
    /// One language (ISO 639-1); the server default is `en`.
    pub language: Option<String>,
    /// Tickers.
    pub tickers: Option<Vec<String>>,
    /// Story countries.
    pub countries: Option<Vec<String>>,
    /// Categories.
    pub categories: Option<Vec<String>>,
    /// Only sources of these types. Articles of unclassified sources never match.
    pub source_types: Option<Vec<SourceType>>,
    /// Source types left out. Articles of unclassified sources are kept.
    pub exclude_source_types: Option<Vec<SourceType>>,
    /// Leave out articles without content.
    pub exclude_empty_content: Option<bool>,
    /// Without it the server never sends `article.updated`.
    pub include_updates: Option<bool>,
    /// Extra blocks (at most 3).
    pub include: Option<Vec<IncludeBlock>>,
}

impl StreamParams {
    /// Empty parameters.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads parameters from a JSON object with the wire keys; unknown keys are rejected.
    pub fn from_json(json: &str) -> Result<Self> {
        serde_json::from_str(json).map_err(|e| from_json_error(e, "stream"))
    }

    string_setters! {
        /// Query.
        query,
        /// One language.
        language,
    }

    list_setters! {
        /// Only these source domains.
        sources,
        /// Source domains left out.
        exclude_sources,
        /// Non-default sources added.
        opt_in_sources,
        /// Tickers.
        tickers,
        /// Story countries.
        countries,
        /// Categories.
        categories,
    }

    vocab_list_setters! {
        /// Only sources of these types (`SourceType::NEWSWIRE`, ...).
        source_types: SourceType,
        /// Source types left out.
        exclude_source_types: SourceType,
        /// Extra blocks.
        include: IncludeBlock,
    }

    bool_setters! {
        /// Leave out articles without content.
        exclude_empty_content,
        /// Also deliver `article.updated`.
        include_updates,
    }

    pub(crate) fn to_frame(&self) -> Result<Map<String, Value>> {
        let mut m = Map::new();
        m.insert("payloadVersion".into(), Value::from(PAYLOAD_VERSION));
        put_list(&mut m, "include", &check_include(&self.include)?);
        put_str(&mut m, "query", &self.query);
        put_list(&mut m, "sources", &self.sources);
        put_list(&mut m, "excludeSources", &self.exclude_sources);
        put_list(&mut m, "optInSources", &self.opt_in_sources);
        put_str(&mut m, "language", &self.language);
        put_list(&mut m, "tickers", &self.tickers);
        put_list(&mut m, "countries", &self.countries);
        put_list(&mut m, "categories", &self.categories);
        put_list(&mut m, "sourceTypes", &self.source_types);
        put_list(&mut m, "excludeSourceTypes", &self.exclude_source_types);
        put_bool(&mut m, "excludeEmptyContent", self.exclude_empty_content);
        put_bool(&mut m, "includeUpdates", self.include_updates);
        Ok(m)
    }
}

/// Subscribe parameters of the raw stream (`client.raw_stream()`). Raw has no `include`:
/// it delivers the article skeleton only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct RawStreamParams {
    /// Query.
    pub query: Option<String>,
    /// Only these source domains.
    pub sources: Option<Vec<String>>,
    /// Source domains left out.
    pub exclude_sources: Option<Vec<String>>,
    /// Only sources of these types. Articles of unclassified sources never match.
    pub source_types: Option<Vec<SourceType>>,
    /// Source types left out. Articles of unclassified sources are kept.
    pub exclude_source_types: Option<Vec<SourceType>>,
    /// One language.
    pub language: Option<String>,
    /// Also deliver `article.updated`.
    pub include_updates: Option<bool>,
}

impl RawStreamParams {
    /// Empty parameters.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads parameters from a JSON object with the wire keys; unknown keys (`include` too)
    /// are rejected.
    pub fn from_json(json: &str) -> Result<Self> {
        serde_json::from_str(json).map_err(|e| {
            let err = from_json_error(e, "raw stream");
            match &err {
                Error::Validation(v) if v.param() == Some("include") => Error::validation(
                    "raw streams have no include: raw delivers the article skeleton only",
                    Some("include"),
                ),
                _ => err,
            }
        })
    }

    string_setters! {
        /// Query.
        query,
        /// One language.
        language,
    }

    list_setters! {
        /// Only these source domains.
        sources,
        /// Source domains left out.
        exclude_sources,
    }

    vocab_list_setters! {
        /// Only sources of these types (`SourceType::NEWSWIRE`, ...).
        source_types: SourceType,
        /// Source types left out.
        exclude_source_types: SourceType,
    }

    bool_setters! {
        /// Also deliver `article.updated`.
        include_updates,
    }

    pub(crate) fn to_frame(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("payloadVersion".into(), Value::from(PAYLOAD_VERSION));
        put_str(&mut m, "query", &self.query);
        put_list(&mut m, "sources", &self.sources);
        put_list(&mut m, "excludeSources", &self.exclude_sources);
        put_list(&mut m, "sourceTypes", &self.source_types);
        put_list(&mut m, "excludeSourceTypes", &self.exclude_source_types);
        put_str(&mut m, "language", &self.language);
        put_bool(&mut m, "includeUpdates", self.include_updates);
        m
    }
}

/// Percent-encodes a query component like `encodeURIComponent`.
pub(crate) fn encode_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Encodes query pairs. Lists are joined with commas into one parameter (never `key[]=`).
pub(crate) fn encode_query(pairs: &[(&str, String)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", encode_component(k), encode_component(v)))
        .collect::<Vec<_>>()
        .join("&")
}
