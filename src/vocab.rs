//! Vocabularies of the API. Every type here is open: known values have a variant and a
//! string constant (`CompanyRole::ACQUIRER`), and any other string is kept as received in
//! `Other(String)`. Compare by string (`role == CompanyRole::ACQUIRER` or `role.as_str()`);
//! a known value received from the wire always decodes to its named variant.

macro_rules! open_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident => $konst:ident = $value:literal, )*
        }
        $( subsets { $( $(#[$smeta:meta])* $sname:ident = [ $($sv:ident),* ]; )* } )?
    ) => {
        $(#[$meta])*
        #[derive(Clone, serde::Serialize, serde::Deserialize)]
        #[serde(from = "String", into = "String")]
        #[non_exhaustive]
        pub enum $name {
            $( $(#[$vmeta])* #[doc = concat!("`", $value, "`")] $variant, )*
            /// A value this SDK version does not know, kept exactly as received.
            Other(String),
        }

        #[allow(missing_docs)]
        impl $name {
            $( pub const $konst: &'static str = $value; )*
            /// Every value this SDK version knows.
            pub const ALL: &'static [&'static str] = &[ $( $value ),* ];
            $( $( $(#[$smeta])* pub const $sname: &'static [&'static str] = &[ $( Self::$sv ),* ]; )* )?

            /// The value as sent on the wire.
            pub fn as_str(&self) -> &str {
                match self {
                    $( Self::$variant => $value, )*
                    Self::Other(value) => value.as_str(),
                }
            }

            /// `true` when this SDK version knows the value.
            pub fn is_known(&self) -> bool {
                !matches!(self, Self::Other(_))
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                match value {
                    $( $value => Self::$variant, )*
                    other => Self::Other(other.to_owned()),
                }
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                match value.as_str() {
                    $( $value => Self::$variant, )*
                    _ => Self::Other(value),
                }
            }
        }

        impl From<&String> for $name {
            fn from(value: &String) -> Self {
                Self::from(value.as_str())
            }
        }

        impl From<$name> for String {
            #[allow(unreachable_patterns)]
            fn from(value: $name) -> Self {
                match value {
                    $name::Other(value) => value,
                    known => known.as_str().to_owned(),
                }
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}({:?})", stringify!($name), self.as_str())
            }
        }

        impl PartialEq for $name {
            fn eq(&self, other: &Self) -> bool {
                self.as_str() == other.as_str()
            }
        }

        impl Eq for $name {}

        impl std::hash::Hash for $name {
            fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                self.as_str().hash(state);
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.as_str() == other
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.as_str() == *other
            }
        }

        impl PartialEq<$name> for &str {
            fn eq(&self, other: &$name) -> bool {
                *self == other.as_str()
            }
        }
    };
}

open_enum! {
    /// A block of `include`: extra data a request asks for (`content`, `companies`, `events`).
    pub enum IncludeBlock {
        Content => CONTENT = "content",
        Companies => COMPANIES = "companies",
        Events => EVENTS = "events",
    }
}

open_enum! {
    /// Timestamp `articles.search` sorts by; `from` and `to` apply to the same timestamp.
    pub enum OrderBy {
        PublishedAt => PUBLISHED_AT = "publishedAt",
        FirstSeenAt => FIRST_SEEN_AT = "firstSeenAt",
        RevisedAt => REVISED_AT = "revisedAt",
        AvailableAt => AVAILABLE_AT = "availableAt",
    }
}

open_enum! {
    /// Sort direction.
    pub enum SortOrder {
        Asc => ASC = "asc",
        Desc => DESC = "desc",
    }
}

open_enum! {
    /// Position of a company in the story (not price impact).
    pub enum CompanyRole {
        Acquirer => ACQUIRER = "acquirer",
        Target => TARGET = "target",
        Seller => SELLER = "seller",
        Defendant => DEFENDANT = "defendant",
        Subject => SUBJECT = "subject",
        Counterparty => COUNTERPARTY = "counterparty",
        Supplier => SUPPLIER = "supplier",
        Customer => CUSTOMER = "customer",
        Competitor => COMPETITOR = "competitor",
        Mentioned => MENTIONED = "mentioned",
    }
}

open_enum! {
    /// Type of a tagged event.
    pub enum EventType {
        Earnings => EARNINGS = "earnings",
        Guidance => GUIDANCE = "guidance",
        Mna => MNA = "mna",
        IpoListing => IPO_LISTING = "ipo_listing",
        CapitalAction => CAPITAL_ACTION = "capital_action",
        RatingAction => RATING_ACTION = "rating_action",
        ManagementChange => MANAGEMENT_CHANGE = "management_change",
        Legal => LEGAL = "legal",
        Regulatory => REGULATORY = "regulatory",
        BankruptcyDistress => BANKRUPTCY_DISTRESS = "bankruptcy_distress",
        ProductPartnership => PRODUCT_PARTNERSHIP = "product_partnership",
        InvestmentExpansion => INVESTMENT_EXPANSION = "investment_expansion",
        OperationalIncident => OPERATIONAL_INCIDENT = "operational_incident",
        LaborRestructuring => LABOR_RESTRUCTURING = "labor_restructuring",
        Macro => MACRO = "macro",
        Geopolitics => GEOPOLITICS = "geopolitics",
    }
}

open_enum! {
    /// Direction of a tagged event.
    pub enum EventDirection {
        Positive => POSITIVE = "positive",
        Negative => NEGATIVE = "negative",
        Neutral => NEUTRAL = "neutral",
        Unclear => UNCLEAR = "unclear",
    }
}

open_enum! {
    /// Importance of a tagged event. Ordered: `low` < `medium` < `high` < `critical`.
    pub enum Importance {
        Low => LOW = "low",
        Medium => MEDIUM = "medium",
        High => HIGH = "high",
        Critical => CRITICAL = "critical",
    }
}

impl Importance {
    /// Position in the known order (`low` = 0); `None` for a value this SDK does not know.
    pub fn rank(&self) -> Option<usize> {
        Self::ALL.iter().position(|v| *v == self.as_str())
    }
}

open_enum! {
    /// Sentiment label.
    pub enum SentimentLabel {
        Positive => POSITIVE = "positive",
        Negative => NEGATIVE = "negative",
        Neutral => NEUTRAL = "neutral",
    }
}

open_enum! {
    /// Type of a source: `source.type` of an article, `type` of `sources().list()`, and the
    /// values of the `source_types` / `exclude_source_types` search filters.
    pub enum SourceType {
        Newswire => NEWSWIRE = "newswire",
        PressRelease => PRESS_RELEASE = "press_release",
        FinancialMedia => FINANCIAL_MEDIA = "financial_media",
        GeneralMedia => GENERAL_MEDIA = "general_media",
        Aggregator => AGGREGATOR = "aggregator",
        Blog => BLOG = "blog",
        Government => GOVERNMENT = "government",
    }
}

open_enum! {
    /// `event` of a WebSocket frame.
    pub enum StreamEventName {
        ArticleCreated => ARTICLE_CREATED = "article.created",
        ArticleUpdated => ARTICLE_UPDATED = "article.updated",
        ConnectionAdmitted => CONNECTION_ADMITTED = "connection.admitted",
        ConnectionPong => CONNECTION_PONG = "connection.pong",
        ConnectionPreempted => CONNECTION_PREEMPTED = "connection.preempted",
        ConnectionError => CONNECTION_ERROR = "connection.error",
    }
    subsets {
        /// The article events (`websocket.articleEvents` in the contract kit).
        ARTICLE_EVENTS = [ARTICLE_CREATED, ARTICLE_UPDATED];
        /// The control events (`websocket.connectionEvents` in the contract kit).
        CONNECTION_EVENTS = [CONNECTION_ADMITTED, CONNECTION_PONG, CONNECTION_PREEMPTED, CONNECTION_ERROR];
    }
}

open_enum! {
    /// `code` of a `connection.error` frame.
    pub enum ConnectionErrorCode {
        PingOnDefaultRoute => PING_ON_DEFAULT_ROUTE = "PING_ON_DEFAULT_ROUTE",
        HandshakeFailed => HANDSHAKE_FAILED = "HANDSHAKE_FAILED",
        ValidationFailed => VALIDATION_FAILED = "VALIDATION_FAILED",
        SearchFailed => SEARCH_FAILED = "SEARCH_FAILED",
        InternalError => INTERNAL_ERROR = "INTERNAL_ERROR",
    }
}

open_enum! {
    /// `reason` of a `connection.preempted` frame.
    pub enum PreemptReason {
        ConnectionLimit => CONNECTION_LIMIT = "connection_limit",
        QuotaExceeded => QUOTA_EXCEEDED = "quota_exceeded",
    }
}

open_enum! {
    /// `event` of a webhook delivery.
    pub enum WebhookEventName {
        ArticleCreated => ARTICLE_CREATED = "article.created",
        ArticleUpdated => ARTICLE_UPDATED = "article.updated",
    }
}

open_enum! {
    /// `code` of an API error (problem+json).
    pub enum ProblemCode {
        ValidationFailed => VALIDATION_FAILED = "VALIDATION_FAILED",
        CursorInvalid => CURSOR_INVALID = "CURSOR_INVALID",
        CursorMismatch => CURSOR_MISMATCH = "CURSOR_MISMATCH",
        Unauthorized => UNAUTHORIZED = "UNAUTHORIZED",
        Forbidden => FORBIDDEN = "FORBIDDEN",
        NotFound => NOT_FOUND = "NOT_FOUND",
        RateLimited => RATE_LIMITED = "RATE_LIMITED",
        InternalError => INTERNAL_ERROR = "INTERNAL_ERROR",
    }
}

open_enum! {
    /// Which limit a 429 hit. `unknown` when the server did not say and the detail text did not tell.
    pub enum RateLimitType {
        Monthly => MONTHLY = "monthly",
        Burst => BURST = "burst",
        Unknown => UNKNOWN = "unknown",
    }
}

open_enum! {
    /// Why a stream opened a new connection (see `ReconnectInfo`).
    pub enum ReconnectReason {
        Rotation => ROTATION = "rotation",
        Watchdog => WATCHDOG = "watchdog",
        Closed => CLOSED = "closed",
        HandshakeFailed => HANDSHAKE_FAILED = "handshake_failed",
        SearchFailed => SEARCH_FAILED = "search_failed",
        DialFailed => DIAL_FAILED = "dial_failed",
        DialRefused => DIAL_REFUSED = "dial_refused",
        ConnectionLimit => CONNECTION_LIMIT = "connection_limit",
        RateLimited => RATE_LIMITED = "rate_limited",
        QuotaExceeded => QUOTA_EXCEEDED = "quota_exceeded",
    }
}
