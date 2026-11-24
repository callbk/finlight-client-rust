//! R-01 .. R-12, P-01 .. P-06, E-01 .. E-25 and the REST part of F-04, against the local
//! HTTP server. Retry waits run on the paused clock (no real waiting).

use std::collections::HashSet;
use std::time::Duration;

use finlight::{
    ApiError, Client, CompanySearchParams, Config, DateInput, Error, GetArticleOptions,
    SearchAllOptions, SearchParams, SourceType,
};
use futures_util::StreamExt;
use serde_json::{Value, json};

use crate::common::http_mock::{HttpMock, Recorded, Scripted};
use crate::common::{KIT_API_KEY, load, query_pairs, run_paused, test_http_client, without};
use crate::webhooks::at;

fn client_with_key(base_url: &str, key: &str) -> Client {
    let config = Config::builder(key)
        .base_url(base_url)
        .http_client(test_http_client())
        .build()
        .unwrap();
    Client::new(config).unwrap()
}

fn client(base_url: &str) -> Client {
    client_with_key(base_url, KIT_API_KEY)
}

/// Method, path, decoded query, JSON body and API key header equal the fixture.
fn expect_request(got: &Recorded, want: &Value) {
    assert_eq!(got.method, want["method"].as_str().unwrap());
    assert_eq!(got.path, want["path"].as_str().unwrap());
    assert_eq!(
        query_pairs(got.query.as_deref()),
        query_pairs(want["query"].as_str())
    );
    if let Some(q) = &got.query {
        assert!(!q.contains("%5B%5D=") && !q.contains("[]="), "never key[]=");
    }
    if want.get("body").is_some() {
        assert_eq!(got.body_json(), want["body"]);
        assert!(
            got.header("content-type")
                .unwrap()
                .contains("application/json")
        );
    } else {
        assert!(got.body.is_empty());
    }
    if let Some(key) = want["headers"]["X-API-KEY"].as_str() {
        assert_eq!(got.header("x-api-key"), Some(key));
    }
    assert_eq!(
        got.header("user-agent").map(str::to_owned),
        Some(format!("finlight-client-rust/{}", finlight::VERSION))
    );
    assert_eq!(got.header("x-client-version"), Some(finlight::VERSION));
    assert!(got.header("x-request-id").is_some());
}

fn ids(v: &Value) -> Vec<String> {
    v["articles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_str().unwrap().to_owned())
        .collect()
}

fn fixture(rel: &str) -> Value {
    load(rel)
}

fn r01() {
    let f = fixture("fixtures/requests/search-minimal.json");
    run_paused(async move {
        let mock = HttpMock::start().await;
        mock.enqueue(Scripted::from_fixture(&f["response"]));
        let params = SearchParams::from_json(&f["sdk"]["args"].to_string()).unwrap();
        let page = client(&mock.base_url)
            .articles()
            .search(&params)
            .await
            .unwrap();
        let req = &mock.requests()[0];
        expect_request(req, &f["request"]);
        assert_eq!(req.body_text(), r#"{"query":"ticker:XMPL"}"#);
        assert_eq!(
            page.articles
                .iter()
                .map(|a| a.id.clone())
                .collect::<Vec<_>>(),
            ids(&f["response"]["body"])
        );
        assert_eq!(
            serde_json::to_value(&page.pagination).unwrap(),
            f["response"]["body"]["pagination"]
        );
    })
}

fn r02() {
    let f = fixture("fixtures/requests/search-all-parameters.json");
    run_paused(async move {
        let mock = HttpMock::start().await;
        mock.enqueue(Scripted::from_fixture(&f["response"]));
        let mut params = SearchParams::from_json(&f["sdk"]["args"].to_string()).unwrap();
        // A native time goes out as UTC ISO 8601 with milliseconds; a string is sent unchanged.
        params.from = Some(DateInput::Time(at(f["sdk"]["args"]["from"]
            .as_str()
            .unwrap())));
        let page = client(&mock.base_url)
            .articles()
            .search(&params)
            .await
            .unwrap();
        let req = &mock.requests()[0];
        expect_request(req, &f["request"]);
        let body = req.body_json();
        assert!(body["limit"].is_u64());
        assert_eq!(body["from"], "2026-11-01T00:00:00.000Z");
        assert_eq!(
            params.source_types,
            Some(vec![SourceType::Newswire, SourceType::FinancialMedia])
        );
        assert_eq!(
            params.exclude_source_types,
            Some(vec![SourceType::PressRelease])
        );
        assert_eq!(page.articles.len(), 5);
    })
}

fn r03() {
    let f = fixture("fixtures/requests/search-no-parameters.json");
    run_paused(async move {
        let mock = HttpMock::start().await;
        mock.enqueue(Scripted::from_fixture(&f["response"]));
        mock.enqueue(Scripted::from_fixture(&f["response"]));
        let client = client(&mock.base_url);
        let params = SearchParams::from_json(&f["sdk"]["args"].to_string()).unwrap();
        client.articles().search(&params).await.unwrap();
        client
            .articles()
            .search(&SearchParams::new())
            .await
            .unwrap();
        let reqs = mock.requests();
        expect_request(&reqs[0], &f["request"]);
        assert_eq!(reqs[0].body_text(), "{}");
        assert_eq!(reqs[1].body_text(), "{}");
    })
}

fn r04() {
    let f = fixture("fixtures/requests/get-by-id.json");
    run_paused(async move {
        let mock = HttpMock::start().await;
        mock.enqueue(Scripted::from_fixture(&f["response"]));
        let args = &f["sdk"]["args"];
        let options = GetArticleOptions::new().include(
            args["include"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_owned()),
        );
        let a = client(&mock.base_url)
            .articles()
            .get(args["id"].as_str().unwrap(), &options)
            .await
            .unwrap();
        let req = &mock.requests()[0];
        expect_request(req, &f["request"]);
        let pairs = query_pairs(req.query.as_deref());
        assert_eq!(
            pairs,
            vec![("include".to_owned(), "content,events".to_owned())]
        );
        assert_eq!(a.id, args["id"].as_str().unwrap());
        assert!(a.content_present());
    })
}

fn r05() {
    let f = fixture("fixtures/requests/get-by-link.json");
    run_paused(async move {
        let mock = HttpMock::start().await;
        mock.enqueue(Scripted::from_fixture(&f["response"]));
        let link = f["sdk"]["args"]["link"].as_str().unwrap().to_owned();
        let a = client(&mock.base_url)
            .articles()
            .get_by_link(&link, &GetArticleOptions::new().include(["companies"]))
            .await
            .unwrap();
        let req = &mock.requests()[0];
        expect_request(req, &f["request"]);
        let pairs = query_pairs(req.query.as_deref());
        assert_eq!(pairs.iter().filter(|(k, _)| k == "link").count(), 1);
        assert_eq!(pairs[0], ("link".to_owned(), link.clone()));
        assert_eq!(a.link, link);
    })
}

fn r06() {
    let f = fixture("fixtures/requests/companies-search.json");
    run_paused(async move {
        let mock = HttpMock::start().await;
        mock.enqueue(Scripted::from_fixture(&f["response"]));
        let args = &f["sdk"]["args"];
        let list = client(&mock.base_url)
            .companies()
            .search(
                &CompanySearchParams::new(args["q"].as_str().unwrap())
                    .mic(args["mic"].as_str().unwrap()),
            )
            .await
            .unwrap();
        expect_request(&mock.requests()[0], &f["request"]);
        assert_eq!(
            list.iter().map(|c| c.company_id).collect::<Vec<_>>(),
            vec![900001]
        );
        assert!(list.skipped().is_empty());
    })
}

fn r07() {
    let f = fixture("fixtures/requests/companies-get-many.json");
    run_paused(async move {
        let mock = HttpMock::start().await;
        mock.enqueue(Scripted::from_fixture(&f["response"]));
        let ids: Vec<i64> = f["sdk"]["args"]["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap())
            .collect();
        let list = client(&mock.base_url)
            .companies()
            .get_many(&ids)
            .await
            .unwrap();
        let req = &mock.requests()[0];
        expect_request(req, &f["request"]);
        assert_eq!(
            query_pairs(req.query.as_deref()),
            vec![("ids".to_owned(), "900001,900002".to_owned())]
        );
        let private = list.iter().find(|c| c.company_id == 900002).unwrap();
        assert!(private.primary_listing.is_none());
        assert!(private.listings.is_empty());
        let mut got: Vec<i64> = list.iter().map(|c| c.company_id).collect();
        got.sort();
        assert_eq!(got, vec![900001, 900002]);
    })
}

fn r08() {
    let f = fixture("fixtures/requests/companies-get.json");
    run_paused(async move {
        let mock = HttpMock::start().await;
        mock.enqueue(Scripted::from_fixture(&f["response"]));
        let record = client(&mock.base_url)
            .companies()
            .get(900001)
            .await
            .unwrap();
        expect_request(&mock.requests()[0], &f["request"]);
        assert_eq!(record.company_id, 900001);
        assert_eq!(record.listings.len(), 2);
        let mics: HashSet<_> = record.listings.iter().map(|l| l.mic.clone()).collect();
        assert_eq!(
            mics.len(),
            record.listings.len(),
            "one listing per exchange"
        );
        assert_eq!(
            record.listings[0].mic,
            record.primary_listing.as_ref().unwrap().mic
        );
        let info = record.cache_info().unwrap();
        assert_eq!(
            info.etag.as_deref(),
            f["response"]["headers"]["etag"].as_str()
        );
        assert_eq!(
            info.cache_control.as_deref(),
            f["response"]["headers"]["cache-control"].as_str()
        );
        assert!(!info.not_modified);
    })
}

fn r09() {
    let first = fixture("fixtures/requests/companies-get.json");
    let f = fixture("fixtures/requests/companies-get-not-modified.json");
    run_paused(async move {
        let mock = HttpMock::start().await;
        mock.enqueue(Scripted::from_fixture(&first["response"]));
        mock.enqueue(Scripted::from_fixture(&f["response"]));
        mock.enqueue(Scripted::from_fixture(&f["response"]));
        let client = client(&mock.base_url);
        let record = client.companies().get(900001).await.unwrap();
        // A caller-supplied ETag: 304 is "not modified", never an error and never a decode.
        let etag = f["sdk"]["args"]["ifNoneMatch"].as_str().unwrap();
        let again = client
            .companies()
            .get_if_none_match(900001, etag)
            .await
            .unwrap();
        assert!(again.is_none());
        let reqs = mock.requests();
        expect_request(&reqs[1], &f["request"]);
        assert_eq!(
            reqs[1].header("if-none-match"),
            f["request"]["headers"]["If-None-Match"].as_str()
        );
        assert!(
            !reqs[1]
                .header("cache-control")
                .unwrap_or_default()
                .contains("no-cache")
        );
        // The SDK revalidates its own copy and returns it on 304.
        let third = client.companies().get(900001).await.unwrap();
        assert_eq!(mock.requests()[2].header("if-none-match"), Some(etag));
        assert!(third.cache_info().unwrap().not_modified);
        assert_eq!(third.raw_json(), record.raw_json());
        assert_eq!(third.name, record.name);
    })
}

fn r10() {
    let f = fixture("fixtures/requests/sources-list-full.json");
    run_paused(async move {
        let mock = HttpMock::start().await;
        mock.enqueue(Scripted::from_fixture(&f["response"]));
        let sources = client(&mock.base_url).sources().list().await.unwrap();
        expect_request(&mock.requests()[0], &f["request"]);
        let want: Vec<Option<bool>> = f["response"]["body"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["hasContent"].as_bool())
            .collect();
        assert!(want.iter().all(Option::is_some));
        assert_eq!(
            sources.iter().map(|s| s.has_content).collect::<Vec<_>>(),
            want
        );
        assert_eq!(
            sources
                .iter()
                .find(|s| s.domain == "custom.example.net")
                .unwrap()
                .is_custom,
            Some(true)
        );
        let domains: Vec<_> = f["response"]["body"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["domain"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            sources.iter().map(|s| s.domain.clone()).collect::<Vec<_>>(),
            domains
        );
        let types: Vec<Option<SourceType>> = f["response"]["body"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["type"].as_str().map(SourceType::from))
            .collect();
        assert_eq!(
            sources.iter().map(|s| s.r#type.clone()).collect::<Vec<_>>(),
            types
        );
        assert!(types.contains(&Some(SourceType::Blog)));
        assert!(types.contains(&Some(SourceType::FinancialMedia)));
    })
}

fn r11() {
    let f = fixture("fixtures/requests/sources-list-basic.json");
    run_paused(async move {
        let mock = HttpMock::start().await;
        mock.enqueue(Scripted::from_fixture(&f["response"]));
        let key = f["request"]["headers"]["X-API-KEY"].as_str().unwrap();
        let sources = client_with_key(&mock.base_url, key)
            .sources()
            .list()
            .await
            .unwrap();
        expect_request(&mock.requests()[0], &f["request"]);
        for s in sources.iter() {
            assert_eq!(s.has_content, None, "unknown, not false");
            let written = serde_json::to_value(s).unwrap();
            assert!(written.get("hasContent").is_none());
        }
        assert!(!sources.iter().any(|s| s.is_custom == Some(true)));
    })
}

fn r12() {
    let f = fixture("fixtures/requests/search-unknown-key.json");
    run_paused(async move {
        let mock = HttpMock::start().await;
        mock.always(Scripted::from_fixture(&f["response"]));
        match SearchParams::from_json(&f["sdk"]["args"].to_string()) {
            Err(Error::Validation(v)) => {
                assert_eq!(v.param(), Some("eventType"));
                assert!(v.to_string().contains("\"eventType\""));
            }
            other => panic!("expected a validation error, got {other:?}"),
        }
        // Shape errors are local too, for search and search_all, before any request.
        let client = client(&mock.base_url);
        let bad = SearchParams::new().limit(0);
        assert!(matches!(
            client.articles().search(&bad).await,
            Err(Error::Validation(_))
        ));
        assert!(matches!(
            client.articles().search_all(bad, SearchAllOptions::new()),
            Err(Error::Validation(_))
        ));
        assert_eq!(mock.count(), 0);
    })
}

fn p01() {
    let f = fixture("fixtures/pagination/three-pages.json");
    run_paused(async move {
        let mock = HttpMock::start().await;
        let steps = f["steps"].as_array().unwrap().clone();
        for step in &steps {
            mock.enqueue(Scripted::from_fixture(&step["response"]));
        }
        let client = client(&mock.base_url);
        let mut sources = vec!["news.example.com".to_owned(), "news.example.de".to_owned()];
        let params = SearchParams::new().sources(sources.clone()).limit(2);
        let mut result = client
            .articles()
            .search_all(params, SearchAllOptions::new())
            .unwrap();
        sources.push("mutated.example.org".to_owned()); // the caller's data cannot change later pages
        let mut got = Vec::new();
        while let Some(a) = result.next().await {
            got.push(a.unwrap().id);
        }
        let reqs = mock.requests();
        assert_eq!(reqs.len(), 3);
        for (req, step) in reqs.iter().zip(&steps) {
            expect_request(req, &step["request"]);
        }
        // Page 2 and 3 equal page 1 byte for byte apart from the cursor.
        for (i, req) in reqs.iter().enumerate().skip(1) {
            let cursor = steps[i]["request"]["body"]["cursor"].as_str().unwrap();
            let without_cursor = without(&req.body_json(), &["cursor"]).to_string();
            assert_eq!(without_cursor, reqs[0].body_text());
            assert_eq!(req.body_json()["cursor"], cursor);
        }
        let want: Vec<String> = steps
            .iter()
            .flat_map(|s| ids(&s["response"]["body"]))
            .collect();
        assert_eq!(got, want);
        assert_eq!(got.len(), 5);
        assert_eq!(result.pages_fetched(), 3);
        assert!(!result.truncated());
        assert!(result.next().await.is_none(), "iterated once");
        let request_ids: HashSet<_> = reqs
            .iter()
            .map(|r| r.header("x-request-id").unwrap().to_owned())
            .collect();
        assert_eq!(request_ids.len(), 3, "each page has its own X-Request-Id");

        // max_pages stops the walk; next_cursor() continues it.
        mock.reset();
        for step in &steps {
            mock.enqueue(Scripted::from_fixture(&step["response"]));
        }
        let params = SearchParams::new()
            .sources(["news.example.com", "news.example.de"])
            .limit(2);
        let mut result = client
            .articles()
            .search_all(params, SearchAllOptions::new().max_pages(2))
            .unwrap();
        let mut pages = 0;
        while let Some(page) = result.next_page().await {
            page.unwrap();
            pages += 1;
        }
        assert_eq!(pages, 2);
        assert!(result.truncated());
        assert_eq!(
            result.next_cursor(),
            steps[2]["request"]["body"]["cursor"].as_str()
        );
        // The other way of iterating is refused.
        assert!(matches!(
            result.next().await,
            Some(Err(Error::Validation(_)))
        ));
        // As a futures Stream.
        mock.reset();
        for step in &steps {
            mock.enqueue(Scripted::from_fixture(&step["response"]));
        }
        let params = SearchParams::new()
            .sources(["news.example.com", "news.example.de"])
            .limit(2);
        let stream = client
            .articles()
            .search_all(params, SearchAllOptions::new())
            .unwrap();
        let all: Vec<_> = stream.collect().await;
        assert_eq!(all.len(), 5);
    })
}

fn p02() {
    let f = fixture("fixtures/pagination/resize-and-reproject.json");
    run_paused(async move {
        let mock = HttpMock::start().await;
        mock.enqueue(Scripted::from_fixture(&f["response"]));
        let params = SearchParams::from_json(&f["request"]["body"].to_string()).unwrap();
        let page = client(&mock.base_url)
            .articles()
            .search(&params)
            .await
            .unwrap();
        expect_request(&mock.requests()[0], &f["request"]);
        assert_eq!(page.articles.len(), 3);
        assert!(page.articles.iter().all(|a| a.content_present()));
    })
}

fn cursor_mismatch(file: &'static str) -> impl FnOnce() {
    move || {
        let f = fixture(&format!("fixtures/pagination/{file}"));
        let three = fixture("fixtures/pagination/three-pages.json");
        run_paused(async move {
            let mock = HttpMock::start().await;
            for step in three["steps"].as_array().unwrap() {
                mock.enqueue(Scripted::from_fixture(&step["response"]));
            }
            let client = client(&mock.base_url);
            // The pager never injects keys: later pages are page 1 plus the cursor.
            let params = SearchParams::new()
                .sources(["news.example.com", "news.example.de"])
                .limit(2);
            let mut result = client
                .articles()
                .search_all(params, SearchAllOptions::new())
                .unwrap();
            while let Some(a) = result.next().await {
                a.unwrap();
            }
            for r in mock.requests().iter().skip(1) {
                let keys: Vec<String> =
                    r.body_json().as_object().unwrap().keys().cloned().collect();
                assert_eq!(keys, vec!["cursor", "limit", "sources"]);
            }
            mock.reset();
            // The server's answer to a changed body is a terminal CURSOR_MISMATCH.
            mock.always(Scripted::from_fixture(&f["response"]));
            let params = SearchParams::from_json(&f["request"]["body"].to_string()).unwrap();
            let err = client.articles().search(&params).await.unwrap_err();
            expect_request(&mock.requests()[0], &f["request"]);
            let Error::Api(e) = err else {
                panic!("expected an API error")
            };
            assert_eq!(e.code().unwrap().as_str(), "CURSOR_MISMATCH");
            assert_eq!(e.api_kind().as_str(), "cursor_mismatch");
            assert_eq!(e.errors()[0].field.as_deref(), Some("cursor"));
            assert_eq!(mock.count(), 1, "not retried");
            if f["id"] == "P-05" {
                let d = e.detail().unwrap();
                assert!(d.contains("publishedAt:desc") && d.contains("publishedAt:asc"));
            }
        })
    }
}

fn p06() {
    let f = fixture("fixtures/pagination/reordered-list.json");
    run_paused(async move {
        let mock = HttpMock::start().await;
        mock.enqueue(Scripted::from_fixture(&f["response"]));
        let params = SearchParams::from_json(&f["request"]["body"].to_string()).unwrap();
        let page = client(&mock.base_url)
            .articles()
            .search(&params)
            .await
            .unwrap();
        expect_request(&mock.requests()[0], &f["request"]);
        assert!(page.pagination.has_more);
        assert_eq!(
            page.pagination.next_cursor.as_deref(),
            f["response"]["body"]["pagination"]["nextCursor"].as_str()
        );
    })
}

/// Drives the SDK call that matches the fixture request as closely as the public API allows.
/// Returns whether the request the SDK sent equals the fixture exactly.
async fn call_for(client: &Client, req: &Value) -> (Error, bool) {
    let method = req["method"].as_str().unwrap();
    let path = req["path"].as_str().unwrap();
    let pairs = query_pairs(req["query"].as_str());
    let q = |k: &str| pairs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    let include = || {
        let mut o = GetArticleOptions::new();
        if let Some(i) = q("include") {
            o = o.include(i.split(',').map(str::to_owned).collect::<Vec<_>>());
        }
        o
    };
    let result: Result<(), Error> = match (method, path) {
        ("POST", "/v3/articles/search") => {
            let parsed = req
                .get("body")
                .and_then(|b| SearchParams::from_json(&b.to_string()).ok())
                .filter(client_side_ok);
            let exact = parsed.is_some() && req.get("rawBody").is_none();
            let params = parsed.unwrap_or_default();
            let r = client.articles().search(&params).await.map(|_| ());
            return (r.unwrap_err(), exact);
        }
        ("GET", "/v3/articles/by-link") => {
            let r = client
                .articles()
                .get_by_link(&q("link").unwrap(), &include())
                .await
                .map(|_| ());
            return (r.unwrap_err(), true);
        }
        ("GET", p) if p.starts_with("/v3/articles/") => {
            let id = p.trim_start_matches("/v3/articles/");
            client.articles().get(id, &include()).await.map(|_| ())
        }
        ("GET", "/v3/companies") => {
            if let Some(ids) = q("ids") {
                let parsed: Vec<i64> = ids.split(',').filter_map(|s| s.parse().ok()).collect();
                let exact = parsed.len() == ids.split(',').count() && q("q").is_none();
                let r = client.companies().get_many(&parsed).await.map(|_| ());
                return (r.unwrap_err(), exact);
            }
            let exact = q("q").is_some();
            let r = client
                .companies()
                .search(&CompanySearchParams::new(
                    q("q").unwrap_or_else(|| "XMPL".to_owned()),
                ))
                .await
                .map(|_| ());
            return (r.unwrap_err(), exact);
        }
        ("GET", p) if p.starts_with("/v3/companies/") => {
            match p.trim_start_matches("/v3/companies/").parse::<i64>() {
                Ok(id) => client.companies().get(id).await.map(|_| ()),
                Err(_) => {
                    // A non-integer id cannot be passed; the server answer is what is tested.
                    let r = client.companies().get(1).await.map(|_| ());
                    return (r.unwrap_err(), false);
                }
            }
        }
        // An unknown route: no SDK method reaches it; another GET carries the server answer.
        _ => {
            let r = client.sources().list().await.map(|_| ());
            return (r.unwrap_err(), false);
        }
    };
    (result.unwrap_err(), true)
}

/// The fixture body passes the SDK's local shape checks.
fn client_side_ok(p: &SearchParams) -> bool {
    !(p.limit.is_some_and(|l| !(1..=100).contains(&l))
        || p.languages.as_ref().is_some_and(Vec::is_empty)
        || p.include.as_ref().is_some_and(|i| i.len() > 3))
}

type Issue = (Option<String>, Option<String>, Option<Vec<String>>);

fn check_api_error(err: &ApiError, f: &Value, sent_id: &str) {
    let resp = &f["response"];
    let expect = &f["expect"];
    assert_eq!(err.status() as u64, resp["status"].as_u64().unwrap());
    assert_eq!(err.api_kind().as_str(), expect["apiKind"].as_str().unwrap());
    match expect["code"].as_str() {
        Some("NESTJS_FALLBACK") => {
            assert!(err.code().is_none());
            assert_eq!(err.detail(), resp["body"]["message"].as_str());
            assert_eq!(err.request_id(), Some(sent_id));
        }
        None => {
            assert!(err.code().is_none());
            assert_eq!(err.raw(), resp["body"].as_str());
            assert_eq!(err.request_id(), Some(sent_id));
        }
        Some(code) => {
            let body = &resp["body"];
            assert_eq!(err.code().unwrap().as_str(), code);
            assert_eq!(err.detail(), body["detail"].as_str());
            assert_eq!(err.title(), body["title"].as_str());
            assert_eq!(err.r#type(), body["type"].as_str());
            assert_eq!(err.request_id(), body["requestId"].as_str());
            let want: Vec<Issue> = body["errors"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .map(|e| {
                            (
                                e["field"].as_str().map(str::to_owned),
                                e["issue"].as_str().map(str::to_owned),
                                e["allowedValues"].as_array().map(|a| {
                                    a.iter().map(|v| v.as_str().unwrap().to_owned()).collect()
                                }),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            let got: Vec<_> = err
                .errors()
                .iter()
                .map(|e| (e.field.clone(), e.issue.clone(), e.allowed_values.clone()))
                .collect();
            assert_eq!(got, want);
            assert_eq!(
                serde_json::from_str::<Value>(err.raw().unwrap()).unwrap(),
                *body
            );
        }
    }
}

fn error_case(file: &'static str) -> impl FnOnce() {
    move || {
        let f = fixture(&format!("fixtures/errors/{file}"));
        run_paused(async move {
            let mock = HttpMock::start().await;
            mock.always(Scripted::from_fixture(&f["response"]));
            let req = &f["request"];
            let key = req["headers"]["X-API-KEY"].as_str().unwrap_or("sk_missing");
            let client = client_with_key(&mock.base_url, key);
            let start = tokio::time::Instant::now();
            let (err, exact) = call_for(&client, req).await;
            let waited = start.elapsed();
            let reqs = mock.requests();
            if exact {
                let mut want = req.clone();
                if want.get("rawBody").is_some() {
                    want.as_object_mut().unwrap().remove("rawBody");
                }
                expect_request(&reqs[0], &want);
            }
            let sent_ids: HashSet<_> = reqs
                .iter()
                .map(|r| r.header("x-request-id").unwrap().to_owned())
                .collect();
            assert_eq!(
                sent_ids.len(),
                1,
                "one X-Request-Id per logical call, the same on every retry"
            );
            let sent_id = sent_ids.into_iter().next().unwrap();
            let Error::Api(e) = &err else {
                panic!("expected an API error, got {err:?}")
            };
            assert_eq!(err.kind(), "api");
            check_api_error(e, &f, &sent_id);
            match f["expect"]["retry"].as_str().unwrap() {
                "never" => {
                    assert_eq!(reqs.len(), 1);
                    assert_eq!(waited, Duration::ZERO, "no wait");
                }
                "backoff" => {
                    assert_eq!(reqs.len(), 4, "3 retries after the first attempt");
                    assert!(waited > Duration::ZERO && waited <= Duration::from_secs(8));
                }
                "once-after-burst-wait" => {
                    assert_eq!(reqs.len(), 2);
                    let header = f["response"]["headers"]["retry-after"]
                        .as_str()
                        .map(|s| s.parse::<u64>().unwrap());
                    assert_eq!(e.retry_after(), header.map(Duration::from_secs));
                    assert_eq!(
                        waited,
                        Duration::from_secs(header.unwrap_or(10)),
                        "exactly Retry-After"
                    );
                }
                other => panic!("unknown retry class {other}"),
            }
            match f["id"].as_str().unwrap() {
                "E-18" => {
                    assert_eq!(e.limit_type().unwrap().as_str(), "monthly");
                    assert!(e.retry_after().is_none());
                }
                "E-19" => assert_eq!(e.limit_type().unwrap().as_str(), "burst"),
                "E-20" => assert_eq!(e.detail(), Some("The request could not be processed.")),
                "E-23" => assert!(e.request_id().unwrap().starts_with("req_")),
                "E-02" => {
                    let d = e.detail().unwrap();
                    assert!(d.ends_with(" request fields are not valid."));
                }
                _ => {}
            }
            if f["id"] == "E-18" {
                monthly_in_search_all(&f).await;
            }
            if f["id"] == "E-19" {
                burst_switches(&f).await;
            }
            if f["id"] == "E-04" || f["id"] == "E-06" || f["id"] == "E-07" {
                local_rejections(&f, &mock.base_url).await;
            }
        })
    }
}

/// E-18: a monthly 429 ends search_all at once, without waiting.
async fn monthly_in_search_all(f: &Value) {
    let mock = HttpMock::start().await;
    mock.always(Scripted::from_fixture(&f["response"]));
    let client = client(&mock.base_url);
    let start = tokio::time::Instant::now();
    let mut result = client
        .articles()
        .search_all(SearchParams::new(), SearchAllOptions::new())
        .unwrap();
    assert!(matches!(result.next().await, Some(Err(Error::Api(_)))));
    assert!(result.next().await.is_none());
    assert_eq!(mock.count(), 1);
    assert_eq!(start.elapsed(), Duration::ZERO);
}

/// E-19: the burst retry is on by default, can be turned off, and waits 10 s without Retry-After.
async fn burst_switches(f: &Value) {
    let mock = HttpMock::start().await;
    mock.always(Scripted::from_fixture(&f["response"]));
    let config = Config::builder(KIT_API_KEY)
        .base_url(&mock.base_url)
        .http_client(test_http_client())
        .retry_on_burst(finlight::RetryOnBurst::Off)
        .build()
        .unwrap();
    let client = Client::new(config).unwrap();
    let start = tokio::time::Instant::now();
    assert!(
        client
            .articles()
            .search(&SearchParams::new())
            .await
            .is_err()
    );
    assert_eq!(mock.count(), 1);
    assert_eq!(start.elapsed(), Duration::ZERO);

    let mut resp = f["response"].clone();
    resp["headers"]
        .as_object_mut()
        .unwrap()
        .remove("retry-after");
    mock.reset();
    mock.always(Scripted::from_fixture(&resp));
    let client = self::client(&mock.base_url);
    let start = tokio::time::Instant::now();
    let err = client
        .articles()
        .search(&SearchParams::new())
        .await
        .unwrap_err();
    assert_eq!(mock.count(), 2);
    assert_eq!(start.elapsed(), Duration::from_secs(10));
    assert!(err.as_api().unwrap().retry_after().is_none());
    // The burst retry does not count against max_retries.
    let config = Config::builder(KIT_API_KEY)
        .base_url(&mock.base_url)
        .http_client(test_http_client())
        .max_retries(0)
        .build()
        .unwrap();
    mock.reset();
    mock.always(Scripted::from_fixture(&f["response"]));
    let _ = Client::new(config)
        .unwrap()
        .articles()
        .search(&SearchParams::new())
        .await;
    assert_eq!(mock.count(), 2);
}

/// E-04, E-06, E-07: what the SDK rejects locally never reaches the server.
async fn local_rejections(f: &Value, base_url: &str) {
    let client = client(base_url);
    match f["id"].as_str().unwrap() {
        "E-04" => {
            let p = SearchParams::from_json(&f["request"]["body"].to_string()).unwrap();
            assert!(matches!(
                client.articles().search(&p).await,
                Err(Error::Validation(_))
            ));
        }
        _ => {
            assert!(matches!(
                client
                    .companies()
                    .search(&CompanySearchParams::new(""))
                    .await,
                Err(Error::Validation(_))
            ));
            assert!(matches!(
                client.companies().get_many(&[]).await,
                Err(Error::Validation(_))
            ));
        }
    }
}

/// F-04, REST part: an unknown problem code and an unknown limitType.
fn f04() {
    let f = fixture("fixtures/forward-compat/unknown-events-and-codes.json");
    let items = f["items"].as_array().unwrap().clone();
    crate::webhooks::f04_webhook(items.iter().find(|i| i["kind"] == "webhook.event").unwrap());
    run_paused(async move {
        for item in items.iter().filter(|i| i["kind"] != "webhook.event") {
            let mock = HttpMock::start().await;
            mock.always(Scripted::from_fixture(&json!({
                "status": item["status"], "headers": item["headers"], "body": item["wire"]
            })));
            let start = tokio::time::Instant::now();
            let err = client(&mock.base_url)
                .articles()
                .search(&SearchParams::new())
                .await
                .unwrap_err();
            let e = err.as_api().unwrap();
            assert_eq!(mock.count(), 1, "not retried");
            assert_eq!(start.elapsed(), Duration::ZERO);
            assert_eq!(e.request_id(), Some("req_synthetic"));
            assert_eq!(e.detail(), item["wire"]["detail"].as_str());
            if item["kind"] == "problem.code" {
                assert_eq!(e.api_kind().as_str(), "other");
                assert_eq!(e.code().unwrap().as_str(), "PAYMENT_REQUIRED");
                assert_eq!(e.status(), 402);
            } else {
                assert_eq!(e.api_kind().as_str(), "rate_limited");
                assert_eq!(e.limit_type().unwrap().as_str(), "daily");
            }
        }
    });
}

pub fn cases() -> Vec<crate::common::Case> {
    vec![
        ("F-04", Box::new(f04)),
        ("R-01", Box::new(r01)),
        ("R-02", Box::new(r02)),
        ("R-03", Box::new(r03)),
        ("R-04", Box::new(r04)),
        ("R-05", Box::new(r05)),
        ("R-06", Box::new(r06)),
        ("R-07", Box::new(r07)),
        ("R-08", Box::new(r08)),
        ("R-09", Box::new(r09)),
        ("R-10", Box::new(r10)),
        ("R-11", Box::new(r11)),
        ("R-12", Box::new(r12)),
        ("P-01", Box::new(p01)),
        ("P-02", Box::new(p02)),
        ("P-03", Box::new(cursor_mismatch("injected-to.json"))),
        (
            "P-04",
            Box::new(cursor_mismatch("injected-default-languages.json")),
        ),
        ("P-05", Box::new(cursor_mismatch("changed-order.json"))),
        ("P-06", Box::new(p06)),
        ("E-01", Box::new(error_case("validation-enum.json"))),
        ("E-02", Box::new(error_case("validation-multiple.json"))),
        (
            "E-03",
            Box::new(error_case("validation-query-language.json")),
        ),
        (
            "E-04",
            Box::new(error_case("validation-empty-languages.json")),
        ),
        (
            "E-05",
            Box::new(error_case("validation-include-query.json")),
        ),
        ("E-06", Box::new(error_case("companies-neither.json"))),
        ("E-07", Box::new(error_case("companies-both.json"))),
        (
            "E-08",
            Box::new(error_case("companies-ids-not-integer.json")),
        ),
        ("E-09", Box::new(error_case("company-id-not-integer.json"))),
        ("E-10", Box::new(error_case("cursor-invalid.json"))),
        ("E-11", Box::new(error_case("cursor-version.json"))),
        (
            "E-12",
            Box::new(error_case("unauthorized-missing-key.json")),
        ),
        (
            "E-13",
            Box::new(error_case("unauthorized-invalid-key.json")),
        ),
        ("E-14", Box::new(error_case("forbidden-companies.json"))),
        ("E-15", Box::new(error_case("not-found-id.json"))),
        ("E-16", Box::new(error_case("not-found-link.json"))),
        ("E-17", Box::new(error_case("not-found-company.json"))),
        ("E-18", Box::new(error_case("rate-limited-monthly.json"))),
        ("E-19", Box::new(error_case("rate-limited-burst.json"))),
        ("E-20", Box::new(error_case("internal-error.json"))),
        ("E-21", Box::new(error_case("malformed-json-body.json"))),
        ("E-22", Box::new(error_case("unknown-route.json"))),
        ("E-23", Box::new(error_case("generated-request-id.json"))),
        ("E-24", Box::new(error_case("gateway-non-json.json"))),
        ("E-25", Box::new(error_case("cursor-mismatch.json"))),
    ]
}
