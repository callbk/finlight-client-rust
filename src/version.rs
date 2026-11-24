/// Version of this SDK, without a leading `v`. Single source: `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The payload version this SDK speaks. Every WebSocket subscribe carries it as the JSON number 3.
pub const PAYLOAD_VERSION: u8 = 3;

/// `User-Agent` sent on every REST request and WebSocket dial.
pub(crate) fn user_agent() -> String {
    format!("finlight-client-rust/{VERSION}")
}
