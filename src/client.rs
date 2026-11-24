use std::fmt;
use std::sync::Arc;

use crate::articles::Articles;
use crate::companies::{Companies, CompanyCache};
use crate::config::Config;
use crate::error::Result;
use crate::http::Transport;
use crate::params::{RawStreamParams, StreamParams};
use crate::sources::Sources;
use crate::stream::{FinlightStream, StreamOptions, start};

pub(crate) struct Inner {
    pub transport: Transport,
    pub companies: CompanyCache,
    pub config: Arc<Config>,
}

/// Client of the finlight API v3. Cheap to clone; clones share the HTTP client and caches.
/// Safe to use from many tasks, with many streams at the same time.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

impl Client {
    /// Builds a client. Fails with `Error::Validation` for an empty API key or a bad URL.
    pub fn new(config: Config) -> Result<Self> {
        config.validate()?;
        let config = Arc::new(config);
        Ok(Client {
            inner: Arc::new(Inner {
                transport: Transport::new(config.clone())?,
                companies: CompanyCache::new(config.company_cache_size),
                config,
            }),
        })
    }

    /// Article search and lookups.
    pub fn articles(&self) -> Articles<'_> {
        Articles { inner: &self.inner }
    }

    /// Company master data.
    pub fn companies(&self) -> Companies<'_> {
        Companies { inner: &self.inner }
    }

    /// Source catalog.
    pub fn sources(&self) -> Sources<'_> {
        Sources { inner: &self.inner }
    }

    /// Opens the enriched stream. The parameters are checked here; the connection opens in the
    /// background, and every outcome arrives through the returned stream. Needs a tokio runtime.
    pub async fn stream(
        &self,
        params: StreamParams,
        options: StreamOptions,
    ) -> Result<FinlightStream> {
        let frame = params.to_frame()?;
        Ok(start(self.inner.config.clone(), false, frame, options))
    }

    /// Opens the raw stream (article skeletons only, before enrichment).
    pub async fn raw_stream(
        &self,
        params: RawStreamParams,
        options: StreamOptions,
    ) -> Result<FinlightStream> {
        Ok(start(
            self.inner.config.clone(),
            true,
            params.to_frame(),
            options,
        ))
    }
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("config", &self.inner.config)
            .finish()
    }
}

impl fmt::Debug for Inner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Inner")
            .field("config", &self.config)
            .finish()
    }
}
