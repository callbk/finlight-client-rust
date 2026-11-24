//! Shared test support: the contract kit, a local HTTP server, a local WebSocket server and
//! helpers for the paused tokio clock. Everything runs on 127.0.0.1; nothing leaves the machine.
#![allow(dead_code)]

pub mod http_mock;
pub mod ws_mock;

use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

/// A kit case: its id and the test.
pub type Case = (&'static str, Box<dyn FnOnce() + Send>);

/// The API key most kit fixtures use.
pub const KIT_API_KEY: &str = "sk_0000000000000000000000000000000000000000000000000000000000000001";

pub fn kit_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("conformance")
}

pub fn load(rel: &str) -> Value {
    let path = kit_dir().join(rel);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("bad JSON in {}: {e}", path.display()))
}

pub fn load_text(rel: &str) -> String {
    std::fs::read_to_string(kit_dir().join(rel)).unwrap()
}

/// Runs a future on a fresh current-thread runtime with the clock paused.
pub fn run_paused<F: Future<Output = ()>>(f: F) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap()
        .block_on(f)
}

/// Runs a future on a fresh current-thread runtime with the real clock.
pub fn run<F: Future<Output = ()>>(f: F) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}

/// Keeps the runtime busy so the paused clock never jumps on its own; time then moves only
/// through `advance`. I/O is still polled on every scheduler round.
pub fn spawn_spinner() -> tokio::task::JoinHandle<()> {
    tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    })
}

/// Lets every task run a few rounds (I/O on loopback included).
pub async fn settle() {
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

/// Waits (by yielding, never by sleeping) until `cond` holds; fails after 10 s of real time.
pub async fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
    let start = std::time::Instant::now();
    while !cond() {
        if start.elapsed() > Duration::from_secs(10) {
            panic!("timed out waiting for {what}");
        }
        tokio::task::yield_now().await;
    }
}

/// Advances the paused clock in steps of at most one second, letting tasks run after each step.
pub async fn advance(ms: u64) {
    let mut left = ms;
    while left > 0 {
        let step = left.min(1000);
        tokio::time::advance(Duration::from_millis(step)).await;
        settle().await;
        left -= step;
    }
    settle().await;
}

/// The test reqwest client: no timeouts and no idle timer, so the paused clock only moves for
/// the SDK's own retry waits.
pub fn test_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .pool_idle_timeout(None)
        .pool_max_idle_per_host(0)
        .retry(reqwest::retry::never())
        .build()
        .unwrap()
}

/// Query string pairs, decoded.
pub fn query_pairs(query: Option<&str>) -> Vec<(String, String)> {
    let Some(q) = query else { return Vec::new() };
    if q.is_empty() {
        return Vec::new();
    }
    q.split('&')
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
            out.push(u8::from_str_radix(hex, 16).unwrap());
            i += 3;
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

/// Removes keys from a JSON object.
pub fn without(value: &Value, keys: &[&str]) -> Value {
    let mut v = value.clone();
    if let Some(o) = v.as_object_mut() {
        for k in keys {
            o.remove(*k);
        }
    }
    v
}
