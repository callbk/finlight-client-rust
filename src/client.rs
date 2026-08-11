use std::sync::Arc;

use crate::config::Config;
use crate::error::Error;
use crate::http::ApiClient;
use crate::services::{ArticleService, SourceService};
use crate::websocket::{RawWebSocketClient, WebSocketClient, WebSocketOptions};

/// Entry point to the finlight API.
pub struct Client {
    /// REST access to articles.
    pub articles: ArticleService,
    /// REST access to sources.
    pub sources: SourceService,
    /// Enhanced article stream, default options. For custom options use
    /// [`WebSocketClient::new`].
    pub websocket: WebSocketClient,
    /// Raw article stream, default options. For custom options use
    /// [`RawWebSocketClient::new`].
    pub raw_websocket: RawWebSocketClient,
}

impl Client {
    /// Validates `config` and returns a ready-to-use client.
    pub fn new(config: Config) -> Result<Self, Error> {
        if config.api_key.is_empty() {
            return Err(Error::MissingApiKey);
        }
        let api = Arc::new(ApiClient::new(config.clone())?);
        Ok(Self {
            articles: ArticleService { api: api.clone() },
            sources: SourceService { api },
            websocket: WebSocketClient::new(config.clone(), WebSocketOptions::default()),
            raw_websocket: RawWebSocketClient::new(config, WebSocketOptions::default()),
        })
    }
}
