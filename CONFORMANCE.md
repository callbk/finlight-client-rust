# Conformance case map

Every case of the finlight API v3 contract kit 3.0.0-rc.5 (`conformance/conformance.md`) and the test that runs it.
Each case is its own test in `cargo test --test conformance`, named `<case id> <title>`;
run one with `cargo test --test conformance -- S-18`. The test `kit: every case has a test and a CONFORMANCE.md row`
fails when a kit case has no test or no row here, and `kit: conformance/ matches its MANIFEST` fails when the copy
differs from the MANIFEST. Re-sync the kit with `scripts/sync-conformance.sh <kit dir>`.

HTTP cases run against a local HTTP/1.1 server on 127.0.0.1 that records the bytes the SDK sent. WebSocket cases run
against an in-process WebSocket server on 127.0.0.1, on the paused tokio clock (`tokio` `test-util`).
Client steps of a WebSocket session that the SDK never produces (a truncated frame, a legacy key, a ping on `$default`,
a re-subscribe on the same connection, a ping before the subscribe) are not sent; the server side of the session is
replayed as is. Error fixtures whose request the typed API cannot produce (a non-integer id, `q` and `ids` together,
an unknown route, a malformed body) are driven through the nearest SDK call; the server answer is what is tested, and
the local rejection of the bad input is checked in the same test.

| Case | Title | Test (`tests/conformance/…`) |
|---|---|---|
| V-01 | Known constants equal vocab.json | `models.rs` `v01` |
| D-01 | Full article: every include, every entitlement | `models.rs` `decode_case("full-article.json")` |
| D-02 | Raw skeleton (raw channel ignores include and entitlements) | `models.rs` `decode_case("raw-skeleton.json")` |
| D-03 | No include, every entitlement | `models.rs` `decode_case("no-include-full-access.json")` |
| D-04 | Every include requested, no entitlement | `models.rs` `decode_case("all-include-no-access.json")` |
| D-05 | events requested and entitled, companies not requested | `models.rs` `decode_case("events-without-companies.json")` |
| D-06 | events and companies requested, entityAccess missing | `models.rs` `decode_case("events-companies-not-entitled.json")` |
| D-07 | Legacy values delivered as they are | `models.rs` `decode_case("legacy-values.json")` |
| D-08 | Non-ASCII text | `models.rs` `decode_case("non-ascii.json")` |
| D-09 | Sentiment with a null label, event with null direction/importance/confidence, companies [] | `models.rs` `decode_case("partial-values.json")` |
| D-10 | Decode matrix (include x entitlements x enriched/raw) | `models.rs` `d10` |
| D-11 | Blank summary and content arrive as null | `models.rs` `decode_case("blank-text.json")` |
| F-01 | Unknown fields at every level of the article | `models.rs` `f01` |
| F-02 | Unknown values in every open response enum | `models.rs` `f02` |
| F-03 | New keys on REST, webhook and WebSocket envelopes | `models.rs` `f03` |
| F-04 | Unknown webhook event name, unknown problem code, unknown 429 limitType | `rest.rs` `f04` |
| R-01 | articles.search with one parameter sends exactly that parameter | `rest.rs` `r01` |
| R-02 | articles.search with every parameter | `rest.rs` `r02` |
| R-03 | articles.search without parameters | `rest.rs` `r03` |
| R-04 | articles.get with include, comma-joined | `rest.rs` `r04` |
| R-05 | articles.getByLink URL-encodes the link | `rest.rs` `r05` |
| R-06 | companies.search with q and mic | `rest.rs` `r06` |
| R-07 | companies.getMany joins ids with commas | `rest.rs` `r07` |
| R-08 | companies.get returns ETag and Cache-Control | `rest.rs` `r08` |
| R-09 | companies.get with If-None-Match answers 304 | `rest.rs` `r09` |
| R-10 | sources.list with fullArticleAccess | `rest.rs` `r10` |
| R-11 | sources.list without fullArticleAccess | `rest.rs` `r11` |
| R-12 | An unknown request key is dropped silently by the server | `rest.rs` `r12` |
| R-13 | WebSocket subscribe frame, enriched | `sessions.rs` `r13` |
| R-14 | WebSocket subscribe frame, raw | `sessions.rs` `r14` |
| P-01 | Three-page cursor walk | `rest.rs` `p01` |
| P-02 | limit and include may change between pages | `rest.rs` `p02` |
| P-03 | Adding "to" on page 2 breaks the cursor | `rest.rs` `cursor_mismatch("injected-to.json")` |
| P-04 | Adding the default languages ["en"] on page 2 breaks the cursor | `rest.rs` `cursor_mismatch("injected-default-languages.json")` |
| P-05 | Changing order on page 2 | `rest.rs` `cursor_mismatch("changed-order.json")` |
| P-06 | Reordering list values keeps the cursor valid | `rest.rs` `p06` |
| E-01 | Unknown enum value | `rest.rs` `error_case("validation-enum.json")` |
| E-02 | Several invalid fields at once | `rest.rs` `error_case("validation-multiple.json")` |
| E-03 | Unknown field in the query language | `rest.rs` `error_case("validation-query-language.json")` |
| E-04 | Empty languages array | `rest.rs` `error_case("validation-empty-languages.json")` |
| E-05 | Unknown include value on a GET lookup | `rest.rs` `error_case("validation-include-query.json")` |
| E-06 | companies.search without q and ids | `rest.rs` `error_case("companies-neither.json")` |
| E-07 | companies.search with q and ids | `rest.rs` `error_case("companies-both.json")` |
| E-08 | Non-integer company id in ids | `rest.rs` `error_case("companies-ids-not-integer.json")` |
| E-09 | Non-integer companyId in the path | `rest.rs` `error_case("company-id-not-integer.json")` |
| E-10 | Cursor that is not a cursor | `rest.rs` `error_case("cursor-invalid.json")` |
| E-11 | Cursor of an unsupported version | `rest.rs` `error_case("cursor-version.json")` |
| E-12 | No API key | `rest.rs` `error_case("unauthorized-missing-key.json")` |
| E-13 | Unknown API key | `rest.rs` `error_case("unauthorized-invalid-key.json")` |
| E-14 | Company lookup without entityAccess | `rest.rs` `error_case("forbidden-companies.json")` |
| E-15 | Unknown article id | `rest.rs` `error_case("not-found-id.json")` |
| E-16 | Unknown article link | `rest.rs` `error_case("not-found-link.json")` |
| E-17 | Unknown company id | `rest.rs` `error_case("not-found-company.json")` |
| E-18 | Monthly token limit reached | `rest.rs` `error_case("rate-limited-monthly.json")` |
| E-19 | Burst limit reached | `rest.rs` `error_case("rate-limited-burst.json")` |
| E-20 | Server failure | `rest.rs` `error_case("internal-error.json")` |
| E-21 | Malformed JSON body (NestJS fallback form) | `rest.rs` `error_case("malformed-json-body.json")` |
| E-22 | Unknown route under /v3 (NestJS fallback form) | `rest.rs` `error_case("unknown-route.json")` |
| E-23 | No X-Request-Id sent: the server generates one | `rest.rs` `error_case("generated-request-id.json")` |
| E-24 | Non-JSON error body | `rest.rs` `error_case("gateway-non-json.json")` |
| E-25 | Cursor issued for another sort order | `rest.rs` `error_case("cursor-mismatch.json")` |
| W-01 | article.created, enriched, every include, non-ASCII body | `webhooks.rs` `case("created-enriched.json")` |
| W-02 | article.updated with changedFields | `webhooks.rs` `case("updated-with-changed-fields.json")` |
| W-03 | article.updated for a plan without fullArticleAccess | `webhooks.rs` `case("updated-content-not-entitled.json")` |
| W-04 | article.created on a raw webhook | `webhooks.rs` `case("created-raw.json")` |
| W-05 | Dashboard test webhook (evt_test…) | `webhooks.rs` `case("test-webhook.json")` |
| W-06 | v2 body is reported as wrong payload version | `webhooks.rs` `case("v2-body.json")` |
| W-07 | Body changed after signing | `webhooks.rs` `case("neg-tampered-body.json")` |
| W-08 | Timestamp older than the tolerance | `webhooks.rs` `case("neg-stale-timestamp.json")` |
| W-09 | Timestamp further in the future than the tolerance | `webhooks.rs` `case("neg-future-timestamp.json")` |
| W-10 | Timestamp header is not ISO 8601 | `webhooks.rs` `case("neg-unparseable-timestamp.json")` |
| W-11 | Timestamp in unix seconds instead of ISO 8601 | `webhooks.rs` `case("neg-unix-seconds-timestamp.json")` |
| W-12 | No X-Webhook-Timestamp header | `webhooks.rs` `case("neg-missing-timestamp.json")` |
| W-13 | No X-Webhook-Signature header | `webhooks.rs` `case("neg-missing-signature.json")` |
| W-14 | Signature without the sha256= prefix | `webhooks.rs` `case("neg-wrong-prefix.json")` |
| W-15 | Signature computed with the hex-decoded secret | `webhooks.rs` `case("neg-secret-hex-decoded.json")` |
| W-16 | Case-insensitive header lookup | `webhooks.rs` `case("lowercase-headers.json")` |
| S-01 | Ping before subscribe gets a v2 pong; after the v3 subscribe pongs are v3 | `sessions.rs` `s01` |
| S-02 | Lowercase x-api-key on the wire | `sessions.rs` `s02` |
| S-03 | connection.admitted echoes the nonce; a re-subscribe re-admits only with the same nonce | `sessions.rs` `simple("admitted-nonce.json")` |
| S-04 | article.created then article.updated for the same id, then an exact duplicate | `sessions.rs` `simple("created-then-updated.json")` |
| S-05 | An unknown event name arrives | `sessions.rs` `s05` |
| S-06 | connection.error PING_ON_DEFAULT_ROUTE | `sessions.rs` `simple("ping-on-default-route.json")` |
| S-07 | VALIDATION_FAILED for a v2 key in a v3 subscribe | `sessions.rs` `s07` |
| S-08 | VALIDATION_FAILED for include on raw | `sessions.rs` `s08` |
| S-09 | connection.preempted, reason connection_limit (another connection took over) | `sessions.rs` `simple("preempted-connection-limit.json")` |
| S-10 | connection.preempted, reason quota_exceeded | `sessions.rs` `simple("preempted-quota-exceeded.json")` |
| S-11 | Preempted before the first subscribe: the frame comes in v2 form | `sessions.rs` `simple("v2-preempt-before-subscribe.json")` |
| S-12 | Error for an unreadable message comes in v2 form | `sessions.rs` `s12` |
| S-13 | connection.error HANDSHAKE_FAILED | `sessions.rs` `simple("handshake-failed.json")` |
| S-14 | connection.error SEARCH_FAILED | `sessions.rs` `s14` |
| S-15 | connection.error INTERNAL_ERROR | `sessions.rs` `simple("internal-error.json")` |
| S-16 | No pong for 60 s | `sessions.rs` `s16` |
| S-17 | Raw stream: admitted, no initial push, skeleton frames | `sessions.rs` `s17` |
| S-18 | Proactive rotation before the 2 h connection lifetime | `sessions.rs` `s18` |
| S-19 | Dial refused: no API key | `sessions.rs` `simple("dial-missing-key.json")` |
| S-20 | Dial refused: invalid API key | `sessions.rs` `simple("dial-invalid-key.json")` |
| S-21 | Dial refused: API key in the wrong format | `sessions.rs` `simple("dial-malformed-key.json")` |
| S-22 | Dial refused: plan without WebSockets | `sessions.rs` `simple("dial-no-websockets.json")` |
| S-23 | Dial refused: connection limit reached, no takeover | `sessions.rs` `simple("dial-too-many-connections.json")` |
| S-24 | Dial refused: WebSocket quota exhausted | `sessions.rs` `simple("dial-quota-exceeded.json")` |
| S-25 | Dial refused: lost the admit race | `sessions.rs` `simple("dial-admit-race.json")` |
| S-26 | Dial refused: server error | `sessions.rs` `simple("dial-server-error.json")` |
| S-27 | connection.preempted, reason quota_exceeded, with quotaReconnect turned off | `sessions.rs` `simple("preempted-quota-exceeded-no-reconnect.json")` |
| S-28 | Dial refused: WebSocket quota exhausted, with quotaReconnect turned off | `sessions.rs` `simple("dial-quota-exceeded-no-reconnect.json")` |
| S-29 | VALIDATION_FAILED for an unknown sourceTypes value | `sessions.rs` `s29` |
