//! A webhook endpoint with axum. The handler takes the raw body (`Bytes`), never a parsed
//! `Json<...>`: the signature is over the exact bytes.
//!
//! Run: `FINLIGHT_WEBHOOK_SECRET=... cargo run --example webhook_endpoint`, then point your
//! webhook at `http://<host>:3000/webhook`.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use finlight::webhooks::{self, WebhookPayload, WebhookVerifyOptions};
use finlight::{Error, WebhookEvent};

#[derive(Clone)]
pub struct AppState {
    secret: Arc<str>,
    // Use a shared store (Redis, a database table) in production.
    seen: Arc<Mutex<HashSet<String>>>,
    on_event: Arc<dyn Fn(WebhookEvent) + Send + Sync>,
}

pub fn app(secret: &str, on_event: impl Fn(WebhookEvent) + Send + Sync + 'static) -> Router {
    let state = AppState {
        secret: Arc::from(secret),
        seen: Arc::new(Mutex::new(HashSet::new())),
        on_event: Arc::new(on_event),
    };
    Router::new()
        .route("/webhook", post(handle))
        .with_state(state)
}

async fn handle(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> StatusCode {
    match webhooks::construct_event(&body, &headers, &state.secret, &WebhookVerifyOptions::new()) {
        Ok(WebhookPayload::Event(event)) => {
            // The same event can arrive more than once: dedupe on event_id.
            if !state.seen.lock().unwrap().insert(event.event_id.clone()) {
                return StatusCode::NO_CONTENT;
            }
            // Answer within 5 s; do the slow work afterwards.
            let on_event = state.on_event.clone();
            tokio::spawn(async move { on_event(event) });
            StatusCode::NO_CONTENT
        }
        Ok(WebhookPayload::WrongPayloadVersion(old)) => {
            // Old-format retries after switching the webhook to v3: answer 2xx, keep the JSON.
            println!(
                "old payload version for {}",
                old.link.as_deref().unwrap_or("?")
            );
            StatusCode::NO_CONTENT
        }
        Ok(_) => StatusCode::NO_CONTENT,
        // Never answer with 410: it disables the webhook.
        Err(Error::WebhookVerification(_)) => StatusCode::BAD_REQUEST,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let secret =
        std::env::var("FINLIGHT_WEBHOOK_SECRET").map_err(|_| "set FINLIGHT_WEBHOOK_SECRET")?;
    let app = app(&secret, |event| {
        let test = if event.is_test() { " (test)" } else { "" };
        println!("{} {}{test}", event.event, event.data.title);
    });
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    println!("listening on :3000");
    axum::serve(listener, app).await?;
    Ok(())
}
