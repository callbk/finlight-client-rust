use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures_core::Stream;
use serde_json::{Map, Value};

use crate::client::Inner;
use crate::error::{Error, Result};
use crate::http::Request;
use crate::models::{Article, ArticlePage, decode_article_envelope, decode_search_page};
use crate::params::{GetArticleOptions, SearchAllOptions, SearchParams, encode_component};

/// Article endpoints: `client.articles()`.
#[derive(Debug, Clone, Copy)]
pub struct Articles<'a> {
    pub(crate) inner: &'a Arc<Inner>,
}

impl Articles<'_> {
    /// One page of a search (`POST /v3/articles/search`). Only the keys you set are sent.
    pub async fn search(&self, params: &SearchParams) -> Result<ArticlePage> {
        let body = Value::Object(params.to_body()?).to_string();
        fetch_page(self.inner.clone(), body).await
    }

    /// Every article of a search, page by page, up to `max_pages` (default `Config::max_pages`).
    ///
    /// The body is frozen at this call; later pages only add the `cursor`. Each page is one
    /// call and costs quota. Errors arrive during iteration; a monthly 429 ends it at once.
    /// The parameters are checked here, before any request.
    pub fn search_all(
        &self,
        params: SearchParams,
        options: SearchAllOptions,
    ) -> Result<SearchAllResult> {
        let mut body = params.to_body()?;
        let cursor = match body.remove("cursor") {
            Some(Value::String(c)) => Some(c),
            _ => None,
        };
        let max_pages = options
            .max_pages
            .unwrap_or(self.inner.transport.config().max_pages);
        if max_pages == 0 {
            return Err(Error::validation(
                "max_pages must be at least 1",
                Some("max_pages"),
            ));
        }
        Ok(SearchAllResult {
            inner: self.inner.clone(),
            frozen: body,
            cursor,
            max_pages,
            pages_fetched: 0,
            truncated: false,
            done: false,
            buffer: VecDeque::new(),
            mode: None,
            pending: None,
        })
    }

    /// One article by id (`GET /v3/articles/{id}`). A miss is `Error::Api` with `api_kind`
    /// `not_found`; articles indexed before the id rollout are reachable only by link.
    pub async fn get(&self, id: &str, options: &GetArticleOptions) -> Result<Article> {
        if id.is_empty() {
            return Err(Error::validation("id must not be empty", Some("id")));
        }
        let mut req = Request::get(format!("/v3/articles/{}", encode_component(id)));
        if let Some(include) = options.include_query()? {
            req.query.push(("include", include));
        }
        let resp = self.inner.transport.call(&req).await?;
        decode_article_envelope(&resp.body)
    }

    /// One article by link (`GET /v3/articles/by-link`). The link must match exactly.
    pub async fn get_by_link(&self, link: &str, options: &GetArticleOptions) -> Result<Article> {
        if link.is_empty() {
            return Err(Error::validation("link must not be empty", Some("link")));
        }
        let mut req = Request::get("/v3/articles/by-link");
        req.query.push(("link", link.to_owned()));
        if let Some(include) = options.include_query()? {
            req.query.push(("include", include));
        }
        let resp = self.inner.transport.call(&req).await?;
        decode_article_envelope(&resp.body)
    }
}

async fn fetch_page(inner: Arc<Inner>, body: String) -> Result<ArticlePage> {
    let resp = inner
        .transport
        .call(&Request::post("/v3/articles/search", body))
        .await?;
    decode_search_page(&resp.body)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Articles,
    Pages,
}

type PageFuture = Pin<Box<dyn Future<Output = Result<ArticlePage>> + Send>>;

/// Result of `articles().search_all()`: iterate articles with [`SearchAllResult::next`] (or as a
/// `Stream`), or pages with [`SearchAllResult::next_page`]. The first call decides; the other
/// way then returns `Error::Validation`. It can be iterated once.
pub struct SearchAllResult {
    inner: Arc<Inner>,
    frozen: Map<String, Value>,
    cursor: Option<String>,
    max_pages: u32,
    pages_fetched: u32,
    truncated: bool,
    done: bool,
    buffer: VecDeque<Article>,
    mode: Option<Mode>,
    pending: Option<PageFuture>,
}

impl SearchAllResult {
    /// The next article; `None` at the end.
    pub async fn next(&mut self) -> Option<Result<Article>> {
        std::future::poll_fn(|cx| self.poll_article(cx)).await
    }

    /// The next page; `None` at the end.
    pub async fn next_page(&mut self) -> Option<Result<ArticlePage>> {
        if let Some(err) = self.lock(Mode::Pages) {
            return Some(Err(err));
        }
        std::future::poll_fn(|cx| self.poll_page(cx)).await
    }

    /// Cursor to continue after the last fetched page (set when `max_pages` stopped the walk).
    pub fn next_cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    /// `true` when `max_pages` stopped the walk before the end.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// Pages fetched so far.
    pub fn pages_fetched(&self) -> u32 {
        self.pages_fetched
    }

    fn lock(&mut self, mode: Mode) -> Option<Error> {
        match self.mode {
            None => {
                self.mode = Some(mode);
                None
            }
            Some(m) if m == mode => None,
            Some(_) => Some(Error::validation(
                "a search_all result is iterated once, either by article or by page; call search_all again",
                None,
            )),
        }
    }

    fn poll_page(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<ArticlePage>>> {
        if self.done {
            return Poll::Ready(None);
        }
        if self.pending.is_none() {
            let mut body = self.frozen.clone();
            if let Some(cursor) = &self.cursor {
                body.insert("cursor".into(), Value::String(cursor.clone()));
            }
            let body = Value::Object(body).to_string();
            self.pending = Some(Box::pin(fetch_page(self.inner.clone(), body)));
        }
        let Some(fut) = self.pending.as_mut() else {
            return Poll::Ready(None);
        };
        let result = match fut.as_mut().poll(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(r) => r,
        };
        self.pending = None;
        match result {
            Ok(page) => {
                self.pages_fetched += 1;
                let next = page.pagination.next_cursor.clone();
                if !page.pagination.has_more || next.is_none() {
                    self.done = true;
                    self.cursor = None;
                } else {
                    self.cursor = next;
                    if self.pages_fetched >= self.max_pages {
                        self.done = true;
                        self.truncated = true;
                        tracing::warn!(
                            target: "finlight",
                            max_pages = self.max_pages,
                            "search_all stopped at max_pages; next_cursor() continues the search"
                        );
                    }
                }
                Poll::Ready(Some(Ok(page)))
            }
            Err(err) => {
                self.done = true;
                Poll::Ready(Some(Err(err)))
            }
        }
    }

    fn poll_article(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<Article>>> {
        if let Some(err) = self.lock(Mode::Articles) {
            return Poll::Ready(Some(Err(err)));
        }
        loop {
            if let Some(article) = self.buffer.pop_front() {
                return Poll::Ready(Some(Ok(article)));
            }
            match self.poll_page(cx) {
                Poll::Ready(Some(Ok(page))) => self.buffer.extend(page.articles),
                Poll::Ready(Some(Err(err))) => return Poll::Ready(Some(Err(err))),
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl Stream for SearchAllResult {
    type Item = Result<Article>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().poll_article(cx)
    }
}

impl std::fmt::Debug for SearchAllResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchAllResult")
            .field("pages_fetched", &self.pages_fetched)
            .field("max_pages", &self.max_pages)
            .field("truncated", &self.truncated)
            .field("done", &self.done)
            .finish()
    }
}
