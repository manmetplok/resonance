//! Drumkit download worker: fetches the server index, downloads kit zips
//! with progress, and extracts them to `$XDG_DATA_HOME/resonance/drumkits/`.
//!
//! Follows the same architecture as the amp plugin's Tone3000 worker:
//! - The editor pushes [`Command`]s via an `mpsc::Sender`.
//! - The worker drains them on a dedicated thread.
//! - Shared [`State`] behind `Arc<Mutex<…>>` is polled by the UI each frame.
//!
//! Dropping the [`WorkerHandle`] never waits on the network: it raises a
//! cancel flag the transfer checks between chunks, and detaches the
//! thread if a transfer is in flight (it then stops at the next chunk or
//! the read timeout, and removes its `.part` file on the way out).
//!
//! The thread is started by the first [`WorkerHandle::send`], not by
//! [`spawn`]: every plugin instance owns a handle, and most — headless
//! renders, instances whose editor is never opened — never download.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use parking_lot::Mutex;
use serde::Deserialize;

use resonance_common::registry::{self, ContentType, InstalledItem};

// ---------------------------------------------------------------------------
// Public constants
// ---------------------------------------------------------------------------

/// Base URL of the drumkit distribution server.
const INDEX_URL: &str = "https://resonance.plok.org/index.json";

/// How long a read may wait for the next byte before the transfer is
/// abandoned as stalled. Per read, not per transfer: a 5 GiB kit takes as
/// long as it takes, but a connection that goes silent fails after this.
pub const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Resolve the per-user directory where drumkits are stored. One
/// definition, shared with the loader's kit naming.
pub fn drumkits_dir() -> Option<PathBuf> {
    crate::kit_loader::drumkits_root()
}

// ---------------------------------------------------------------------------
// Server index types
// ---------------------------------------------------------------------------

/// Top-level response from the index endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerIndex {
    #[serde(default)]
    pub drumkits: Vec<ServerKit>,
}

/// One kit available for download.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct ServerKit {
    pub name: String,
    pub file: String,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub added: Option<String>,
}

// ---------------------------------------------------------------------------
// Worker protocol
// ---------------------------------------------------------------------------

/// Commands the UI pushes to the worker.
pub enum Command {
    /// Fetch the server index.
    FetchIndex,
    /// Download and extract a kit.
    Download(ServerKit),
    /// Gracefully stop the worker thread.
    Shutdown,
}

/// Current activity, surfaced in the UI.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Status {
    Idle,
    FetchingIndex,
    Downloading {
        name: String,
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    Extracting(String),
    Done(String),
    Error(String),
}

impl Status {
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            Status::FetchingIndex | Status::Downloading { .. } | Status::Extracting(_)
        )
    }
}

/// Shared state the UI reads each frame.
pub struct State {
    pub status: Status,
    pub index: Option<ServerIndex>,
    pub last_error: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            status: Status::Idle,
            index: None,
            last_error: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Worker handle
// ---------------------------------------------------------------------------

/// Where and how a worker downloads. [`spawn`] uses the defaults; tests
/// point it at a local server.
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct WorkerConfig {
    /// The server index. Kit files resolve relative to it.
    pub index_url: String,
    /// Resonance's per-user data directory: kits are extracted into its
    /// `drumkits/` and recorded in its `installed.json`. `None` = the
    /// platform's (see [`drumkits_dir`]). Set by tests so they never
    /// touch the real one — through this rather than `$XDG_DATA_HOME`,
    /// which `dirs::data_dir` ignores on macOS.
    pub data_dir: Option<PathBuf>,
    /// The per-read stall limit, [`READ_TIMEOUT`] by default. Tests
    /// shorten it to see a stalled transfer fail in milliseconds.
    pub read_timeout: Duration,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            index_url: INDEX_URL.to_string(),
            data_dir: None,
            read_timeout: READ_TIMEOUT,
        }
    }
}

impl WorkerConfig {
    /// Where kits are extracted.
    pub fn drumkits_dir(&self) -> Option<PathBuf> {
        match &self.data_dir {
            Some(dir) => Some(dir.join("drumkits")),
            None => drumkits_dir(),
        }
    }

    /// Record `item` in the installed-content registry.
    fn mark_installed(&self, item: InstalledItem) -> Result<(), String> {
        match &self.data_dir {
            None => registry::mark_installed(item).map_err(|e| e.to_string()),
            Some(dir) => {
                // `registry::mark_installed`, against this directory's file.
                let path = dir.join("installed.json");
                let mut reg = registry::load_registry_from(&path);
                reg.items.retain(|existing| {
                    !(existing.name == item.name && existing.content_type == item.content_type)
                });
                reg.items.push(item);
                registry::save_registry_to(&reg, &path).map_err(|e| e.to_string())
            }
        }
    }
}

pub struct WorkerHandle {
    pub state: Arc<Mutex<State>>,
    config: WorkerConfig,
    /// The worker thread, once the first command has started it.
    running: Mutex<Option<Running>>,
    /// Raised by `drop`; the worker checks it before each command and
    /// between body chunks, and abandons what it is doing.
    cancel: Arc<AtomicBool>,
    /// True while the worker is on the network (index fetch or download).
    busy: Arc<AtomicBool>,
}

/// A started worker thread and its command channel.
struct Running {
    tx: Sender<Command>,
    join: Option<JoinHandle<()>>,
}

impl WorkerHandle {
    /// Queue `cmd`, starting the worker thread if this is the first one.
    pub fn send(&self, cmd: Command) {
        let mut running = self.running.lock();
        if running.is_none() {
            if matches!(cmd, Command::Shutdown) {
                return;
            }
            *running = self.start();
        }
        if let Some(r) = running.as_ref() {
            let _ = r.tx.send(cmd);
        }
    }

    /// Whether the worker thread has been started.
    pub fn is_running(&self) -> bool {
        self.running.lock().is_some()
    }

    fn start(&self) -> Option<Running> {
        let (tx, rx) = mpsc::channel();
        let worker = Worker {
            config: self.config.clone(),
            state: self.state.clone(),
            cancel: self.cancel.clone(),
            busy: self.busy.clone(),
        };
        match std::thread::Builder::new()
            .name("drums-download".into())
            .spawn(move || worker_loop(rx, worker))
        {
            Ok(join) => Some(Running {
                tx,
                join: Some(join),
            }),
            Err(e) => {
                set_error(&self.state, &format!("start download worker: {e}"));
                None
            }
        }
    }
}

impl Drop for WorkerHandle {
    /// Never blocks on the network. Closing a project during a 5 GiB
    /// download used to wait for the download to finish.
    ///
    /// `cancel` is raised before `busy` is read and the worker raises
    /// `busy` before it reads `cancel` (all `SeqCst`), so either the
    /// worker sees the cancel and returns at once — the join is then
    /// short — or we see it busy and detach.
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        let Some(mut running) = self.running.get_mut().take() else {
            // Never started: no thread to stop.
            return;
        };
        let _ = running.tx.send(Command::Shutdown);
        if let Some(j) = running.join.take() {
            if self.busy.load(Ordering::SeqCst) {
                // Detached: it notices the cancel at its next chunk (or
                // the read timeout), deletes its `.part` and exits.
                drop(j);
            } else {
                let _ = j.join();
            }
        }
    }
}

/// A handle for the default server. No thread is started until the
/// first command is sent.
pub fn spawn() -> WorkerHandle {
    spawn_with(WorkerConfig::default())
}

/// [`spawn`] against another index URL; kit files resolve relative to
/// it. Test hook: the download tests serve both from a local listener.
#[doc(hidden)]
pub fn spawn_with_index(index_url: String) -> WorkerHandle {
    spawn_with(WorkerConfig {
        index_url,
        ..WorkerConfig::default()
    })
}

/// [`spawn`] with every knob exposed. Test hook.
#[doc(hidden)]
pub fn spawn_with(config: WorkerConfig) -> WorkerHandle {
    WorkerHandle {
        state: Arc::new(Mutex::new(State::default())),
        config,
        running: Mutex::new(None),
        cancel: Arc::new(AtomicBool::new(false)),
        busy: Arc::new(AtomicBool::new(false)),
    }
}

/// What the worker thread owns.
struct Worker {
    config: WorkerConfig,
    state: Arc<Mutex<State>>,
    cancel: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
}

// ---------------------------------------------------------------------------
// Worker loop
// ---------------------------------------------------------------------------

fn worker_loop(rx: Receiver<Command>, worker: Worker) {
    let read_timeout = worker.config.read_timeout;
    let config = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_send_request(Some(read_timeout))
        .timeout_recv_response(Some(read_timeout))
        // No total body budget — large downloads need more time. A
        // stalled body is caught per read by `ReadTimeout` instead.
        .build();
    let agent = ureq::Agent::with_parts(
        config,
        read_timeout::ReadTimeout::new(read_timeout),
        ureq::unversioned::resolver::DefaultResolver::default(),
    );
    let state = &worker.state;

    loop {
        let cmd = match rx.recv() {
            Ok(c) => c,
            Err(_) => return,
        };
        if matches!(cmd, Command::Shutdown) {
            return;
        }
        // Raise `busy` before looking at `cancel` — the other half of
        // the ordering `WorkerHandle::drop` relies on.
        worker.busy.store(true, Ordering::SeqCst);
        if worker.cancel.load(Ordering::SeqCst) {
            worker.busy.store(false, Ordering::SeqCst);
            return;
        }
        match cmd {
            Command::Shutdown => {}
            Command::FetchIndex => {
                state.lock().status = Status::FetchingIndex;
                match fetch_index(&agent, &worker.config.index_url) {
                    Ok(index) => {
                        let mut s = state.lock();
                        s.index = Some(index);
                        s.status = Status::Idle;
                        s.last_error = None;
                    }
                    Err(e) => set_error(state, &e),
                }
            }
            Command::Download(kit) => {
                state.lock().status = Status::Downloading {
                    name: kit.name.clone(),
                    downloaded_bytes: 0,
                    total_bytes: 0,
                };
                match download_and_extract(&agent, &kit, &worker) {
                    Ok(dest) => {
                        // Mark in the shared registry.
                        let _ = worker.config.mark_installed(InstalledItem {
                            name: kit.name.clone(),
                            content_type: ContentType::Drumkit,
                            path: dest.to_string_lossy().into_owned(),
                            installed_at: registry::today_iso(),
                        });
                        let mut s = state.lock();
                        s.status = Status::Done(kit.name.clone());
                        s.last_error = None;
                    }
                    Err(e) => set_error(state, &e),
                }
            }
        }
        worker.busy.store(false, Ordering::SeqCst);
    }
}

fn set_error(state: &Arc<Mutex<State>>, msg: &str) {
    let mut s = state.lock();
    s.last_error = Some(msg.to_string());
    s.status = Status::Error(msg.to_string());
}

// ---------------------------------------------------------------------------
// Network helpers
// ---------------------------------------------------------------------------

fn fetch_index(agent: &ureq::Agent, index_url: &str) -> Result<ServerIndex, String> {
    let mut resp = agent
        .get(index_url)
        .call()
        .map_err(|e| format!("fetch index: {e}"))?;
    let index: ServerIndex = resp
        .body_mut()
        .read_json()
        .map_err(|e| format!("parse index: {e}"))?;
    Ok(index)
}

/// Stream-download the kit zip, then extract it.
fn download_and_extract(
    agent: &ureq::Agent,
    kit: &ServerKit,
    worker: &Worker,
) -> Result<PathBuf, String> {
    // Build the download URL relative to the index URL base.
    let index_url = worker.config.index_url.as_str();
    let base = index_url
        .rsplit_once('/')
        .map(|(base, _)| base)
        .unwrap_or(index_url);
    let url = format!("{base}/{}", kit.file);

    let resp = agent
        .get(&url)
        .call()
        .map_err(|e| format!("download {}: {e}", kit.name))?;

    let total: u64 = resp
        .headers()
        .get("Content-Length")
        .and_then(|s| s.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // Stream to a temporary file so we don't hold GiBs in RAM.
    let dir = worker
        .config
        .drumkits_dir()
        .ok_or_else(|| "no data dir".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;

    let tmp_path = dir.join(part_file_name(&kit.name));
    // From here on every way out removes the partial file: a cancel, a
    // network or disk error, a bad zip, and success alike.
    let result = stream_and_extract(resp, kit, worker, &tmp_path, &dir, total);
    let _ = std::fs::remove_file(&tmp_path);
    result
}

/// Stream the body into `tmp_path` and extract it. The caller removes
/// `tmp_path` whatever this returns.
fn stream_and_extract(
    resp: ureq::http::Response<ureq::Body>,
    kit: &ServerKit,
    worker: &Worker,
    tmp_path: &Path,
    dir: &Path,
    total: u64,
) -> Result<PathBuf, String> {
    let state = &worker.state;
    let mut tmp_file =
        std::fs::File::create(tmp_path).map_err(|e| format!("create temp file: {e}"))?;

    // Read in 256 KiB chunks, updating progress.
    let mut reader = resp.into_body().into_reader();
    let mut buf = vec![0u8; 256 * 1024];
    let mut downloaded: u64 = 0;

    loop {
        if worker.cancel.load(Ordering::SeqCst) {
            return Err(format!("download of {} cancelled", kit.name));
        }
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("read body: {e}"))?;
        if n == 0 {
            break;
        }
        tmp_file
            .write_all(&buf[..n])
            .map_err(|e| format!("write temp file: {e}"))?;
        downloaded += n as u64;

        state.lock().status = Status::Downloading {
            name: kit.name.clone(),
            downloaded_bytes: downloaded,
            total_bytes: total,
        };
    }
    drop(tmp_file);

    // Extract the zip.
    state.lock().status = Status::Extracting(kit.name.clone());

    let dest = dir.join(sanitize(&kit.name));
    extract_zip(tmp_path, &dest)?;

    Ok(dest)
}

fn extract_zip(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| format!("open zip: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("read zip: {e}"))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("zip entry {i}: {e}"))?;

        let out_path = match entry.enclosed_name() {
            Some(name) => dest.join(name),
            None => continue, // skip entries with suspicious paths
        };

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)
                .map_err(|e| format!("mkdir {}: {e}", out_path.display()))?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
            }
            let mut out_file = std::fs::File::create(&out_path)
                .map_err(|e| format!("create {}: {e}", out_path.display()))?;
            std::io::copy(&mut entry, &mut out_file)
                .map_err(|e| format!("extract {}: {e}", out_path.display()))?;
        }
    }
    Ok(())
}

/// The temporary file a download of `kit_name` streams into:
/// `.<Kit>.<pid>-<n>.zip.part`. Unique per process and per download, so
/// two plugin instances (or two processes) fetching the same kit at once
/// each write their own file instead of interleaving into one — and one
/// finishing never deletes the other's.
fn part_file_name(kit_name: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!(
        ".{}.{}-{n}.zip.part",
        sanitize(kit_name),
        std::process::id()
    )
}

/// Conservative filename sanitizer: keep ASCII alphanumerics, `-`, `_`, `.`;
/// replace whitespace with `_`, drop everything else.
fn sanitize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else if ch.is_whitespace() {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push_str("kit");
    }
    out
}

/// A per-read timeout for ureq 3, which only offers a total budget for a
/// response body (`ConfigBuilder::timeout_recv_body`) — unusable for a
/// multi-GiB download. This connector wraps ureq's default chain (TCP,
/// proxies, TLS) and caps every wait on the socket at the configured read
/// timeout ([`READ_TIMEOUT`] unless a test sets another),
/// so a connection that goes silent fails instead of hanging the worker
/// forever.
///
/// Built on `ureq::unversioned`, which is outside ureq's semver promise:
/// a ureq update that breaks this fails the build here, not at runtime.
mod read_timeout {
    use std::fmt;
    use std::time::Duration;

    use ureq::unversioned::transport::time::Duration as UreqDuration;
    use ureq::unversioned::transport::{
        Buffers, ConnectionDetails, Connector, DefaultConnector, NextTimeout, Transport,
    };
    use ureq::Error;

    pub struct ReadTimeout {
        inner: DefaultConnector,
        limit: Duration,
    }

    impl ReadTimeout {
        pub fn new(limit: Duration) -> Self {
            Self {
                inner: DefaultConnector::new(),
                limit,
            }
        }
    }

    impl fmt::Debug for ReadTimeout {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("ReadTimeout")
                .field("limit", &self.limit)
                .finish()
        }
    }

    impl Connector<()> for ReadTimeout {
        type Out = Capped;

        fn connect(
            &self,
            details: &ConnectionDetails,
            chained: Option<()>,
        ) -> Result<Option<Self::Out>, Error> {
            Ok(self.inner.connect(details, chained)?.map(|inner| Capped {
                inner,
                limit: self.limit,
            }))
        }
    }

    pub struct Capped {
        inner: Box<dyn Transport>,
        limit: Duration,
    }

    impl fmt::Debug for Capped {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("Capped")
                .field("inner", &self.inner)
                .finish()
        }
    }

    impl Capped {
        /// The earlier of ureq's own deadline and our per-read limit.
        fn cap(&self, timeout: NextTimeout) -> NextTimeout {
            if timeout.after.is_not_happening() || *timeout.after > self.limit {
                NextTimeout {
                    after: UreqDuration::Exact(self.limit),
                    reason: timeout.reason,
                }
            } else {
                timeout
            }
        }
    }

    impl Transport for Capped {
        fn buffers(&mut self) -> &mut dyn Buffers {
            self.inner.buffers()
        }

        fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), Error> {
            let timeout = self.cap(timeout);
            self.inner.transmit_output(amount, timeout)
        }

        fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
            let timeout = self.cap(timeout);
            self.inner.await_input(timeout)
        }

        fn is_open(&mut self) -> bool {
            self.inner.is_open()
        }

        fn is_tls(&self) -> bool {
            self.inner.is_tls()
        }
    }
}
