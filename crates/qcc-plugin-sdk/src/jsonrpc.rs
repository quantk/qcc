//! A line-delimited JSON-RPC 2.0 peer, for bridges whose agent speaks it (ACP,
//! `codex app-server`). It only builds and classifies lines; the bridge does the IO.

use std::collections::HashMap;

use serde_json::{Value, json};

pub const METHOD_NOT_FOUND: i64 = -32601;

#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

/// A line received from the agent.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming<T> {
    /// The answer to a request made with [`Peer::request`]; `tag` is what the
    /// request was made for.
    Response {
        tag: T,
        result: Result<Value, RpcError>,
    },
    /// The agent asks something and waits for [`Peer::respond`].
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    /// Not JSON-RPC, or an answer to nothing we asked.
    Unknown(String),
}

/// Our side of the conversation: request ids and what each pending request is for.
#[derive(Debug)]
pub struct Peer<T> {
    next_id: u64,
    pending: HashMap<u64, T>,
}

impl<T> Default for Peer<T> {
    fn default() -> Self {
        Self { next_id: 1, pending: HashMap::new() }
    }
}

impl<T> Peer<T> {
    /// A request line. `tag` comes back with the response.
    pub fn request(&mut self, method: &str, params: Value, tag: T) -> String {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(id, tag);
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
    }

    pub fn notify(&self, method: &str, params: Value) -> String {
        if params.is_null() {
            json!({"jsonrpc": "2.0", "method": method}).to_string()
        } else {
            json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string()
        }
    }

    /// The answer to a request the agent made. `id` is echoed exactly as received.
    pub fn respond(&self, id: &Value, result: Value) -> String {
        json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
    }

    pub fn respond_error(&self, id: &Value, code: i64, message: &str) -> String {
        json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}).to_string()
    }

    /// Requests are told from responses by the presence of `method`, never by the
    /// shape of a result. The two directions have independent id spaces.
    pub fn classify(&mut self, line: &str) -> Incoming<T> {
        let Ok(Value::Object(mut message)) = serde_json::from_str::<Value>(line) else {
            return Incoming::Unknown(line.to_owned());
        };
        if let Some(Value::String(method)) = message.remove("method") {
            let params = message.remove("params").unwrap_or(Value::Null);
            return match message.remove("id") {
                Some(id) if !id.is_null() => Incoming::Request { id, method, params },
                _ => Incoming::Notification { method, params },
            };
        }
        let tag = message.get("id").and_then(Value::as_u64).and_then(|id| self.pending.remove(&id));
        let Some(tag) = tag else {
            return Incoming::Unknown(line.to_owned());
        };
        let result = match message.remove("error") {
            Some(error) if !error.is_null() => Err(RpcError {
                code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                message: error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error")
                    .to_owned(),
            }),
            _ => Ok(message.remove("result").unwrap_or(Value::Null)),
        };
        Incoming::Response { tag, result }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_and_responses_pair_up_by_tag() {
        let mut peer = Peer::<&str>::default();
        let first = peer.request("initialize", json!({"v": 1}), "init");
        let second = peer.request("session/new", json!({}), "new");
        assert_eq!(serde_json::from_str::<Value>(&first).unwrap()["id"], 1);
        assert_eq!(serde_json::from_str::<Value>(&second).unwrap()["method"], "session/new");

        // Out of order, and without the `jsonrpc` field some agents omit.
        assert_eq!(
            peer.classify(r#"{"id":2,"result":{"sessionId":"s"}}"#),
            Incoming::Response { tag: "new", result: Ok(json!({"sessionId": "s"})) }
        );
        assert_eq!(
            peer.classify(
                r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"no auth"}}"#
            ),
            Incoming::Response {
                tag: "init",
                result: Err(RpcError { code: -32000, message: "no auth".into() })
            }
        );
        // Answered once; a duplicate is not ours any more.
        assert!(matches!(peer.classify(r#"{"id":1,"result":{}}"#), Incoming::Unknown(_)));
    }

    #[test]
    fn agent_requests_use_their_own_id_space() {
        let mut peer = Peer::<()>::default();
        let _ = peer.request("a", Value::Null, ());
        // Id 1 with a method is the agent's request, not the answer to ours.
        let Incoming::Request { id, method, params } =
            peer.classify(r#"{"jsonrpc":"2.0","id":1,"method":"fs/read","params":{"p":1}}"#)
        else {
            panic!("expected a request");
        };
        assert_eq!((method.as_str(), &params), ("fs/read", &json!({"p": 1})));
        assert_eq!(
            peer.respond_error(&id, METHOD_NOT_FOUND, "nope"),
            r#"{"error":{"code":-32601,"message":"nope"},"id":1,"jsonrpc":"2.0"}"#
        );
        assert_eq!(
            peer.classify(r#"{"method":"session/update","params":{}}"#),
            Incoming::Notification { method: "session/update".into(), params: json!({}) }
        );
        assert!(matches!(peer.classify("not json"), Incoming::Unknown(_)));
        assert_eq!(
            peer.notify("initialized", Value::Null),
            r#"{"jsonrpc":"2.0","method":"initialized"}"#
        );
    }
}
