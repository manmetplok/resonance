//! Unix-socket control endpoint — socket side (ba doc #265, todo #1147).
//!
//! A background listener thread accepts connections on a per-user unix
//! socket and forwards each parsed JSON-RPC request into the iced update
//! loop as a [`ControlMessage`]; the `update/control` handler executes it
//! against `Resonance` on the main thread and replies over the per-request
//! [`ReplySender`] back to that connection's writer thread. This module
//! owns everything that runs *off* the update loop:
//!
//! - socket path resolution + lifecycle ([`socket_path`], [`spawn`],
//!   [`ControlServer`]),
//! - the accept loop and per-connection reader/writer threads,
//! - the bridge into iced (a `futures` unbounded channel whose receiver is
//!   handed to [`iced::Subscription::run`] via [`bridge_stream`]).
//!
//! The update loop is never blocked on socket I/O: reads happen on the
//! per-connection reader threads, writes on the writer threads, and both
//! sides of the bridge are unbounded non-blocking channels. Requests are
//! serialized in arrival order through the single bridge channel, so
//! handlers never race the GUI.
//!
//! Protocol framing and envelope types come from the `resonance-control`
//! crate — the single source of truth both sides compile against.

use crate::control_jobs::JobBoard;
use iced::futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use iced::futures::stream::BoxStream;
use resonance_control::{FramingError, MessageReader, Request, Response, RpcError};
use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Identifies one accepted client connection for the lifetime of the app.
pub type ConnId = u64;

/// Value of the env var that disables the control endpoint entirely.
pub const NO_CONTROL_ENV: &str = "RESONANCE_NO_CONTROL";
/// Env var overriding the socket path.
pub use resonance_control::socket::SOCKET_PATH_ENV;

// ---------------------------------------------------------------------------
// Bridge message types (socket threads -> update loop)
// ---------------------------------------------------------------------------

/// One request forwarded from a socket reader thread into the update loop.
///
/// Carries everything the `update/control` handler needs: which
/// connection it came from (for per-connection handshake state), the
/// parsed JSON-RPC request, and the reply channel back to that
/// connection's writer thread.
#[derive(Debug, Clone)]
pub struct ControlRequest {
    pub conn: ConnId,
    pub request: Request,
    pub reply: ReplySender,
}

/// The `Message::Control` payload produced by the socket threads.
///
/// Connect/disconnect events flow through the same ordered channel as
/// requests so the update loop's `control_clients` bookkeeping can never
/// observe a request from a connection it hasn't seen yet.
#[derive(Debug, Clone)]
pub enum ControlMessage {
    /// A client connected.
    Connected { conn: ConnId },
    /// A client disconnected (EOF or read error).
    Disconnected { conn: ConnId },
    /// A parsed request awaiting execution on the update loop.
    Request(ControlRequest),
}

impl ControlMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::UndoAction;
        match self {
            // Connect / disconnect events and request execution carry no undo
            // weight at this level (doc #265, todo #1147). Mutating control
            // methods synthesize ordinary domain messages that re-enter `update()`
            // individually and are classified there.
            Self::Connected { .. } | Self::Disconnected { .. } | Self::Request(..) => {
                UndoAction::Skip
            }
        }
    }
}

/// Non-blocking reply channel to one connection's writer thread.
///
/// Cloneable so the update loop can hold it across an async job if a
/// later todo needs to defer a reply. Sending never blocks (unbounded
/// channel) and errors (writer gone after a disconnect) are deliberately
/// swallowed — a reply to a vanished client is a no-op, not a failure.
#[derive(Clone)]
pub struct ReplySender(crossbeam_channel::Sender<Response>);

impl ReplySender {
    /// Send a response to the client. Ignores a closed channel: the
    /// client already disconnected and nobody is listening.
    pub fn send(&self, response: Response) {
        let _ = self.0.send(response);
    }

    /// A connected (sender, receiver) pair, for tests that act as the
    /// writer thread and assert on the replies the handler produced.
    pub fn test_pair() -> (Self, crossbeam_channel::Receiver<Response>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        (Self(tx), rx)
    }
}

impl std::fmt::Debug for ReplySender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReplySender")
    }
}

// ---------------------------------------------------------------------------
// Socket path resolution
// ---------------------------------------------------------------------------

/// True when `RESONANCE_NO_CONTROL=1` disables the control endpoint.
pub fn control_disabled() -> bool {
    std::env::var(NO_CONTROL_ENV).as_deref() == Ok("1")
}

/// Resolve the control-socket path. The rule lives in
/// [`resonance_control::socket`] so this server side and the
/// `resonance-mcp` client side can never disagree on it (doc #265).
pub use resonance_control::socket::socket_path;

/// Create the socket's parent directory private (`0700`), or refuse an
/// existing one that is a symlink, someone else's, or open to group /
/// other (code review CTL-11 / UPD-12). Never `chmod`s anything — see
/// [`resonance_control::socket::prepare_socket_dir`].
fn prepare_parent_dir(path: &Path) -> io::Result<()> {
    match path.parent().filter(|d| !d.as_os_str().is_empty()) {
        Some(dir) => resonance_control::socket::prepare_socket_dir(dir),
        None => Ok(()),
    }
}

/// The instance lock guarding the socket path: `<socket>.lock`, held
/// with a `flock`-style exclusive lock for the server's lifetime.
fn lock_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}

/// Take the single-instance lock for `path`: open (creating) the
/// adjacent `<socket>.lock` file and `try_lock` it exclusively. The
/// winner holds the lock (the open `File`) until drop; a loser gets
/// `AddrInUse` immediately — another live instance owns the socket.
///
/// This is what makes stale-socket replacement race-free: only the
/// lock holder may unlink or rebind the socket path, so two instances
/// starting concurrently can no longer both probe a stale file and
/// have the loser's `remove_file` unlink the winner's freshly bound
/// socket. The lock file itself is never removed — unlinking it would
/// let a racer holding the old inode and a fresh starter creating a
/// new one both "win" at the same path.
fn acquire_instance_lock(path: &Path) -> io::Result<std::fs::File> {
    let lock = std::fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .open(lock_path(path))?;
    match lock.try_lock() {
        Ok(()) => Ok(lock),
        Err(std::fs::TryLockError::WouldBlock) => Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!(
                "another resonance instance is serving {}",
                path.display()
            ),
        )),
        Err(std::fs::TryLockError::Error(e)) => Err(e),
    }
}

/// Bind the listener, replacing a stale socket file left by a crashed
/// instance. Caller holds the instance lock, so anything sitting at
/// the path is stale by definition — a live instance would have kept
/// the lock. The connect probe stays as a belt-and-braces guard
/// against a server that is accepting without holding the lock (e.g.
/// a pre-lock build): if something answers, bail out rather than
/// yank a live socket.
fn bind_or_replace_stale(path: &Path) -> io::Result<UnixListener> {
    match UnixListener::bind(path) {
        Ok(listener) => Ok(listener),
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
            if UnixStream::connect(path).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!(
                        "another resonance instance is serving {}",
                        path.display()
                    ),
                ));
            }
            std::fs::remove_file(path)?;
            UnixListener::bind(path)
        }
        Err(e) => Err(e),
    }
}

/// Filesystem identity (`st_dev`, `st_ino`) of the socket file a
/// freshly bound listener created, recorded so [`ControlServer::drop`]
/// can prove the path still points at *its* socket before unlinking.
fn socket_file_identity(path: &Path) -> io::Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(path)?;
    Ok((meta.dev(), meta.ino()))
}

// ---------------------------------------------------------------------------
// Server lifecycle
// ---------------------------------------------------------------------------

/// Handle to the running listener, held in app state. Dropping it (clean
/// shutdown) wakes the accept loop, stops it, and removes the socket
/// file.
pub struct ControlServer {
    path: PathBuf,
    shutdown: Arc<AtomicBool>,
    /// `(st_dev, st_ino)` of the socket file this instance bound; the
    /// unlink in `drop` is gated on the path still matching it.
    bound_identity: (u64, u64),
    /// Held single-instance lock (see [`acquire_instance_lock`]).
    /// Dropped last (declaration order), so the lock outlives the
    /// socket-file removal above it.
    _instance_lock: std::fs::File,
}

impl ControlServer {
    /// The bound socket path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl std::fmt::Debug for ControlServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlServer")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // Wake the accept loop so it observes the flag and exits; the
        // connection is dropped immediately on the other side.
        let _ = UnixStream::connect(&self.path);
        // Remove the socket only if the path still points at the file
        // this instance bound. If the lock file was purged externally
        // (tmpfiles-style cleanup) a newer instance may have rebound
        // the path legitimately — a blind unlink here would silently
        // unpublish that live server.
        if socket_file_identity(&self.path).is_ok_and(|id| id == self.bound_identity) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Bind `path` and start the accept loop, forwarding connection events
/// and parsed requests into `bridge` (the update loop's subscription
/// channel). `jobs` is the shared job ledger the reader threads use to
/// serve `job.wait` without touching the update loop (todo #1149).
/// Returns the lifecycle handle to keep in app state.
pub fn spawn(
    path: PathBuf,
    bridge: UnboundedSender<ControlMessage>,
    jobs: Arc<JobBoard>,
) -> io::Result<ControlServer> {
    prepare_parent_dir(&path)?;
    let instance_lock = acquire_instance_lock(&path)?;
    let listener = bind_or_replace_stale(&path)?;
    let bound_identity = socket_file_identity(&path)?;
    let shutdown = Arc::new(AtomicBool::new(false));
    let accept_shutdown = Arc::clone(&shutdown);
    std::thread::Builder::new()
        .name("control-accept".into())
        .spawn(move || accept_loop(listener, bridge, jobs, accept_shutdown))?;
    Ok(ControlServer {
        path,
        shutdown,
        bound_identity,
        _instance_lock: instance_lock,
    })
}

fn accept_loop(
    listener: UnixListener,
    bridge: UnboundedSender<ControlMessage>,
    jobs: Arc<JobBoard>,
    shutdown: Arc<AtomicBool>,
) {
    let mut next_conn: ConnId = 0;
    for stream in listener.incoming() {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = stream else {
            // Transient accept failure; keep serving.
            continue;
        };
        next_conn += 1;
        let conn = next_conn;
        if bridge
            .unbounded_send(ControlMessage::Connected { conn })
            .is_err()
        {
            // Update loop is gone — the app is shutting down.
            break;
        }
        let conn_bridge = bridge.clone();
        let conn_jobs = Arc::clone(&jobs);
        let spawned = std::thread::Builder::new()
            .name(format!("control-conn-{conn}"))
            .spawn(move || serve_connection(stream, conn, conn_bridge, conn_jobs));
        if spawned.is_err() {
            let _ = bridge.unbounded_send(ControlMessage::Disconnected { conn });
        }
    }
}

/// Per-connection reader loop (runs on its own thread) plus a paired
/// writer thread. The reader parses requests and forwards them over the
/// bridge; the writer serializes every [`Response`] sent through the
/// connection's [`ReplySender`] back onto the socket.
fn serve_connection(
    stream: UnixStream,
    conn: ConnId,
    bridge: UnboundedSender<ControlMessage>,
    jobs: Arc<JobBoard>,
) {
    let (reply_tx, reply_rx) = crossbeam_channel::unbounded::<Response>();

    let writer_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => {
            let _ = bridge.unbounded_send(ControlMessage::Disconnected { conn });
            return;
        }
    };
    let writer = std::thread::Builder::new()
        .name(format!("control-conn-{conn}-w"))
        .spawn(move || {
            let mut writer_stream = writer_stream;
            // Exits when every ReplySender clone is dropped (reader done
            // and all in-flight requests replied) or the client goes away.
            for response in reply_rx {
                if resonance_control::write_message(&mut writer_stream, &response).is_err() {
                    break;
                }
            }
            let _ = writer_stream.shutdown(std::net::Shutdown::Both);
        });
    if writer.is_err() {
        let _ = bridge.unbounded_send(ControlMessage::Disconnected { conn });
        return;
    }

    let mut reader = MessageReader::from_reader(stream);
    loop {
        match reader.read_message::<Request>() {
            Ok(Some(request)) => {
                // `job.wait` blocks — serve it here on the reader
                // thread against the shared job board, never through
                // the update loop (doc #265). Subsequent requests on
                // this connection queue behind the wait, matching the
                // one-outstanding-request shape of a blocking call;
                // other connections are unaffected.
                if request.method == resonance_control::job::WAIT {
                    let _ = reply_tx.send(job_wait_response(&jobs, &request));
                    continue;
                }
                let message = ControlMessage::Request(ControlRequest {
                    conn,
                    request,
                    reply: ReplySender(reply_tx.clone()),
                });
                if bridge.unbounded_send(message).is_err() {
                    break;
                }
            }
            // Clean EOF: client hung up.
            Ok(None) => break,
            // Bad line: reply with a parse error (id unknown -> null) and
            // keep reading — the reader is positioned at the next line.
            Err(FramingError::Invalid { source, .. }) => {
                let _ = reply_tx.send(Response::failure(
                    None,
                    RpcError::parse_error(source.to_string()),
                ));
            }
            Err(_) => break,
        }
    }
    drop(reply_tx);
    let _ = bridge.unbounded_send(ControlMessage::Disconnected { conn });
}

/// Serve one `job.wait` on the reader thread: block on the job board
/// until the job is terminal or the timeout elapses, replying with the
/// then-current status either way (`not_found` for an unknown or
/// dropped job).
fn job_wait_response(jobs: &JobBoard, request: &Request) -> Response {
    use resonance_control::job::WaitParams;
    let params: WaitParams = match request.params() {
        Ok(p) => p,
        Err(e) => return Response::failure(Some(request.id.clone()), e),
    };
    let timeout = params
        .timeout_ms
        .map(std::time::Duration::from_millis);
    match jobs.wait(params.job_id.0, timeout) {
        Some(status) => Response::success(request.id.clone(), &status).unwrap_or_else(|e| {
            Response::failure(
                Some(request.id.clone()),
                RpcError::internal(format!("failed to encode job status: {e}")),
            )
        }),
        None => Response::failure(
            Some(request.id.clone()),
            RpcError::not_found(format!("no job with id {}", params.job_id)),
        ),
    }
}

// ---------------------------------------------------------------------------
// Bridge into the iced subscription
// ---------------------------------------------------------------------------

/// Slot holding the bridge receiver between [`install_bridge`] (at
/// startup, before the first `subscription()` call) and the moment the
/// iced runtime spawns the [`bridge_stream`] recipe, which takes it.
static BRIDGE_RX: Mutex<Option<UnboundedReceiver<ControlMessage>>> = Mutex::new(None);

/// Park the bridge receiver for [`bridge_stream`] to pick up.
pub fn install_bridge(rx: UnboundedReceiver<ControlMessage>) {
    if let Ok(mut slot) = BRIDGE_RX.lock() {
        *slot = Some(rx);
    }
}

/// Builder for `Subscription::run`: turns the bridge receiver into the
/// subscription's message stream. `Subscription::run` keys the recipe on
/// this function pointer, so the stream is created exactly once per app
/// run; if the receiver was never installed (control disabled) the
/// stream just stays pending forever.
pub fn bridge_stream() -> BoxStream<'static, crate::message::Message> {
    use iced::futures::StreamExt;
    let receiver = BRIDGE_RX.lock().ok().and_then(|mut slot| slot.take());
    match receiver {
        Some(rx) => rx.map(crate::message::Message::Control).boxed(),
        None => iced::futures::stream::pending().boxed(),
    }
}
