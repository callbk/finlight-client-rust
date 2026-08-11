use std::sync::Arc;

use serde::Deserialize;

use crate::error::Error;
use crate::http::ApiClient;
use crate::models::{Article, ArticleResponse, Source};
use crate::params::{GetArticleByLinkParams, GetArticlesParams};

/// Fetches financial news articles.
#[derive(Clone)]
pub struct ArticleService {
    pub(crate) api: Arc<ApiClient>,
}

impl ArticleService {
    /// Searches articles matching `params` and returns one result page.
    pub async fn fetch_articles(
        &self,
        params: &GetArticlesParams,
    ) -> Result<ArticleResponse, Error> {
        self.api.post("/v2/articles", params).await
    }

    /// Fetches a single article by its URL.
    pub async fn fetch_article_by_link(
        &self,
        params: &GetArticleByLinkParams,
    ) -> Result<Article, Error> {
        #[derive(Deserialize)]
        struct Envelope {
            article: Article,
        }

        let mut query = vec![("link", params.link.clone())];
        if params.include_content {
            query.push(("includeContent", "true".to_owned()));
        }
        if params.include_entities {
            query.push(("includeEntities", "true".to_owned()));
        }
        let envelope: Envelope = self.api.get("/v2/articles/by-link", &query).await?;
        Ok(envelope.article)
    }
}

/// Lists the news sources available through the API.
#[derive(Clone)]
pub struct SourceService {
    pub(crate) api: Arc<ApiClient>,
}

impl SourceService {
    /// Returns all sources with their availability flags.
    pub async fn get_sources(&self) -> Result<Vec<Source>, Error> {
        self.api.get("/v2/sources", &[]).await
    }
}
