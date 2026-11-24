//! R-LOG-1: building a client and opening a stream installs no global subscriber or handler.
//! Its own test binary, so no other test can have set anything before.

use finlight::{Client, Config, StreamOptions, StreamParams};
use tracing::level_filters::LevelFilter;

#[test]
fn client_installs_nothing_global() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let client = Client::new(
            Config::builder("sk_test")
                .wss_url("ws://127.0.0.1:9")
                .build()
                .unwrap(),
        )
        .unwrap();
        let mut stream = client
            .stream(StreamParams::new(), StreamOptions::new())
            .await
            .unwrap();
        stream.close().await;
    });
    assert!(
        !tracing::dispatcher::has_been_set(),
        "no tracing subscriber was installed"
    );
    assert_eq!(
        LevelFilter::current(),
        LevelFilter::OFF,
        "logging stays off without a subscriber"
    );
}
