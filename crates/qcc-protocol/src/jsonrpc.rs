//! JSON-RPC 2.0 envelopes. Requests and responses are told apart by the presence of
//! `method`, never by the shape of a result.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const VERSION: &str = "2.0";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    /// Absent on notifications.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

impl Request {
    pub fn new(id: u64, method: &str, params: Value) -> Self {
        Self { jsonrpc: VERSION.into(), id: Some(id.into()), method: method.into(), params }
    }

    pub fn notification(method: &str, params: Value) -> Self {
        Self { jsonrpc: VERSION.into(), id: None, method: method.into(), params }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: Value, result: Value) -> Self {
        Self { jsonrpc: VERSION.into(), id, result: Some(result), error: None }
    }

    pub fn err(id: Value, error: RpcError) -> Self {
        Self { jsonrpc: VERSION.into(), id, result: None, error: Some(error) }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), data: None }
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (code {})", self.message, self.code)
    }
}

impl std::error::Error for RpcError {}

/// Error codes. The standard JSON-RPC ones plus the daemon's own.
pub mod codes {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;

    pub const THREAD_NOT_FOUND: i32 = -32001;
    pub const PROVIDER_NOT_FOUND: i32 = -32002;
    pub const INTERACTION_NOT_PENDING: i32 = -32003;
    /// The provider plugin refused or failed the request.
    pub const PROVIDER_ERROR: i32 = -32004;
    /// The thread is in the middle of a turn or of starting its session.
    pub const THREAD_BUSY: i32 = -32005;
}

/// Either kind of message a peer can receive.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Request(Request),
    Response(Response),
}

impl Incoming {
    pub fn parse(line: &str) -> Result<Self, serde_json::Error> {
        let value: Value = serde_json::from_str(line)?;
        if value.get("method").is_some() {
            serde_json::from_value(value).map(Self::Request)
        } else {
            serde_json::from_value(value).map(Self::Response)
        }
    }
}
