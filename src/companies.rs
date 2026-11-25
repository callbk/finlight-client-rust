use std::sync::Arc;

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

fn cache_info(resp: &Response) -> CacheInfo {
    CacheInfo {
        etag: resp.header("etag"),
        cache_control: resp.header("cache-control"),
    }
}

impl Companies<'_> {
    /// Resolves companies by ticker, name, alias, ISIN or vendor notation (at most 10 results,
    /// no pages). Costs 1 credit per request, errors included.
    pub async fn search(&self, params: &CompanySearchParams) -> Result<CompanyList> {
        let mut req = Request::get("/v3/companies");
        req.query = params.to_query()?;
        let resp = self.inner.transport.call(&req).await?;
        decode_companies(&resp.body)
    }

    /// Resolves 1 to 10 company ids. Unknown ids are left out; the order is not guaranteed.
    /// Costs 1 credit per request, however many ids it carries, errors included.
    pub async fn get_many(&self, ids: &[i64]) -> Result<CompanyList> {
        let mut req = Request::get("/v3/companies");
        req.query.push(("ids", company_ids_query(ids)?));
        let resp = self.inner.transport.call(&req).await?;
        decode_companies(&resp.body)
    }

    /// One company. A miss is `Error::Api` with `api_kind` `not_found`.
    ///
    /// The SDK keeps no cache of company records and sends no `If-None-Match` here. The
    /// record carries its `ETag` and `Cache-Control` in `cache_info()`: to save credits, keep
    /// the record yourself and do not call again within `max-age`, or revalidate with
    /// `get_if_none_match`. Costs 1 credit per request, like an article request; errors cost
    /// 1 credit too.
    pub async fn get(&self, company_id: i64) -> Result<CompanyRecord> {
        check_company_id(company_id, "company_id")?;
        let req = Request::get(format!("/v3/companies/{company_id}"));
        let resp = self.inner.transport.call(&req).await?;
        if resp.status == 304 {
            return Err(Error::decode(
                "the server answered 304 to a request without If-None-Match",
                None,
                None,
                None,
            ));
        }
        let mut record = decode_company_envelope(&resp.body)?;
        record.set_cache_info(cache_info(&resp));
        Ok(record)
    }

    /// One company, revalidated with an ETag you kept from an earlier record
    /// (`cache_info().etag`), sent unchanged as `If-None-Match`. `Ok(None)` means
    /// "not modified" (304), which is never an error: keep using your own copy. Costs 1 credit
    /// per request; a 304 and errors cost 1 credit too, so revalidating saves the download,
    /// not the credit.
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
        record.set_cache_info(cache_info(&resp));
        Ok(Some(record))
    }
}
