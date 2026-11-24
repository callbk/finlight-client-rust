use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::client::Inner;
use crate::error::{Error, Result};
use crate::http::{Request, Response};
use crate::models::{
    CacheInfo, CompanyList, CompanyRecord, decode_companies, decode_company_envelope,
};
use crate::params::{CompanySearchParams, check_company_id, company_ids_query};

/// Company endpoints: `client.companies()`. They need the `entityAccess` entitlement
/// (without it: `Error::Api` with `api_kind` `forbidden`).
#[derive(Debug, Clone, Copy)]
pub struct Companies<'a> {
    pub(crate) inner: &'a Arc<Inner>,
}

/// Small LRU of records with an ETag, for revalidation in `get`.
pub(crate) struct CompanyCache {
    cap: usize,
    entries: Mutex<VecDeque<CompanyRecord>>,
}

impl CompanyCache {
    pub fn new(cap: usize) -> Self {
        CompanyCache {
            cap,
            entries: Mutex::new(VecDeque::new()),
        }
    }

    fn get(&self, id: i64) -> Option<CompanyRecord> {
        let entries = self.entries.lock().ok()?;
        entries.iter().find(|r| r.company_id == id).cloned()
    }

    fn put(&self, record: &CompanyRecord) {
        if self.cap == 0 {
            return;
        }
        if let Ok(mut entries) = self.entries.lock() {
            entries.retain(|r| r.company_id != record.company_id);
            entries.push_back(record.clone());
            while entries.len() > self.cap {
                entries.pop_front();
            }
        }
    }
}

fn cache_info(resp: &Response, not_modified: bool) -> CacheInfo {
    CacheInfo {
        etag: resp.header("etag"),
        cache_control: resp.header("cache-control"),
        not_modified,
    }
}

impl Companies<'_> {
    /// Resolves companies by ticker, name, alias, ISIN or vendor notation (at most 10 results,
    /// no pages).
    pub async fn search(&self, params: &CompanySearchParams) -> Result<CompanyList> {
        let mut req = Request::get("/v3/companies");
        req.query = params.to_query()?;
        let resp = self.inner.transport.call(&req).await?;
        decode_companies(&resp.body)
    }

    /// Resolves 1 to 10 company ids. Unknown ids are left out; the order is not guaranteed.
    pub async fn get_many(&self, ids: &[i64]) -> Result<CompanyList> {
        let mut req = Request::get("/v3/companies");
        req.query.push(("ids", company_ids_query(ids)?));
        let resp = self.inner.transport.call(&req).await?;
        decode_companies(&resp.body)
    }

    /// One company. A miss is `Error::Api` with `api_kind` `not_found`.
    ///
    /// The SDK keeps recent records with their ETag (`Config::company_cache_size`) and
    /// revalidates them; on 304 it returns the stored record with `cache_info().not_modified`.
    pub async fn get(&self, company_id: i64) -> Result<CompanyRecord> {
        check_company_id(company_id, "company_id")?;
        let cached = self.inner.companies.get(company_id);
        let mut req = Request::get(format!("/v3/companies/{company_id}"));
        req.if_none_match = cached
            .as_ref()
            .and_then(|r| r.cache_info())
            .and_then(|c| c.etag.clone());
        let resp = self.inner.transport.call(&req).await?;
        if resp.status == 304 {
            let Some(mut record) = cached else {
                return Err(Error::decode(
                    "the server answered 304 without a stored record",
                    None,
                    None,
                    None,
                ));
            };
            let mut info = cache_info(&resp, true);
            if info.etag.is_none() {
                info.etag = record.cache_info().and_then(|c| c.etag.clone());
            }
            record.set_cache_info(info);
            return Ok(record);
        }
        let mut record = decode_company_envelope(&resp.body)?;
        record.set_cache_info(cache_info(&resp, false));
        self.inner.companies.put(&record);
        Ok(record)
    }

    /// One company, revalidated with your own ETag (`If-None-Match`). `Ok(None)` means
    /// "not modified" (304), which is never an error.
    pub async fn get_if_none_match(
        &self,
        company_id: i64,
        etag: &str,
    ) -> Result<Option<CompanyRecord>> {
        check_company_id(company_id, "company_id")?;
        if etag.is_empty() {
            return Err(Error::validation(
                "etag must not be empty",
                Some("if_none_match"),
            ));
        }
        let mut req = Request::get(format!("/v3/companies/{company_id}"));
        req.if_none_match = Some(etag.to_owned());
        let resp = self.inner.transport.call(&req).await?;
        if resp.status == 304 {
            return Ok(None);
        }
        let mut record = decode_company_envelope(&resp.body)?;
        record.set_cache_info(cache_info(&resp, false));
        self.inner.companies.put(&record);
        Ok(Some(record))
    }
}
