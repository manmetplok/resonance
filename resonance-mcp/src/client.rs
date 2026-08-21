//! Blocking JSON-RPC client for the app's unix control socket.
//!
//! One connection, lazily (re)established: the first call — and the first
//! call after a dropped socket — connects and performs the
//! `control.hello` handshake before sending the real request. All socket
//! I/O is synchronous and serialized behind a mutex (the app serializes
//! requests through its update loop anyway); the async [`ControlClient::call`]
//! wrapper runs it on a blocking thread so the MCP server's tokio runtime
//! is never blocked.

use resonance_control::methods::control::{HelloParams, HelloResult};
use resonance_control::{
    write_message, ErrorKind, MessageReader, Request, RequestId, Response, RpcError,
    PROTOCOL_VERSION,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::io::BufReader;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

/// Env var overriding the control-socket path (same as the app).
pub use resonance_control::socket::SOCKET_PATH_ENV;

/// Resolve the control-socket path exactly like the app does: both
/// sides call the same [`resonance_control::socket`] rule (doc #265),
/// so they agree by construction.
pub use resonance_control::socket::socket_path;

/// A failed control call, with enough context to phrase an actionable
/// tool error for the model.
#[derive(Debug)]
pub enum CallError {
    /// Could not connect to the socket: the app is (probably) not running.
    NotRunning { path: PathBuf, source: std::io::Error },
    /// The connection dropped mid-call; the next call reconnects.
    Disconnected { source: std::io::Error },
    /// The `control.hello` handshake was rejected or version-incompatible.
    Handshake { detail: String },
    /// The server answered with a JSON-RPC error.
    Rpc(RpcError),
    /// The server answered with something malformed.
    Protocol { detail: String },
}

impl CallError {
    /// Human/model-facing message: states what went wrong and what to do
    /// next (ba doc #266 §3 — execution errors should enable
    /// self-correction).
    pub fn actionable_message(&self) -> String {
        match self {
            CallError::NotRunning { path, source } => format!(
                "resonance is not running: could not connect to the control socket at {} ({source}). \
                 Start the resonance app (with control enabled, i.e. without RESONANCE_NO_CONTROL=1) \
                 and retry this tool call.",
                path.display()
            ),
            CallError::Disconnected { source } => format!(
                "the connection to resonance dropped mid-call ({source}). The app may have quit or \
                 restarted; the next tool call reconnects automatically — verify state with \
                 song_summary before retrying any edit."
            ),
            CallError::Handshake { detail } => format!(
                "the running resonance app is protocol-incompatible with this MCP server: {detail}. \
                 Update the resonance app and/or the resonance-mcp binary so both come from the \
                 same build."
            ),
            CallError::Rpc(error) => {
                let hint = match error.kind() {
                    ErrorKind::NotFound => {
                        " Call the matching song_* view tool (song_summary, song_tracks, \
                         song_sections, song_notes, song_vocal) to list currently valid ids."
                    }
                    ErrorKind::NeedsConfirmation => {
                        " If the described loss is intended, retry with \"confirm\": true \
                         (\"overwrite\": true for render targets)."
                    }
                    ErrorKind::Busy => {
                        " The app is busy with a conflicting operation; wait for the running job \
                         (job_status) and retry."
                    }
                    ErrorKind::Unsupported => {
                        " The running app version does not support this operation."
                    }
                    _ => "",
                };
                format!("{}{hint}", error.message)
            }
            CallError::Protocol { detail } => {
                format!("malformed response from the resonance app: {detail}")
            }
        }
    }
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.actionable_message())
    }
}

impl std::error::Error for CallError {}

/// One live connection: framed reader + raw writer + the server's hello.
struct Connection {
    reader: MessageReader<BufReader<UnixStream>>,
    writer: UnixStream,
    hello: HelloResult,
}

/// Lazily-connecting control-socket client. Cheap to clone via [`Arc`].
pub struct ControlClient {
    path: PathBuf,
    connection: Mutex<Option<Connection>>,
    next_id: AtomicI64,
}

impl ControlClient {
    /// A client for the socket at `path`; does not connect yet.
    pub fn new(path: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            path,
            connection: Mutex::new(None),
            next_id: AtomicI64::new(1),
        })
    }

    /// The socket path this client targets.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Async wrapper around [`Self::call_blocking`]: runs the blocking
    /// socket round-trip on a tokio blocking thread.
    pub async fn call<P: Serialize>(
        self: &Arc<Self>,
        method: &'static str,
        params: &P,
    ) -> Result<Value, CallError> {
        let params = serde_json::to_value(params).map_err(|e| CallError::Protocol {
            detail: format!("failed to encode params for {method}: {e}"),
        })?;
        let client = Arc::clone(self);
        tokio::task::spawn_blocking(move || client.call_blocking(method, Some(params)))
            .await
            .unwrap_or_else(|e| {
                Err(CallError::Protocol {
                    detail: format!("internal task failure: {e}"),
                })
            })
    }

    /// [`Self::call`] with the result deserialized into `T`.
    pub async fn call_typed<P: Serialize, T: DeserializeOwned>(
        self: &Arc<Self>,
        method: &'static str,
        params: &P,
    ) -> Result<T, CallError> {
        let value = self.call(method, params).await?;
        serde_json::from_value(value).map_err(|e| CallError::Protocol {
            detail: format!("unexpected result shape for {method}: {e}"),
        })
    }

    /// One blocking request/response round-trip, (re)connecting and
    /// handshaking first when needed.
    pub fn call_blocking(&self, method: &str, params: Option<Value>) -> Result<Value, CallError> {
        let mut slot = self.connection.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            *slot = Some(self.connect()?);
        }
        let connection = slot.as_mut().expect("connection just established");
        match Self::round_trip(connection, self.fresh_id(), method, params) {
            Ok(value) => Ok(value),
            Err(error) => {
                // Drop the connection on transport-level failures so the
                // next call reconnects; RPC errors keep it alive.
                if !matches!(error, CallError::Rpc(_)) {
                    *slot = None;
                }
                Err(error)
            }
        }
    }

    /// The server's `control.hello` result, connecting if needed.
    pub fn hello_blocking(&self) -> Result<HelloResult, CallError> {
        let mut slot = self.connection.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            *slot = Some(self.connect()?);
        }
        Ok(slot.as_ref().expect("connection just established").hello.clone())
    }

    fn fresh_id(&self) -> i64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Connect and perform the `control.hello` handshake.
    fn connect(&self) -> Result<Connection, CallError> {
        let stream = UnixStream::connect(&self.path).map_err(|source| CallError::NotRunning {
            path: self.path.clone(),
            source,
        })?;
        let reader_stream = stream.try_clone().map_err(|source| CallError::Disconnected { source })?;
        let mut connection = Connection {
            reader: MessageReader::from_reader(reader_stream),
            writer: stream,
            hello: HelloResult {
                app_version: String::new(),
                protocol_version: 0,
                capabilities: Vec::new(),
            },
        };
        let params = HelloParams {
            protocol_version: PROTOCOL_VERSION,
        };
        let hello_params = serde_json::to_value(params).expect("hello params serialize");
        let value = Self::round_trip(
            &mut connection,
            self.fresh_id(),
            resonance_control::methods::control::HELLO,
            Some(hello_params),
        )
        .map_err(|error| match error {
            // A rejected handshake is a compatibility problem, not a
            // generic RPC failure.
            CallError::Rpc(e) => CallError::Handshake { detail: e.message },
            other => other,
        })?;
        let hello: HelloResult = serde_json::from_value(value).map_err(|e| CallError::Protocol {
            detail: format!("unexpected control.hello result: {e}"),
        })?;
        if hello.protocol_version != PROTOCOL_VERSION {
            return Err(CallError::Handshake {
                detail: format!(
                    "app speaks control protocol v{}, this MCP server speaks v{PROTOCOL_VERSION}",
                    hello.protocol_version
                ),
            });
        }
        tracing::info!(
            app_version = %hello.app_version,
            capabilities = hello.capabilities.len(),
            "connected to resonance at {}",
            self.path.display()
        );
        connection.hello = hello.clone();
        Ok(connection)
    }

    /// Write one request and read responses until the matching id.
    fn round_trip(
        connection: &mut Connection,
        id: i64,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, CallError> {
        let request = Request {
            jsonrpc: resonance_control::rpc::JSONRPC_VERSION.to_owned(),
            id: RequestId::Number(id),
            method: method.to_owned(),
            params,
        };
        write_message(&mut connection.writer, &request).map_err(|e| CallError::Disconnected {
            source: match e {
                resonance_control::FramingError::Io(io) => io,
                other => std::io::Error::other(other.to_string()),
            },
        })?;
        loop {
            let response: Response = connection
                .reader
                .read_message()
                .map_err(|e| match e {
                    resonance_control::FramingError::Io(io) => {
                        CallError::Disconnected { source: io }
                    }
                    other => CallError::Protocol {
                        detail: other.to_string(),
                    },
                })?
                .ok_or_else(|| CallError::Disconnected {
                    source: std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "server closed the connection",
                    ),
                })?;
            // Requests are strictly serialized (one in flight per client),
            // so anything with a different id is a stray reply from a
            // previous, abandoned call — skip it.
            if response.id != Some(RequestId::Number(id)) {
                continue;
            }
            return match response.result::<Value>() {
                Ok(value) => Ok(value),
                Err(rpc) => Err(CallError::Rpc(rpc)),
            };
        }
    }
}

impl std::fmt::Debug for ControlClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlClient")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}
