//! Decoder of WebSocket frames.
//!
//! A frame with `event` is a v3 frame. A frame with only `action` is a control frame in the
//! older v2 form, which the server still sends before the first v3 subscribe (pong, preempted,
//! error for an unreadable message, admit). It is mapped onto the same frame types. A v2
//! `sendArticle` is never delivered as an article.

use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{Map, Value};

use crate::vocab::{ConnectionErrorCode, PreemptReason, StreamEventName};

#[derive(Debug)]
pub(crate) enum Frame {
    Admitted {
        lease_id: Option<String>,
        server_now: Option<i64>,
        client_nonce: Option<String>,
    },
    Pong,
    Preempted {
        reason: PreemptReason,
        new_lease_id: Option<String>,
        timestamp: Option<i64>,
    },
    Error {
        code: Option<ConnectionErrorCode>,
        message: Option<String>,
        details: Option<Value>,
    },
    Article {
        event: StreamEventName,
        updated: bool,
        data: Box<RawValue>,
    },
    Other {
        event: StreamEventName,
    },
    Ignored(&'static str),
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    event: Option<Value>,
    #[serde(default)]
    action: Option<Value>,
    #[serde(default)]
    data: Option<Box<RawValue>>,
}

fn obj(raw: &Option<Box<RawValue>>) -> Map<String, Value> {
    raw.as_ref()
        .and_then(|r| serde_json::from_str::<Value>(r.get()).ok())
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default()
}

fn s(m: &Map<String, Value>, key: &str) -> Option<String> {
    m.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn int(m: &Map<String, Value>, key: &str) -> Option<i64> {
    m.get(key).and_then(Value::as_i64)
}

/// Decodes one text frame. `None` when the text is not a JSON object.
pub(crate) fn decode(text: &str) -> Option<Frame> {
    let env: Envelope = serde_json::from_str(text).ok()?;
    if let Some(event) = env.event {
        let Value::String(event) = event else {
            return Some(Frame::Ignored("event is not a string"));
        };
        let name = StreamEventName::from(event);
        return Some(match name {
            StreamEventName::ArticleCreated | StreamEventName::ArticleUpdated => match env.data {
                Some(data) => Frame::Article {
                    updated: name == StreamEventName::ArticleUpdated,
                    event: name,
                    data,
                },
                None => Frame::Ignored("article frame without data"),
            },
            StreamEventName::ConnectionAdmitted => {
                let d = obj(&env.data);
                Frame::Admitted {
                    lease_id: s(&d, "leaseId"),
                    server_now: int(&d, "serverNow"),
                    client_nonce: s(&d, "clientNonce"),
                }
            }
            StreamEventName::ConnectionPong => Frame::Pong,
            StreamEventName::ConnectionPreempted => {
                let d = obj(&env.data);
                Frame::Preempted {
                    reason: PreemptReason::from(s(&d, "reason").unwrap_or_default()),
                    new_lease_id: s(&d, "newLeaseId"),
                    timestamp: int(&d, "timestamp"),
                }
            }
            StreamEventName::ConnectionError => {
                let d = obj(&env.data);
                Frame::Error {
                    code: s(&d, "code").map(ConnectionErrorCode::from),
                    message: s(&d, "message"),
                    details: d.get("details").cloned(),
                }
            }
            other => Frame::Other { event: other },
        });
    }
    // v2 form: control frames only.
    let Some(Value::String(action)) = env.action else {
        return Some(Frame::Ignored("frame without event or action"));
    };
    let top: Map<String, Value> = match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    };
    Some(match action.as_str() {
        "pong" => Frame::Pong,
        "admit" => Frame::Admitted {
            lease_id: s(&top, "leaseId"),
            server_now: int(&top, "serverNow"),
            client_nonce: s(&top, "clientNonce"),
        },
        "preempted" => Frame::Preempted {
            reason: PreemptReason::from(s(&top, "reason").unwrap_or_default()),
            new_lease_id: s(&top, "newLeaseId"),
            timestamp: int(&top, "timestamp"),
        },
        "error" => {
            let message = match top.get("error").or_else(|| top.get("data")) {
                Some(Value::String(text)) => Some(text.clone()),
                Some(Value::Object(m)) => {
                    s(m, "message").or_else(|| Some(Value::Object(m.clone()).to_string()))
                }
                Some(other) => Some(other.to_string()),
                None => None,
            };
            Frame::Error {
                code: None,
                message,
                details: None,
            }
        }
        "sendArticle" => Frame::Ignored("v2 article frame (sendArticle) on a v3 stream"),
        _ => Frame::Ignored("unknown v2 action"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_control_frames_map_to_v3() {
        assert!(matches!(
            decode(r#"{"action":"pong","t":1,"serverNow":2}"#),
            Some(Frame::Pong)
        ));
        assert!(matches!(
            decode(
                r#"{"action":"preempted","reason":"connection_limit","newLeaseId":"x","timestamp":5}"#
            ),
            Some(Frame::Preempted {
                reason: PreemptReason::ConnectionLimit,
                new_lease_id: Some(_),
                timestamp: Some(5)
            })
        ));
        assert!(matches!(
            decode(r#"{"action":"error","data":"Message processing failed"}"#),
            Some(Frame::Error { code: None, message: Some(m), .. }) if m == "Message processing failed"
        ));
        assert!(matches!(
            decode(r#"{"action":"admit","leaseId":"l","serverNow":1,"clientNonce":null}"#),
            Some(Frame::Admitted {
                lease_id: Some(_),
                server_now: Some(1),
                client_nonce: None
            })
        ));
        assert!(matches!(
            decode(r#"{"action":"sendArticle","data":{}}"#),
            Some(Frame::Ignored(_))
        ));
        assert!(decode("not json").is_none());
    }

    #[test]
    fn unknown_events_are_other() {
        assert!(matches!(
            decode(r#"{"event":"connection.draining","payloadVersion":3,"data":{"inMs":1}}"#),
            Some(Frame::Other { event }) if event == "connection.draining"
        ));
        assert!(matches!(
            decode(r#"{"event":"article.retracted","data":{}}"#),
            Some(Frame::Other { .. })
        ));
    }
}
