# Changelog

## 0.1.0

Initial release.

- REST API: article search, fetch by link, source listing — with retry and exponential backoff
- WebSocket streaming: enhanced and raw article streams as async `Stream`s with auto-reconnect, application-level keepalive with pong watchdog, proactive connection rotation, rate-limit and admin-kick handling, and duplicate suppression on the enhanced stream
- Webhook verification: HMAC-SHA256 with optional timestamp binding and 5-minute replay tolerance
- Flexible model parsing: confidence as number or string, timestamps in all API formats
