# Changelog

## 0.2.0

**Breaking release: finlight API v3 only.** 0.2 speaks REST `/v3`, WebSocket `payloadVersion` 3 and v3 webhooks, and nothing of API v1/v2. It does not run 0.1 code. Cargo keeps `"0.1"` on 0.1; to stay on API v2, do not change that requirement. See "Migrating from 0.1" in the README and the guide on [docs.finlight.me](https://docs.finlight.me).

- The library is imported as `finlight` (`use finlight::Client`); the crate stays `finlight-client`.
- REST: `articles().search()`, `search_all()` (cursor pager, frozen body, `max_pages`), `get()`, `get_by_link()`; `companies().search()`, `get_many()`, `get()` and `get_if_none_match()`; `sources().list()`. The SDK sends only what you set and checks the shape locally.
- Search filters `source_types` and `exclude_source_types` with the open `SourceType` vocabulary (`newswire`, `press_release`, `financial_media`, `general_media`, `aggregator`, `blog`, `government`). Article search, `search_all`, `StreamParams` and `RawStreamParams` take them (sent as `sourceTypes` / `excludeSourceTypes` only when set). `source.type` and `Source::type` are `None` when a source is not classified.
- Models follow the v3 wire format: tolerant readers, three states for gated keys (`Field<T>`, `Option<_>`), timestamps as sent plus strict `*_time()` accessors, open vocabularies, `raw_json()`, broken list entries in `skipped`.
- Errors: one `Error` enum with `kind()`; `ApiError` reads problem+json, the NestJS fallback and text bodies, with `api_kind()`, `limit_type()` and `retry_after()`. `FieldIssue::allowed_values` is filled from REST problems and from stream `connection.error` details.
- Retries: 5xx, network errors and timeouts up to 3 times with backoff; a burst 429 waits exactly `Retry-After` (10 s without it) and retries once, on by default (`RetryOnBurst::Off`); a monthly 429 fails at once, also in `search_all`. One `X-Request-Id` per call, kept across retries.
- Streams: `client.stream()` and `client.raw_stream()` as `futures` streams. Ping every 25 s, 60 s pong watchdog, renewal after 115 min, re-subscribe on every reconnect, duplicate filter, v2-form control frames understood, hooks `on_admitted`, `on_reconnect`, `on_error`, `on_quota_wait`. A used-up WebSocket quota redials every hour by default (`Config::quota_reconnect`); a `connection_limit` preemption ends the stream with `Error::Preempted`. A refused renewal dial (a plan at its connection limit) closes the old connection and dials again at once. The reconnect backoff starts over only after a pong or an article, not on `connection.admitted`.
- The SDK speaks the WebSocket protocol itself; tokio-tungstenite and webpki-roots are no longer dependencies. Stream TLS verifies against the operating system's trust store, like REST (rustls-platform-verifier).
- Webhooks: `webhooks::construct_event()`, `construct_event_from_values()` and `verify_signature()` with strict timestamps and header checks; a correctly signed v2 body returns `WebhookPayload::WrongPayloadVersion` (answer it with 2xx).
- Every request and dial sends `User-Agent: finlight-client-rust/0.2.0` and `x-client-version: 0.2.0`; the stream dial sends the API key only as the lowercase `x-api-key` header.
- Logging through `tracing` (target `finlight`); keys, secrets, signatures and bodies are never logged.
- MSRV stays 1.85. chrono and thiserror are no longer dependencies.
- Lookups (`companies()`, `sources().list()`) cost 1 credit per request, like an article request; a 304 revalidation and errors cost 1 credit too. The method docs say so.
- The SDK keeps no cache of company records. `companies().get()` sends no `If-None-Match`; `cache_info()` gives the `ETag` and `Cache-Control` of a record, so you can keep records yourself and pass the ETag to `get_if_none_match()` (`Ok(None)` on 304). A 304 costs 1 credit too, so to save credits do not call again within `max-age`. Removed before release (pre-release builds had them): `Config::company_cache_size`, `ConfigBuilder::company_cache_size()` and `CacheInfo::not_modified`.
- Tested against the API v3 contract kit 3.0.0-rc.6: every kit case runs as its own test (`CONFORMANCE.md`).

## 0.1.1

No functional changes. Validates the trusted-publishing release pipeline.

## 0.1.0

Initial release for API v2.
