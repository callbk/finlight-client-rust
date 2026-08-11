/// Client identifier sent to the server in the `x-client-version` and
/// `User-Agent` headers.
pub(crate) const CLIENT_VERSION: &str =
    concat!("rust/finlight-client-rust@", env!("CARGO_PKG_VERSION"));
