//! Webhook verification tests: HMAC-SHA256 signatures with and without
//! timestamp binding, prefix handling, replay tolerance, and payload
//! validation.

use chrono::{Duration, SecondsFormat, Utc};
use finlight_client::construct_webhook_event;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

const SECRET: &str = "whsec_test_secret";

fn body() -> Vec<u8> {
    serde_json::json!({
        "link": "https://example.com/a",
        "title": "T",
        "publishDate": "2024-01-01T00:00:00Z",
        "source": "example.com",
        "language": "en",
    })
    .to_string()
    .into_bytes()
}

fn sign(message: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(message);
    hex::encode(mac.finalize().into_bytes())
}

#[test]
fn accepts_valid_signature_without_timestamp() {
    let body = body();
    let article = construct_webhook_event(&body, &sign(&body), SECRET, None).unwrap();
    assert_eq!(article.link, "https://example.com/a");
}

#[test]
fn accepts_sha256_prefix() {
    let body = body();
    let signature = format!("sha256={}", sign(&body));
    assert!(construct_webhook_event(&body, &signature, SECRET, None).is_ok());
}

#[test]
fn accepts_timestamped_signature() {
    let body = body();
    let ts = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let signature = sign(format!("{}.{}", ts, String::from_utf8_lossy(&body)).as_bytes());
    assert!(construct_webhook_event(&body, &signature, SECRET, Some(&ts)).is_ok());
}

#[test]
fn rejects_wrong_signature() {
    let body = body();
    let err = construct_webhook_event(&body, &sign(b"other message"), SECRET, None).unwrap_err();
    assert_eq!(err.to_string(), "Invalid webhook signature");
}

#[test]
fn rejects_non_hex_signature() {
    let body = body();
    let err = construct_webhook_event(&body, "not-hex!", SECRET, None).unwrap_err();
    assert_eq!(err.to_string(), "Invalid webhook signature");
}

#[test]
fn rejects_signature_not_bound_to_timestamp() {
    // A signature over the body alone must not pass once a timestamp is
    // supplied — the timestamp is part of the signed message.
    let body = body();
    let ts = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let err = construct_webhook_event(&body, &sign(&body), SECRET, Some(&ts)).unwrap_err();
    assert_eq!(err.to_string(), "Invalid webhook signature");
}

#[test]
fn rejects_stale_timestamp() {
    let body = body();
    let ts = (Utc::now() - Duration::minutes(10)).to_rfc3339_opts(SecondsFormat::Millis, true);
    let signature = sign(format!("{}.{}", ts, String::from_utf8_lossy(&body)).as_bytes());
    let err = construct_webhook_event(&body, &signature, SECRET, Some(&ts)).unwrap_err();
    assert_eq!(
        err.to_string(),
        "Webhook timestamp outside allowed tolerance"
    );
}

#[test]
fn rejects_future_timestamp() {
    let body = body();
    let ts = (Utc::now() + Duration::minutes(10)).to_rfc3339_opts(SecondsFormat::Millis, true);
    let signature = sign(format!("{}.{}", ts, String::from_utf8_lossy(&body)).as_bytes());
    assert!(construct_webhook_event(&body, &signature, SECRET, Some(&ts)).is_err());
}

#[test]
fn rejects_invalid_timestamp_format() {
    let body = body();
    let ts = "not a timestamp";
    let signature = sign(format!("{}.{}", ts, String::from_utf8_lossy(&body)).as_bytes());
    let err = construct_webhook_event(&body, &signature, SECRET, Some(ts)).unwrap_err();
    assert_eq!(err.to_string(), "Invalid timestamp format");
}

#[test]
fn accepts_python_isoformat_timestamp() {
    // Webhook timestamps may arrive in Python isoformat (no zone suffix).
    let body = body();
    let ts = Utc::now()
        .naive_utc()
        .format("%Y-%m-%dT%H:%M:%S%.6f")
        .to_string();
    let signature = sign(format!("{}.{}", ts, String::from_utf8_lossy(&body)).as_bytes());
    assert!(construct_webhook_event(&body, &signature, SECRET, Some(&ts)).is_ok());
}

#[test]
fn rejects_invalid_json_payload() {
    let body = b"not json";
    let err = construct_webhook_event(body, &sign(body), SECRET, None).unwrap_err();
    assert_eq!(err.to_string(), "Invalid JSON payload");
}
