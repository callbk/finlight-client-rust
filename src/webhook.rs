use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::error::WebhookVerificationError;
use crate::models::Article;
use crate::models::flex::parse_datetime;

const SIGNATURE_PREFIX: &str = "sha256=";
const REPLAY_TOLERANCE_SECS: i64 = 5 * 60;

/// Verifies a finlight webhook and returns the contained article.
///
/// `raw_body` must be the unmodified request body. `signature` is the value
/// of the `X-Webhook-Signature` header (with or without the `sha256=`
/// prefix). `endpoint_secret` is your webhook secret from the finlight
/// dashboard. `timestamp` is the `X-Webhook-Timestamp` header; pass `None` if
/// the webhook has none, otherwise it is included in the signed message and
/// checked against a 5-minute replay tolerance.
pub fn construct_webhook_event(
    raw_body: &[u8],
    signature: &str,
    endpoint_secret: &str,
    timestamp: Option<&str>,
) -> Result<Article, WebhookVerificationError> {
    let signature = signature
        .strip_prefix(SIGNATURE_PREFIX)
        .unwrap_or(signature);
    let signature = hex::decode(signature)
        .map_err(|_| WebhookVerificationError("Invalid webhook signature".into()))?;

    let mut mac = Hmac::<Sha256>::new_from_slice(endpoint_secret.as_bytes())
        .expect("HMAC accepts keys of any length");
    if let Some(ts) = timestamp {
        mac.update(ts.as_bytes());
        mac.update(b".");
    }
    mac.update(raw_body);
    mac.verify_slice(&signature)
        .map_err(|_| WebhookVerificationError("Invalid webhook signature".into()))?;

    if let Some(ts) = timestamp {
        verify_timestamp(ts)?;
    }

    serde_json::from_slice(raw_body)
        .map_err(|_| WebhookVerificationError("Invalid JSON payload".into()))
}

fn verify_timestamp(timestamp: &str) -> Result<(), WebhookVerificationError> {
    let parsed: DateTime<Utc> = parse_datetime(timestamp)
        .ok_or_else(|| WebhookVerificationError("Invalid timestamp format".into()))?;
    let age = Utc::now().signed_duration_since(parsed).num_seconds();
    if age.abs() > REPLAY_TOLERANCE_SECS {
        return Err(WebhookVerificationError(
            "Webhook timestamp outside allowed tolerance".into(),
        ));
    }
    Ok(())
}
