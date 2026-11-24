//! W-01 .. W-16 and the webhook part of F-04.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use finlight::webhooks::{self, WebhookPayload, WebhookVerifyOptions};
use finlight::{Error, WebhookEvent};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::Value;
use sha2::Sha256;

use crate::common::load;

pub fn at(ts: &str) -> SystemTime {
    // Test-side parsing of fixture clocks (ISO with milliseconds and Z).
    let date = &ts[..10];
    let time = &ts[11..23];
    let (y, m, d): (i64, i64, i64) = (
        date[..4].parse().unwrap(),
        date[5..7].parse().unwrap(),
        date[8..10].parse().unwrap(),
    );
    let (hh, mm, ss, ms): (i64, i64, i64, i64) = (
        time[..2].parse().unwrap(),
        time[3..5].parse().unwrap(),
        time[6..8].parse().unwrap(),
        time[9..12].parse().unwrap(),
    );
    // Days from civil (Howard Hinnant).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let millis = ((days * 86_400 + hh * 3600 + mm * 60 + ss) * 1000 + ms) as u64;
    UNIX_EPOCH + Duration::from_millis(millis)
}

/// Signs a body like the server does.
pub fn sign(body: &str, secret: &str, timestamp: &str) -> (String, String) {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(body.as_bytes());
    (
        format!("sha256={}", hex::encode(mac.finalize().into_bytes())),
        timestamp.to_owned(),
    )
}

pub fn verify_ok(body: &str, sig: &str, ts: &str, secret: &str, now: &str) -> WebhookEvent {
    let headers = [("X-Webhook-Signature", sig), ("X-Webhook-Timestamp", ts)];
    let options = WebhookVerifyOptions::new().now(at(now));
    match webhooks::construct_event(body, &headers, secret, &options).unwrap() {
        WebhookPayload::Event(e) => e,
        other => panic!("expected an event, got {other:?}"),
    }
}

fn case(file: &'static str) -> impl FnOnce() {
    move || {
        let f = load(&format!("fixtures/webhook/{file}"));
        let body = base64::engine::general_purpose::STANDARD
            .decode(f["request"]["rawBodyBase64"].as_str().unwrap())
            .unwrap();
        assert_eq!(body, f["request"]["rawBody"].as_str().unwrap().as_bytes());
        let headers: BTreeMap<String, String> = f["request"]["headers"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
            .collect();
        let secret = f["secret"].as_str().unwrap();
        let options = WebhookVerifyOptions::new()
            .now(at(f["verify"]["now"].as_str().unwrap()))
            .tolerance(Duration::from_secs(
                f["verify"]["toleranceSeconds"].as_u64().unwrap(),
            ));
        let expect = &f["expect"];
        // Bytes and the UTF-8 string give the same outcome.
        let from_bytes = webhooks::construct_event(&body, &headers, secret, &options);
        let text = String::from_utf8(body.clone()).unwrap();
        let from_text = webhooks::construct_event(text.as_str(), &headers, secret, &options);
        for result in [from_bytes, from_text] {
            match expect["outcome"].as_str().unwrap() {
                "event" => {
                    let WebhookPayload::Event(e) = result.unwrap() else {
                        panic!("expected an event")
                    };
                    let want = &expect["event"];
                    assert_eq!(e.event_id, want["eventId"].as_str().unwrap());
                    assert_eq!(e.event.as_str(), want["event"].as_str().unwrap());
                    assert_eq!(e.occurred_at, want["occurredAt"].as_str().unwrap());
                    assert_eq!(e.subscription_id, want["subscriptionId"].as_str().unwrap());
                    assert_eq!(e.is_test(), want["isTest"].as_bool().unwrap());
                    assert_eq!(e.data.link, want["link"].as_str().unwrap());
                    assert_eq!(e.payload_version, 3);
                    assert!(e.occurred_at_time().is_some());
                    let want_changed = want.get("changedFields").map(|c| {
                        c.as_array()
                            .unwrap()
                            .iter()
                            .map(|v| v.as_str().unwrap().to_owned())
                            .collect::<Vec<_>>()
                    });
                    assert_eq!(e.changed_fields, want_changed);
                    assert_eq!(e.raw_json().as_bytes(), body.as_slice());
                    match f["id"].as_str().unwrap() {
                        "W-03" => assert!(!e.data.content_present(), "content absent"),
                        "W-04" => {
                            assert!(!e.data.content_present() && !e.data.sentiment_present());
                            assert!(!e.data.companies_present() && !e.data.events_present());
                            assert!(e.data.available_at.is_none());
                            assert_eq!(Some(&e.occurred_at), e.data.first_seen_at.as_ref());
                        }
                        _ => {}
                    }
                }
                "wrongPayloadVersion" => {
                    let WebhookPayload::WrongPayloadVersion(w) = result.unwrap() else {
                        panic!("expected WrongPayloadVersion")
                    };
                    assert_eq!(
                        w.detected_version as u64,
                        expect["detectedVersion"].as_u64().unwrap()
                    );
                    assert_eq!(
                        w.payload_version_present,
                        expect["payloadVersionPresent"].as_bool().unwrap()
                    );
                    assert!(w.payload_version.is_none());
                    assert_eq!(w.link.as_deref(), expect["link"].as_str());
                    let raw: Value = serde_json::from_str(&w.raw).unwrap();
                    let sent: Value = serde_json::from_slice(&body).unwrap();
                    assert_eq!(raw, sent);
                }
                "error" => match result {
                    Err(Error::WebhookVerification(v)) => {
                        assert_eq!(v.reason().as_str(), expect["reason"].as_str().unwrap());
                    }
                    other => panic!("expected a verification error, got {other:?}"),
                },
                other => panic!("unknown outcome {other}"),
            }
        }
        // The signature check alone agrees with the fixture.
        let sig = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("x-webhook-signature"))
            .map(|(_, v)| v.as_str());
        let ts = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("x-webhook-timestamp"))
            .map(|(_, v)| v.as_str());
        if let (Some(sig), Some(ts)) = (sig, ts) {
            let ok = webhooks::verify_signature(&body, sig, ts, secret, &options).is_ok();
            let in_tolerance = !matches!(
                expect["reason"].as_str(),
                Some("timestamp_outside_tolerance")
            );
            assert_eq!(
                ok,
                expect["signatureValid"].as_bool().unwrap() && in_tolerance
            );
        }
        if f["id"] == "W-06" {
            // A changed v2 body fails the signature first.
            let tampered = text.replacen("example", "exampel", 1);
            match webhooks::construct_event(tampered.as_str(), &headers, secret, &options) {
                Err(Error::WebhookVerification(v)) => {
                    assert_eq!(v.reason().as_str(), "signature_mismatch")
                }
                other => panic!("expected signature_mismatch, got {other:?}"),
            }
        }
    }
}

/// F-04, webhook part: an unknown event name is kept as a string.
pub fn f04_webhook(item: &Value) {
    let body = item["wire"].to_string();
    let (sig, ts) = sign(&body, "whsec_test", "2026-09-18T12:03:06.000Z");
    let e = verify_ok(&body, &sig, &ts, "whsec_test", "2026-09-18T12:03:06.000Z");
    assert_eq!(e.event.as_str(), "article.retracted");
    assert!(!e.event.is_known());
}

pub fn cases() -> Vec<crate::common::Case> {
    vec![
        ("W-01", Box::new(case("created-enriched.json"))),
        ("W-02", Box::new(case("updated-with-changed-fields.json"))),
        ("W-03", Box::new(case("updated-content-not-entitled.json"))),
        ("W-04", Box::new(case("created-raw.json"))),
        ("W-05", Box::new(case("test-webhook.json"))),
        ("W-06", Box::new(case("v2-body.json"))),
        ("W-07", Box::new(case("neg-tampered-body.json"))),
        ("W-08", Box::new(case("neg-stale-timestamp.json"))),
        ("W-09", Box::new(case("neg-future-timestamp.json"))),
        ("W-10", Box::new(case("neg-unparseable-timestamp.json"))),
        ("W-11", Box::new(case("neg-unix-seconds-timestamp.json"))),
        ("W-12", Box::new(case("neg-missing-timestamp.json"))),
        ("W-13", Box::new(case("neg-missing-signature.json"))),
        ("W-14", Box::new(case("neg-wrong-prefix.json"))),
        ("W-15", Box::new(case("neg-secret-hex-decoded.json"))),
        ("W-16", Box::new(case("lowercase-headers.json"))),
    ]
}
