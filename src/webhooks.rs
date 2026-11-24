//! Verifies and reads incoming finlight webhooks on your server.
//!
//! Always pass the raw body bytes exactly as received; never re-serialize parsed JSON.
//! Answer within 5 s with a 2xx and do slow work afterwards, dedupe on `event_id`, answer a
//! [`WrongPayloadVersion`] with a 2xx too, and never answer with 410 (it disables the webhook).

use std::collections::{BTreeMap, HashMap};
use std::hash::BuildHasher;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use serde_json::Value;
use serde_json::value::RawValue;
use sha2::Sha256;

use crate::error::{Error, Result, WebhookVerificationReason as Reason};
use crate::models::{Article, decode_article, decode_value};
use crate::time::{parse_opt, parse_rfc3339_nanos};
use crate::vocab::WebhookEventName;

/// Default allowed clock difference between the signature timestamp and now: 300 s.
pub const DEFAULT_TOLERANCE: Duration = Duration::from_secs(300);

const SIGNATURE_HEADER: &str = "x-webhook-signature";
const TIMESTAMP_HEADER: &str = "x-webhook-timestamp";
const SIGNATURE_PREFIX: &str = "sha256=";

/// Options of the webhook functions.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct WebhookVerifyOptions {
    /// Allowed clock difference, in both directions. Default [`DEFAULT_TOLERANCE`].
    pub tolerance: Duration,
    /// The time to verify against; `None` uses the system clock.
    pub now: Option<SystemTime>,
}

impl Default for WebhookVerifyOptions {
    fn default() -> Self {
        WebhookVerifyOptions {
            tolerance: DEFAULT_TOLERANCE,
            now: None,
        }
    }
}

impl WebhookVerifyOptions {
    /// Default options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Allowed clock difference.
    pub fn tolerance(mut self, value: Duration) -> Self {
        self.tolerance = value;
        self
    }

    /// Verify against this time instead of the system clock.
    pub fn now(mut self, value: SystemTime) -> Self {
        self.now = Some(value);
        self
    }
}

/// A header collection. Names are matched case-insensitively, the CGI form
/// (`HTTP_X_WEBHOOK_TIMESTAMP`) included.
///
/// Implemented for `http::HeaderMap`, `HashMap` and `BTreeMap` of strings, and slices,
/// arrays and `Vec`s of `(name, value)` string pairs.
pub trait WebhookHeaders {
    /// Every value of the header `name` (given in lowercase). `None` marks a value that is not
    /// valid text.
    fn header_values(&self, name: &str) -> Vec<Option<String>>;
}

fn name_matches(key: &str, name: &str) -> bool {
    if key.eq_ignore_ascii_case(name) {
        return true;
    }
    let Some(rest) = key
        .get(..5)
        .filter(|p| p.eq_ignore_ascii_case("http_"))
        .map(|_| &key[5..])
    else {
        return false;
    };
    rest.len() == name.len()
        && rest
            .bytes()
            .zip(name.bytes())
            .all(|(a, b)| a.to_ascii_lowercase() == if b == b'-' { b'_' } else { b })
}

fn pairs_values<'a, K, V>(
    pairs: impl Iterator<Item = (&'a K, &'a V)>,
    name: &str,
) -> Vec<Option<String>>
where
    K: AsRef<str> + 'a + ?Sized,
    V: AsRef<str> + 'a + ?Sized,
{
    pairs
        .filter(|(k, _)| name_matches(k.as_ref(), name))
        .map(|(_, v)| Some(v.as_ref().to_owned()))
        .collect()
}

impl WebhookHeaders for http::HeaderMap {
    fn header_values(&self, name: &str) -> Vec<Option<String>> {
        self.iter()
            .filter(|(k, _)| name_matches(k.as_str(), name))
            .map(|(_, v)| v.to_str().ok().map(str::to_owned))
            .collect()
    }
}

impl<K: AsRef<str>, V: AsRef<str>, S: BuildHasher> WebhookHeaders for HashMap<K, V, S> {
    fn header_values(&self, name: &str) -> Vec<Option<String>> {
        pairs_values(self.iter(), name)
    }
}

impl<K: AsRef<str>, V: AsRef<str>> WebhookHeaders for BTreeMap<K, V> {
    fn header_values(&self, name: &str) -> Vec<Option<String>> {
        pairs_values(self.iter(), name)
    }
}

impl<K: AsRef<str>, V: AsRef<str>> WebhookHeaders for [(K, V)] {
    fn header_values(&self, name: &str) -> Vec<Option<String>> {
        pairs_values(self.iter().map(|(k, v)| (k, v)), name)
    }
}

impl<K: AsRef<str>, V: AsRef<str>, const N: usize> WebhookHeaders for [(K, V); N] {
    fn header_values(&self, name: &str) -> Vec<Option<String>> {
        self.as_slice().header_values(name)
    }
}

impl<K: AsRef<str>, V: AsRef<str>> WebhookHeaders for Vec<(K, V)> {
    fn header_values(&self, name: &str) -> Vec<Option<String>> {
        self.as_slice().header_values(name)
    }
}

impl<T: WebhookHeaders + ?Sized> WebhookHeaders for &T {
    fn header_values(&self, name: &str) -> Vec<Option<String>> {
        (**self).header_values(name)
    }
}

/// A verified webhook delivery in payload version 3.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct WebhookEvent {
    /// Idempotency key: the same on every retry of the same event. Dedupe on it.
    pub event_id: String,
    /// `article.created` or `article.updated` (open).
    pub event: WebhookEventName,
    /// When the event happened (stable across retries), as sent.
    pub occurred_at: String,
    /// Always 3.
    pub payload_version: u32,
    /// The webhook id.
    pub subscription_id: String,
    /// Changed fields of an `article.updated`; `None` when the key is absent.
    pub changed_fields: Option<Vec<String>>,
    /// The article.
    pub data: Article,
    raw: Arc<str>,
}

impl WebhookEvent {
    /// `true` for the dashboard test webhook (`event_id` starts with `evt_test`).
    pub fn is_test(&self) -> bool {
        self.event_id.starts_with("evt_test")
    }

    /// `occurred_at` parsed strictly.
    pub fn occurred_at_time(&self) -> Option<SystemTime> {
        parse_opt(Some(&self.occurred_at))
    }

    /// The body exactly as received.
    pub fn raw_json(&self) -> &str {
        &self.raw
    }
}

/// A correctly signed body in another payload version (e.g. retries in the v2 format for
/// about 2 h after a webhook was switched to v3). It is a result, not an error: answer it with
/// a 2xx and keep `raw` if you need the data.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct WrongPayloadVersion {
    /// Version by the server's rule: 3 only for the JSON number 3, otherwise 2.
    pub detected_version: u32,
    /// `payloadVersion` as received (JSON text, e.g. `"3"` with quotes); `None` when absent.
    pub payload_version: Option<String>,
    /// Whether the body had a `payloadVersion` key.
    pub payload_version_present: bool,
    /// The parsed body as JSON text.
    pub raw: String,
    /// The article link, when it can be read.
    pub link: Option<String>,
}

impl WrongPayloadVersion {
    /// The body as JSON text.
    pub fn raw_json(&self) -> &str {
        &self.raw
    }
}

/// Result of [`construct_event`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
#[allow(clippy::large_enum_variant)]
pub enum WebhookPayload {
    /// A v3 event.
    Event(WebhookEvent),
    /// A correctly signed body in another payload version; answer it with a 2xx.
    WrongPayloadVersion(WrongPayloadVersion),
}

fn single_header(
    headers: &(impl WebhookHeaders + ?Sized),
    name: &str,
    label: &str,
) -> Result<Option<String>> {
    let values = headers.header_values(name);
    match values.as_slice() {
        [] => Ok(None),
        [None] => Err(Error::webhook(
            Reason::NonAsciiHeader,
            format!("the {label} header contains characters outside printable ASCII"),
        )),
        [Some(v)] if v.contains(',') => Err(Error::webhook(
            Reason::DuplicateHeader,
            format!("the {label} header appears more than once"),
        )),
        [Some(v)] => Ok(Some(v.clone())),
        _ => Err(Error::webhook(
            Reason::DuplicateHeader,
            format!("the {label} header appears more than once"),
        )),
    }
}

fn printable_ascii(value: &str) -> bool {
    value.bytes().all(|b| (0x20..=0x7e).contains(&b))
}

/// Verifies the signature and the timestamp of a raw body. `Ok(())` when both pass.
///
/// The HMAC-SHA256 runs over `timestamp + "." + raw_body`, keyed with the UTF-8 bytes of the
/// secret (not hex-decoded), and is compared in constant time with the hex after `sha256=`.
pub fn verify_signature(
    raw_body: impl AsRef<[u8]>,
    signature: &str,
    timestamp: &str,
    secret: &str,
    options: &WebhookVerifyOptions,
) -> Result<()> {
    verify(
        raw_body.as_ref(),
        Some(signature),
        Some(timestamp),
        secret,
        options,
    )
}

fn verify(
    body: &[u8],
    signature: Option<&str>,
    timestamp: Option<&str>,
    secret: &str,
    options: &WebhookVerifyOptions,
) -> Result<()> {
    if secret.is_empty() {
        return Err(Error::webhook(
            Reason::InvalidSecret,
            "the webhook secret must not be empty",
        ));
    }
    let signature = match signature {
        Some(s) if !s.is_empty() => s,
        _ => {
            return Err(Error::webhook(
                Reason::MissingSignature,
                "the X-Webhook-Signature header is missing",
            ));
        }
    };
    let timestamp = match timestamp {
        Some(t) if !t.is_empty() => t,
        _ => {
            return Err(Error::webhook(
                Reason::MissingTimestamp,
                "the X-Webhook-Timestamp header is missing",
            ));
        }
    };
    if !printable_ascii(signature) || !printable_ascii(timestamp) {
        return Err(Error::webhook(
            Reason::NonAsciiHeader,
            "a webhook header contains characters outside printable ASCII",
        ));
    }
    let Some(given) = signature.strip_prefix(SIGNATURE_PREFIX) else {
        return Err(Error::webhook(
            Reason::UnsupportedSignatureScheme,
            "the X-Webhook-Signature header must start with \"sha256=\"",
        ));
    };
    let Some(signed_at) = parse_rfc3339_nanos(timestamp) else {
        return Err(Error::webhook(
            Reason::UnparseableTimestamp,
            "the X-Webhook-Timestamp header is not an RFC 3339 timestamp",
        ));
    };
    let mismatch = || {
        Error::webhook(
            Reason::SignatureMismatch,
            "the webhook signature does not match",
        )
    };
    let given = hex::decode(given).map_err(|_| mismatch())?;
    let mut mac =
        <Hmac<Sha256> as KeyInit>::new_from_slice(secret.as_bytes()).map_err(|_| mismatch())?;
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(body);
    // Length check and constant-time comparison.
    mac.verify_slice(&given).map_err(|_| mismatch())?;

    let now = options.now.unwrap_or_else(SystemTime::now);
    let now_nanos: i128 = match now.duration_since(UNIX_EPOCH) {
        Ok(d) => i128::try_from(d.as_nanos()).unwrap_or(i128::MAX),
        Err(e) => -i128::try_from(e.duration().as_nanos()).unwrap_or(i128::MAX),
    };
    let tolerance = i128::try_from(options.tolerance.as_nanos()).unwrap_or(i128::MAX);
    if (now_nanos - signed_at).abs() > tolerance {
        return Err(Error::webhook(
            Reason::TimestampOutsideTolerance,
            "the webhook timestamp is outside the allowed tolerance",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Envelope {
    event_id: String,
    event: WebhookEventName,
    occurred_at: String,
    subscription_id: String,
    #[serde(default)]
    changed_fields: Option<Vec<String>>,
    data: Box<RawValue>,
}

fn read_event(body: &[u8]) -> Result<WebhookPayload> {
    let text = std::str::from_utf8(body).map_err(|e| {
        Error::decode(
            "the verified webhook body is not UTF-8",
            None,
            None,
            Some(Box::new(e)),
        )
    })?;
    let json: Value = serde_json::from_str(text).map_err(|e| {
        Error::decode(
            "the verified webhook body is not JSON",
            None,
            None,
            Some(Box::new(e)),
        )
    })?;
    let Value::Object(obj) = &json else {
        return Err(Error::decode(
            "the verified webhook body is not a JSON object",
            None,
            Some(text.to_owned()),
            None,
        ));
    };
    let version = obj.get("payloadVersion");
    let is_v3 = matches!(version, Some(Value::Number(n)) if n.as_f64() == Some(3.0));
    if !is_v3 {
        let link = obj
            .get("link")
            .and_then(Value::as_str)
            .or_else(|| {
                obj.get("data")
                    .and_then(|d| d.get("link"))
                    .and_then(Value::as_str)
            })
            .map(str::to_owned);
        return Ok(WebhookPayload::WrongPayloadVersion(WrongPayloadVersion {
            detected_version: 2,
            payload_version: version.map(Value::to_string),
            payload_version_present: version.is_some(),
            raw: json.to_string(),
            link,
        }));
    }
    let env: Envelope = decode_value(text, "")?;
    let data = decode_article(&env.data, "data")?;
    Ok(WebhookPayload::Event(WebhookEvent {
        event_id: env.event_id,
        event: env.event,
        occurred_at: env.occurred_at,
        payload_version: 3,
        subscription_id: env.subscription_id,
        changed_fields: env.changed_fields,
        data,
        raw: Arc::from(text),
    }))
}

/// Verifies a webhook and reads it.
///
/// Order: signature, timestamp, JSON, `payloadVersion`, envelope. Returns
/// [`WebhookPayload::Event`], or [`WebhookPayload::WrongPayloadVersion`] for a correctly signed
/// body in another payload version. Fails with `Error::WebhookVerification` when verification
/// fails (answer 400) and with `Error::Decode` when a verified body cannot be read.
///
/// ```
/// use finlight::webhooks::{self, WebhookPayload, WebhookVerifyOptions};
///
/// fn handle(body: &[u8], headers: &http::HeaderMap, secret: &str) -> u16 {
///     match webhooks::construct_event(body, headers, secret, &WebhookVerifyOptions::new()) {
///         Ok(WebhookPayload::Event(event)) => {
///             println!("{} {}", event.event_id, event.data.title);
///             200
///         }
///         Ok(_) => 200, // WrongPayloadVersion and future outcomes: answer 2xx
///         Err(_) => 400, // never 410
///     }
/// }
/// ```
pub fn construct_event<H: WebhookHeaders + ?Sized>(
    raw_body: impl AsRef<[u8]>,
    headers: &H,
    secret: &str,
    options: &WebhookVerifyOptions,
) -> Result<WebhookPayload> {
    let signature = single_header(headers, SIGNATURE_HEADER, "X-Webhook-Signature")?;
    let timestamp = single_header(headers, TIMESTAMP_HEADER, "X-Webhook-Timestamp")?;
    construct_event_from_values(
        raw_body,
        signature.as_deref(),
        timestamp.as_deref(),
        secret,
        options,
    )
}

/// Same as [`construct_event`], with the two header values passed directly
/// (`X-Webhook-Signature`, `X-Webhook-Timestamp`).
pub fn construct_event_from_values(
    raw_body: impl AsRef<[u8]>,
    signature: Option<&str>,
    timestamp: Option<&str>,
    secret: &str,
    options: &WebhookVerifyOptions,
) -> Result<WebhookPayload> {
    let body = raw_body.as_ref();
    verify(body, signature, timestamp, secret, options)?;
    read_event(body)
}
