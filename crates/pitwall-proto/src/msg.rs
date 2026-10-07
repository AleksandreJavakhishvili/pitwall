//! JSON messages (architecture.md §4): the handshake, then requests,
//! responses and events.
//!
//! ```json
//! → {"hello":{"protocol":{"min":1,"max":1},"client":"pitwall-cli/0.1.0","role":"cli"}}
//! ← {"welcome":{"protocol":1,"daemon":"0.1.0","caps":["agents","sessions","approvals"],"instance":"…"}}
//! ← {"reject":{"code":"incompatible","daemon":"0.3.0","protocol":{"min":2,"max":2}}}
//! → {"id":7,"method":"agent.list","params":{}}
//! ← {"id":7,"result":[…]}
//! ← {"id":8,"error":{"code":"not_found","message":"…"}}
//! ← {"event":"approvals.changed","data":[…]}
//! ```
//!
//! Within a protocol version changes are additive only: new fields are
//! optional and both sides ignore fields they don't know. Features some
//! servers lack are gated by `welcome.caps`, never by version comparisons.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::PROTOCOL;

/// A range of protocol versions a side speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Range {
    pub min: u32,
    pub max: u32,
}

impl Range {
    /// What this build speaks.
    pub const fn ours() -> Range {
        Range { min: PROTOCOL, max: PROTOCOL }
    }

    /// The highest version both sides speak.
    pub fn negotiate(self, other: Range) -> Option<u32> {
        let v = self.max.min(other.max);
        (v >= self.min.max(other.min)).then_some(v)
    }
}

/// Who is connecting. Only a verified UI client may answer approvals; a
/// claimed role is never trusted for that (the server checks the peer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Ui,
    Cli,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Hello {
    pub protocol: Range,
    /// "pitwall-cli/0.1.0"
    pub client: String,
    pub role: Role,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClientHello {
    Hello(Hello),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Welcome {
    pub protocol: u32,
    /// The server's version ("0.1.0").
    pub daemon: String,
    /// Feature flags ("agents", "sessions", "approvals", …).
    pub caps: Vec<String>,
    /// Changes when the server restarts.
    pub instance: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Reject {
    /// `incompatible`, or `bad_hello`.
    pub code: String,
    pub daemon: String,
    pub protocol: Range,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerHello {
    Welcome(Welcome),
    Reject(Reject),
}

/// Answer a client's hello: welcome when a version is shared.
pub fn answer_hello(hello: &Hello, server: Range, daemon: &str, caps: &[&str], instance: &str) -> ServerHello {
    match server.negotiate(hello.protocol) {
        Some(protocol) => ServerHello::Welcome(Welcome {
            protocol,
            daemon: daemon.into(),
            caps: caps.iter().map(|c| c.to_string()).collect(),
            instance: instance.into(),
        }),
        None => ServerHello::Reject(Reject { code: code::INCOMPATIBLE.into(), daemon: daemon.into(), protocol: server }),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ErrorBody {
    /// One of [`code`]; clients react to it without parsing `message`.
    pub code: String,
    pub message: String,
}

impl ErrorBody {
    pub fn new(code: &str, message: impl Into<String>) -> ErrorBody {
        ErrorBody { code: code.into(), message: message.into() }
    }
}

impl std::fmt::Display for ErrorBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

/// Error codes.
pub mod code {
    pub const INCOMPATIBLE: &str = "incompatible";
    pub const BAD_HELLO: &str = "bad_hello";
    pub const UNKNOWN_METHOD: &str = "unknown_method";
    pub const BAD_PARAMS: &str = "bad_params";
    pub const NOT_FOUND: &str = "not_found";
    pub const UNSUPPORTED: &str = "unsupported";
    pub const NOT_RUNNING: &str = "not_running";
    pub const UNREACHABLE: &str = "unreachable";
    pub const CONFLICT: &str = "conflict";
    /// The user said no in Pitwall's approval dialog, or the caller may not
    /// do this at all.
    pub const DENIED: &str = "denied";
    /// Nobody answered the approval dialog in time (a denial).
    pub const APPROVAL_TIMEOUT: &str = "approval_timeout";
    pub const OTHER: &str = "other";
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

impl Response {
    pub fn ok(id: u64, result: Value) -> Response {
        Response { id, result: Some(result), error: None }
    }

    pub fn err(id: u64, error: ErrorBody) -> Response {
        Response { id, result: None, error: Some(error) }
    }

    /// The result, or the error (a response with neither is `null`).
    pub fn into_result(self) -> Result<Value, ErrorBody> {
        match self.error {
            Some(e) => Err(e),
            None => Ok(self.result.unwrap_or(Value::Null)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub event: String,
    #[serde(default)]
    pub data: Value,
}

/// Anything the server sends after the handshake.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ServerMsg {
    Response(Response),
    Event(Event),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn round_trip<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(v: &T, wire: Value) {
        assert_eq!(serde_json::to_value(v).unwrap(), wire);
        assert_eq!(&serde_json::from_value::<T>(wire).unwrap(), v);
    }

    #[test]
    fn handshake_golden_json() {
        let hello = ClientHello::Hello(Hello { protocol: Range { min: 1, max: 1 }, client: "pitwall-cli/0.1.0".into(), role: Role::Cli });
        round_trip(&hello, json!({"hello":{"protocol":{"min":1,"max":1},"client":"pitwall-cli/0.1.0","role":"cli"}}));
        let ServerHello::Welcome(w) = answer_hello(
            &Hello { protocol: Range { min: 1, max: 3 }, client: "x".into(), role: Role::Ui },
            Range { min: 1, max: 1 },
            "0.1.0",
            &["agents"],
            "i-1",
        ) else {
            panic!("shared version 1")
        };
        round_trip(
            &ServerHello::Welcome(w),
            json!({"welcome":{"protocol":1,"daemon":"0.1.0","caps":["agents"],"instance":"i-1"}}),
        );
        let reject = answer_hello(
            &Hello { protocol: Range { min: 2, max: 2 }, client: "x".into(), role: Role::Cli },
            Range { min: 1, max: 1 },
            "0.1.0",
            &[],
            "i",
        );
        round_trip(&reject, json!({"reject":{"code":"incompatible","daemon":"0.1.0","protocol":{"min":1,"max":1}}}));
    }

    #[test]
    fn versions_negotiate_to_the_highest_shared() {
        let r = |min, max| Range { min, max };
        assert_eq!(r(1, 3).negotiate(r(2, 5)), Some(3));
        assert_eq!(r(1, 1).negotiate(r(1, 1)), Some(1));
        assert_eq!(r(1, 1).negotiate(r(2, 2)), None);
        assert_eq!(r(3, 4).negotiate(r(1, 2)), None);
    }

    #[test]
    fn requests_responses_and_events_golden_json() {
        round_trip(&Request { id: 7, method: "agent.list".into(), params: Value::Null }, json!({"id":7,"method":"agent.list"}));
        round_trip(
            &Request { id: 8, method: "session.add".into(), params: json!({"provider":"agw"}) },
            json!({"id":8,"method":"session.add","params":{"provider":"agw"}}),
        );
        let ok = ServerMsg::Response(Response::ok(7, json!([])));
        round_trip(&ok, json!({"id":7,"result":[]}));
        let err = ServerMsg::Response(Response::err(8, ErrorBody::new(code::DENIED, "no")));
        round_trip(&err, json!({"id":8,"error":{"code":"denied","message":"no"}}));
        let ev = ServerMsg::Event(Event { event: "approvals.changed".into(), data: json!([]) });
        round_trip(&ev, json!({"event":"approvals.changed","data":[]}));
        // Unknown fields are ignored (additive changes).
        let later: ServerMsg = serde_json::from_value(json!({"id":1,"result":2,"trace":"x"})).unwrap();
        assert_eq!(later, ServerMsg::Response(Response::ok(1, json!(2))));
        assert_eq!(Response::ok(1, json!(2)).into_result(), Ok(json!(2)));
        assert_eq!(Response::err(1, ErrorBody::new("x", "y")).into_result().unwrap_err().code, "x");
    }
}
