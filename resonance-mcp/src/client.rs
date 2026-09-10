//! Blocking JSON-RPC client for the app's unix control socket.
//!
//! One connection, lazily (re)established: the first call — and the first
//! call after a dropped socket — connects and performs the
//! `control.hello` handshake before sending the real request. All socket
//! I/O is synchronous and serialized behind a mutex (the app serializes
//! requests through its update loop anyway); the async [`ControlClient::call`]
//! wrapper runs it on a blocking thread so the MCP server's tokio runtime
//! is never blocked.
//!
//! Every read and write carries a deadline ([`CallTimeouts`]) so an app
//! whose update loop has stalled cannot park a call — and with it the
//! connection mutex, and every later call — forever. A timed-out
//! connection is dropped, never reused: the next call reconnects on a
//! fresh stream, so a late reply to the abandoned request can never be
//! mistaken for the answer to a new one.

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
use std::time::Duration;

/// Env var overriding the control-socket path (same as the app).
pub use resonance_control::socket::SOCKET_PATH_ENV;

/// Resolve the control-socket path exactly like the app does: both
/// sides call the same [`resonance_control::socket`] rule (doc #265),
/// so they agree by construction.
pub use resonance_control::socket::socket_path;

/// Per-call socket deadlines. The defaults suit production; tests
/// shrink them via [`ControlClient::with_timeouts`].
#[derive(Debug, Clone, Copy)]
pub struct CallTimeouts {
    /// Read deadline for ordinary calls. The app answers within one
    /// update-loop turn, so two minutes cleanly separates "slow" from
    /// "stuck".
    pub read: Duration,
    /// Read deadline for `job.wait`, the one method that legitimately
    /// blocks server-side: the app caps a wait at 600 s regardless of
    /// the requested `timeout_ms` (`JobBoard::MAX_WAIT`), so this must
    /// exceed that bound plus margin.
    pub job_wait_read: Duration,
    /// Write deadline. A write only blocks when the app stops draining
    /// its receive buffer, and requests are small.
    pub write: Duration,
}

impl Default for CallTimeouts {
    fn default() -> Self {
        Self {
            read: Duration::from_secs(120),
            job_wait_read: Duration::from_secs(630),
            write: Duration::from_secs(30),
        }
    }
}

/// A failed control call, with enough context to phrase an actionable
/// tool error for the model.
#[derive(Debug)]
pub enum CallError {
    /// Could not connect to the socket: the app is (probably) not running.
    NotRunning { path: PathBuf, source: std::io::Error },
    /// The connection dropped mid-call; the next call reconnects.
    Disconnected { source: std::io::Error },
    /// The app accepted the request but did not answer within the read
    /// deadline: it is running but its update loop is stalled. The
    /// connection is dropped — a late reply on a reused stream would
    /// desync request/response pairing — and the next call reconnects
    /// on a fresh stream.
    Unresponsive { method: String, timeout: Duration },
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
            CallError::Unresponsive { method, timeout } => format!(
                "resonance did not answer {method} within {}s: the app is running but \
                 unresponsive (its update loop may be stalled). The connection was dropped and \
                 the next tool call reconnects on a fresh stream — check the app, then verify \
                 state with song_summary before retrying any edit.",
                timeout.as_secs()
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
    timeouts: CallTimeouts,
}

impl ControlClient {
    /// A client for the socket at `path`; does not connect yet.
    pub fn new(path: PathBuf) -> Arc<Self> {
        Self::with_timeouts(path, CallTimeouts::default())
    }

    /// [`Self::new`] with explicit socket deadlines (tests shrink them
    /// to keep an unresponsive-app scenario fast).
    pub fn with_timeouts(path: PathBuf, timeouts: CallTimeouts) -> Arc<Self> {
        Arc::new(Self {
            path,
            connection: Mutex::new(None),
            next_id: AtomicI64::new(1),
            timeouts,
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
        match self.round_trip(connection, self.fresh_id(), method, params) {
            Ok(value) => Ok(value),
            Err(error) => {
                // Drop the connection on transport-level failures —
                // including timeouts, whose late reply on a reused
                // stream would desync request/response pairing — so
                // the next call reconnects; RPC errors keep it alive.
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
        // Timeouts are socket options, shared with every `try_clone` of
        // the stream — setting them here covers the reader half too.
        stream
            .set_write_timeout(Some(self.timeouts.write))
            .and_then(|()| stream.set_read_timeout(Some(self.timeouts.read)))
            .map_err(|source| CallError::Disconnected { source })?;
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
        let value = self
            .round_trip(
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
    ///
    /// Both directions carry a deadline: hitting it maps to
    /// [`CallError::Unresponsive`], which [`Self::call_blocking`]
    /// treats like any transport failure — the connection is dropped
    /// and never reused.
    fn round_trip(
        &self,
        connection: &mut Connection,
        id: i64,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, CallError> {
        // `job.wait` legitimately blocks server-side (bounded at 600 s
        // by the app), so it gets the long deadline; everything else
        // answers within one update-loop turn.
        let read_timeout = if method == resonance_control::job::WAIT {
            self.timeouts.job_wait_read
        } else {
            self.timeouts.read
        };
        // The writer and the reader clone share one socket, so this
        // reaches the reader half too.
        connection
            .writer
            .set_read_timeout(Some(read_timeout))
            .map_err(|source| CallError::Disconnected { source })?;
        let unresponsive = |timeout: Duration| CallError::Unresponsive {
            method: method.to_owned(),
            timeout,
        };
        let is_timeout = |io: &std::io::Error| {
            matches!(
                io.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            )
        };
        let request = Request {
            jsonrpc: resonance_control::rpc::JSONRPC_VERSION.to_owned(),
            id: RequestId::Number(id),
            method: method.to_owned(),
            params,
        };
        write_message(&mut connection.writer, &request).map_err(|e| match e {
            resonance_control::FramingError::Io(io) if is_timeout(&io) => {
                unresponsive(self.timeouts.write)
            }
            resonance_control::FramingError::Io(io) => CallError::Disconnected { source: io },
            other => CallError::Disconnected {
                source: std::io::Error::other(other.to_string()),
            },
        })?;
        loop {
            let response: Response = connection
                .reader
                .read_message()
                .map_err(|e| match e {
                    resonance_control::FramingError::Io(io) if is_timeout(&io) => {
                        unresponsive(read_timeout)
                    }
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
