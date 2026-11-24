use std::sync::Arc;

use crate::client::Inner;
use crate::error::Result;
use crate::http::Request;
use crate::models::{SourceList, decode_sources};

/// Source catalog: `client.sources()`.
#[derive(Debug, Clone, Copy)]
pub struct Sources<'a> {
    pub(crate) inner: &'a Arc<Inner>,
}

impl Sources<'_> {
    /// The full source catalog, sorted by domain. `has_content` is `None` without
    /// `fullArticleAccess` (unknown, not "no content"). Costs 1 credit per successful
    /// request; errors are free.
    pub async fn list(&self) -> Result<SourceList> {
        let resp = self
            .inner
            .transport
            .call(&Request::get("/v3/sources"))
            .await?;
        decode_sources(&resp.body)
    }
}
