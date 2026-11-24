//! R-TEST-1: the five standard examples are built and run against the local servers.

#![allow(dead_code)]

mod common;

#[path = "../examples/lookup.rs"]
mod lookup;
#[path = "../examples/search.rs"]
mod search;
#[path = "../examples/search_all.rs"]
mod search_all;
#[path = "../examples/stream.rs"]
mod stream;
#[path = "../examples/webhook_endpoint.rs"]
mod webhook_endpoint;

use std::sync::{Arc, Mutex};

use finlight::{Client, Config};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

use common::http_mock::{HttpMock, Scripted};
use common::ws_mock::WsMock;
use common::{KIT_API_KEY, load, run_paused, spawn_spinner, test_http_client};

fn rest_client(base_url: &str) -> Client {
    Client::new(
        Config::builder(KIT_API_KEY)
            .base_url(base_url)
            .http_client(test_http_client())
            .build()
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn example_search() {
    run_paused(async {
        let mock = HttpMock::start().await;
        let f = load("fixtures/requests/search-all-parameters.json");
        mock.enqueue(Scripted::from_fixture(&f["response"]));
        search::run(&rest_client(&mock.base_url)).await.unwrap();
        let body = mock.requests()[0].body_json();
        assert_eq!(
            body,
            json!({"query": "ticker:NVDA", "languages": ["en"], "include": ["companies", "events"], "limit": 10})
        );
        // An API error is printed with status, code and request id, not raised.
        mock.enqueue(Scripted::from_fixture(
            &load("fixtures/errors/unauthorized-invalid-key.json")["response"],
        ));
        search::run(&rest_client(&mock.base_url)).await.unwrap();
    });
}

#[test]
fn example_search_all() {
    run_paused(async {
        let mock = HttpMock::start().await;
        for step in load("fixtures/pagination/three-pages.json")["steps"]
            .as_array()
            .unwrap()
        {
            mock.enqueue(Scripted::from_fixture(&step["response"]));
        }
        let count = search_all::run(&rest_client(&mock.base_url)).await.unwrap();
        assert_eq!(count, 5);
        assert_eq!(mock.count(), 3);
    });
}

#[test]
fn example_lookup() {
    run_paused(async {
        let mock = HttpMock::start().await;
        let by_link = load("fixtures/requests/get-by-id.json")["response"].clone();
        let article = by_link["body"]["article"].clone();
        mock.enqueue(Scripted::json(
            200,
            &json!({"articles": [article],
            "pagination": {"limit": 1, "returned": 1, "hasMore": true, "nextCursor": "c"}}),
        ));
        mock.enqueue(Scripted::from_fixture(&by_link));
        mock.enqueue(Scripted::from_fixture(&by_link));
        mock.enqueue(Scripted::from_fixture(
            &load("fixtures/errors/not-found-id.json")["response"],
        ));
        mock.enqueue(Scripted::from_fixture(
            &load("fixtures/requests/companies-search.json")["response"],
        ));
        mock.enqueue(Scripted::from_fixture(
            &load("fixtures/requests/companies-get.json")["response"],
        ));
        mock.enqueue(Scripted::from_fixture(
            &load("fixtures/requests/sources-list-full.json")["response"],
        ));
        lookup::run(&rest_client(&mock.base_url)).await.unwrap();
        let paths: Vec<String> = mock.requests().iter().map(|r| r.path.clone()).collect();
        assert_eq!(
            paths,
            [
                "/v3/articles/search",
                "/v3/articles/by-link",
                "/v3/articles/EU8JwpsZ8iJehuyPZjMqRz",
                "/v3/articles/AAAAAAAAAAAAAAAAAAAAAA",
                "/v3/companies",
                "/v3/companies/900001",
                "/v3/sources",
            ]
        );
    });
}

#[test]
fn example_stream() {
    run_paused(async {
        let ws = WsMock::start().await;
        let spinner = spawn_spinner();
        let client = Client::new(
            Config::builder(KIT_API_KEY)
                .wss_url(ws.url.clone())
                .build()
                .unwrap(),
        )
        .unwrap();
        let task = tokio::spawn(async move { stream::run(&client, 5).await });
        let conn = ws.wait_dial(0).await.conn.unwrap();
        let subscribe = conn.next("subscribe").await;
        assert_eq!(subscribe["include"], json!(["companies"]));
        conn.send(&json!({"event": "connection.admitted", "payloadVersion": 3,
            "data": {"leaseId": "l", "serverNow": 1, "clientNonce": null}}));
        conn.send(&json!({"event": "connection.draining", "payloadVersion": 3, "data": {}}));
        let article = load("fixtures/decode/full-article.json")["wire"].clone();
        for i in 0..6 {
            let mut a = article.clone();
            a["id"] = Value::String(format!("id-{i}"));
            conn.send(&json!({"event": "article.created", "payloadVersion": 3, "data": a}));
        }
        common::wait_for("example finished", || task.is_finished()).await;
        assert_eq!(task.await.unwrap().unwrap(), 5);
        common::wait_for("socket closed", || conn.closed_code().is_some()).await;
        assert_eq!(conn.closed_code(), Some(Some(1000)));
        spinner.abort();
    });
}

fn sign(body: &[u8], secret: &str, ts: &str) -> String {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(ts.as_bytes());
    mac.update(b".");
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

#[test]
fn example_webhook_endpoint() {
    common::run(async {
        let received = Arc::new(Mutex::new(Vec::new()));
        let r = received.clone();
        let app =
            webhook_endpoint::app("whsec_example", move |e| r.lock().unwrap().push(e.event_id));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/webhook", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let http = reqwest::Client::new();
        // Timestamps must be close to now for the default tolerance.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let ts = iso(now);
        let event_body = {
            let mut f =
                load("fixtures/forward-compat/envelope-keys.json")["envelopes"][2]["wire"].clone();
            f["eventId"] = json!("evt_example_1");
            f.to_string()
        };
        let send = |body: String, sig: String| {
            let http = http.clone();
            let url = url.clone();
            let ts = ts.clone();
            async move {
                http.post(&url)
                    .header("content-type", "application/json")
                    .header("x-webhook-signature", sig)
                    .header("x-webhook-timestamp", ts)
                    .body(body)
                    .send()
                    .await
                    .unwrap()
                    .status()
                    .as_u16()
            }
        };
        let sig = sign(event_body.as_bytes(), "whsec_example", &ts);
        assert_eq!(send(event_body.clone(), sig.clone()).await, 204);
        assert_eq!(
            send(event_body.clone(), sig).await,
            204,
            "a duplicate is acknowledged"
        );
        let v2 = load("fixtures/webhook/v2-body.json")["request"]["rawBody"]
            .as_str()
            .unwrap()
            .to_owned();
        let sig = sign(v2.as_bytes(), "whsec_example", &ts);
        assert_eq!(
            send(v2, sig).await,
            204,
            "an old payload version is acknowledged"
        );
        assert_eq!(
            send(event_body, "sha256=00".to_owned()).await,
            400,
            "a bad signature is 400, never 410"
        );
        common::wait_for("event handled", || !received.lock().unwrap().is_empty()).await;
        assert_eq!(received.lock().unwrap().as_slice(), ["evt_example_1"]);
    });
}

/// Seconds since the epoch as ISO 8601 UTC with milliseconds.
fn iso(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}
