//! JSON-RPC 2.0 envelope types.
//!
//! One request or response per line (see [`crate::framing`]). Batch
//! requests and notifications are not part of this protocol: every
//! request carries an id and receives exactly one response.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The `jsonrpc` field value on every message.
pub const JSONRPC_VERSION: &str = "2.0";

fn jsonrpc_version() -> String {
    JSONRPC_VERSION.to_owned()
}

/// A JSON-RPC request id: number or string, echoed verbatim in the response.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    String(String),
}

impl From<i64> for RequestId {
    fn from(n: i64) -> Self {
        RequestId::Number(n)
    }
}

impl From<&str> for RequestId {
    fn from(s: &str) -> Self {
        RequestId::String(s.to_owned())
    }
}

impl From<String> for RequestId {
    fn from(s: String) -> Self {
        RequestId::String(s)
    }
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestId::Number(n) => write!(f, "{n}"),
            RequestId::String(s) => write!(f, "{s}"),
        }
    }
}

/// A JSON-RPC 2.0 request: `{"jsonrpc":"2.0","id":1,"method":"song.summary","params":{...}}`.
///
/// `params` stays an opaque [`Value`] in the envelope; use
/// [`Request::params`] to parse it into the typed struct for the method
/// (see [`crate::methods`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    #[serde(default = "jsonrpc_version")]
    pub jsonrpc: String,
    pub id: RequestId,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl Request {
    /// Build a request with typed params.
    pub fn new<T: Serialize>(
        id: impl Into<RequestId>,
        method: impl Into<String>,
        params: &T,
    ) -> Result<Self, serde_json::Error> {
        Ok(Self {
            jsonrpc: jsonrpc_version(),
            id: id.into(),
            method: method.into(),
            params: Some(serde_json::to_value(params)?),
        })
    }

    /// Build a request for a method that takes no params.
    pub fn without_params(id: impl Into<RequestId>, method: impl Into<String>) -> Self {
        Self {
            jsonrpc: jsonrpc_version(),
            id: id.into(),
            method: method.into(),
            params: None,
        }
    }

    /// Parse the params into the typed struct for the method.
    ///
    /// Absent params are treated as `null`, so methods without params can
    /// use `()` and structs whose fields are all optional/defaulted also
    /// accept an omitted params object. A mismatch produces an
    /// [`ErrorKind::InvalidParams`] error ready to send back.
    pub fn params<T: DeserializeOwned>(&self) -> Result<T, RpcError> {
        let value = self.params.clone().unwrap_or(Value::Null);
        serde_json::from_value(value).map_err(|e| {
            RpcError::invalid_params(format!("invalid params for {}: {e}", self.method))
        })
    }
}

/// A JSON-RPC 2.0 response: exactly one of `result` / `error` is set.
///
/// `id` is `None` (serialized as `null`) only when the request id could
/// not be determined, e.g. on a parse error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    #[serde(default = "jsonrpc_version")]
    pub jsonrpc: String,
    pub id: Option<RequestId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    /// Build a success response with a typed result.
    pub fn success<T: Serialize>(
        id: impl Into<RequestId>,
        result: &T,
    ) -> Result<Self, serde_json::Error> {
        Ok(Self {
            jsonrpc: jsonrpc_version(),
            id: Some(id.into()),
            result: Some(serde_json::to_value(result)?),
            error: None,
        })
    }

    /// Build an error response. `id` is `None` when the request id is
    /// unknown (parse error).
    pub fn failure(id: Option<RequestId>, error: RpcError) -> Self {
        Self {
            jsonrpc: jsonrpc_version(),
            id,
            result: None,
            error: Some(error),
        }
    }

    /// Extract the typed result, surfacing a server-sent error verbatim.
    pub fn result<T: DeserializeOwned>(&self) -> Result<T, RpcError> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        let value = self.result.clone().unwrap_or(Value::Null);
        serde_json::from_value(value)
            .map_err(|e| RpcError::internal(format!("malformed result payload: {e}")))
    }
}

/// Stable machine-readable error categories (`error.data.kind`).
///
/// Clients branch on this, not on `code` or `message`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// A referenced entity (track, clip, section, chord, job, ...) does not exist.
    NotFound,
    /// Params were missing, of the wrong type, or out of range.
    InvalidParams,
    /// A destructive operation needs `"confirm": true`; `detail` summarizes what would be lost.
    NeedsConfirmation,
    /// The app cannot service the request right now (e.g. conflicting job in flight).
    Busy,
    /// Unknown method, or an operation the app does not support.
    Unsupported,
    /// An unexpected internal failure.
    Internal,
    /// Forward-compat catch-all for kinds introduced by newer peers.
    /// Never sent deliberately.
    #[serde(other)]
    Unknown,
}

impl ErrorKind {
    /// The stable wire string for this kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorKind::NotFound => "not_found",
            ErrorKind::InvalidParams => "invalid_params",
            ErrorKind::NeedsConfirmation => "needs_confirmation",
            ErrorKind::Busy => "busy",
            ErrorKind::Unsupported => "unsupported",
            ErrorKind::Internal => "internal",
            ErrorKind::Unknown => "unknown",
        }
    }

    /// The default JSON-RPC error code for this kind. Standard codes for
    /// the two kinds JSON-RPC covers; application codes (>= 1000) for the
    /// rest.
    pub fn code(&self) -> i64 {
        match self {
            ErrorKind::InvalidParams => codes::INVALID_PARAMS,
            ErrorKind::Internal | ErrorKind::Unknown => codes::INTERNAL_ERROR,
            ErrorKind::NotFound => codes::NOT_FOUND,
            ErrorKind::NeedsConfirmation => codes::NEEDS_CONFIRMATION,
            ErrorKind::Busy => codes::BUSY,
            ErrorKind::Unsupported => codes::UNSUPPORTED,
        }
    }
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// JSON-RPC error codes used by this protocol.
pub mod codes {
    /// Standard JSON-RPC 2.0: invalid JSON was received.
    pub const PARSE_ERROR: i64 = -32700;
    /// Standard JSON-RPC 2.0: the JSON was not a valid request object.
    pub const INVALID_REQUEST: i64 = -32600;
    /// Standard JSON-RPC 2.0: unknown method.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// Standard JSON-RPC 2.0: invalid method params.
    pub const INVALID_PARAMS: i64 = -32602;
    /// Standard JSON-RPC 2.0: internal error.
    pub const INTERNAL_ERROR: i64 = -32603;
    /// Application: referenced entity not found.
    pub const NOT_FOUND: i64 = 1000;
    /// Application: destructive operation requires `"confirm": true`.
    pub const NEEDS_CONFIRMATION: i64 = 1001;
    /// Application: the app cannot service the request right now.
    pub const BUSY: i64 = 1002;
    /// Application: unsupported operation.
    pub const UNSUPPORTED: i64 = 1003;
}

/// Machine-readable error payload carried in `error.data`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorData {
    pub kind: ErrorKind,
    #[serde(default)]
    pub detail: String,
}

/// A JSON-RPC error object: `{"code":1000,"message":"...","data":{"kind":"not_found","detail":"..."}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[error("{message} ({})", self.kind())]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<ErrorData>,
}

impl RpcError {
    /// Build an error of the given kind; the message doubles as the
    /// machine-readable `detail`.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            code: kind.code(),
            message: message.clone(),
            data: Some(ErrorData {
                kind,
                detail: message,
            }),
        }
    }

    /// The error kind, [`ErrorKind::Unknown`] when `data` is absent.
    pub fn kind(&self) -> ErrorKind {
        self.data.as_ref().map_or(ErrorKind::Unknown, |d| d.kind)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, message)
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidParams, message)
    }

    pub fn needs_confirmation(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NeedsConfirmation, message)
    }

    pub fn busy(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Busy, message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unsupported, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Internal, message)
    }

    /// Standard `method not found` error (code -32601, kind `unsupported`).
    pub fn method_not_found(method: &str) -> Self {
        let message = format!("unknown method: {method}");
        Self {
            code: codes::METHOD_NOT_FOUND,
            message: message.clone(),
            data: Some(ErrorData {
                kind: ErrorKind::Unsupported,
                detail: message,
            }),
        }
    }

    /// Standard parse error (code -32700, kind `invalid_params`).
    pub fn parse_error(detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self {
            code: codes::PARSE_ERROR,
            message: format!("parse error: {detail}"),
            data: Some(ErrorData {
                kind: ErrorKind::InvalidParams,
                detail,
            }),
        }
    }
}
