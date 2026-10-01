//! The process-wide plok.org download worker (drums-plugin-rework.md §4):
//! fetches the server index, downloads kit zips with progress, verifies
//! them, and installs them through the kit library.
//!
//! There is **one** worker per process. It is owned by the shared kit
//! library ([`crate::library::SharedKitLibrary`]), not by each plugin
//! instance: N drum instances used to mean N `drums-download` threads.
//!
//! - The editor pushes [`Command`]s through [`WorkerHandle::send`]; the
//!   worker drains them on a dedicated thread, one at a time.
//! - Shared [`State`] behind `Arc<Mutex<…>>` is polled by the UI each frame.
//! - [`Command::Cancel`] acts at once, without queueing behind the
//!   transfer it cancels: it raises the abort flag the transfer checks
//!   every chunk (and the extraction every chunk and file), or drops a
//!   queued download before it starts. A cancelled download deletes its
//!   `.part` file.
//!
//! Dropping the [`WorkerHandle`] never waits on the network: it raises the
//! abort flag and detaches the thread if a transfer is in flight (it then
//! stops at the next chunk, entry or read timeout, and cleans up on the
//! way out).
//!
//! **Resume.** A transfer that fails on the network keeps its `.part`
//! (`.<Kit>.<pid>.zip.part` in the library root — per process, so two
//! processes never share one). The next download of that kit by this
//! process asks for `Range: bytes=<len>-` and appends when the server
//! answers 206; on any other answer it starts over.
//!
//! **Install.** A verified zip goes through `Library::install_zip` (or
//! `install_zip_replacing` for an update / re-download), so the library's
//! staging, promote and sidecar logic is the only way a kit lands: the
//! library never lists a half-extracted kit. The zip is deleted
//! afterwards, whatever happened.
//!
//! The thread is started by the first [`WorkerHandle::send`]: most
//! processes — headless renders, editors that never open the plok.org
//! tab — never download.

use std::collections::HashSet;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

use resonance_common::drumkit_library::{self, ImportJob, IndexMatch, Sidecar};
use resonance_common::library_marks;

use crate::library::SharedKitLibrary;

// ---------------------------------------------------------------------------
// Public constants
// ---------------------------------------------------------------------------

/// The drumkit distribution server's index.
pub const INDEX_URL: &str = "https://resonance.plok.org/index.json";

/// How long a read may wait for the next byte before the transfer is
/// abandoned as stalled. Per read, not per transfer: a 5 GiB kit takes as
/// long as it takes, but a connection that goes silent fails after this.
pub const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a fetched index is fresh: opening the plok.org tab within this
/// of the last fetch does not fetch again (Refresh always does).
pub const INDEX_TTL: Duration = Duration::from_secs(10 * 60);

/// A download needs this much free space per byte of zip: the zip and the
/// extracted kit coexist until the zip is deleted (§4.1).
pub const DOWNLOAD_SPACE_FACTOR: f64 = 2.1;

/// The fetched index, cached under the library root (hidden, so a scan
/// never sees it) for the `installed.json` migration and an offline tab.
pub const INDEX_CACHE_FILE: &str = ".plok-index.json";

// ---------------------------------------------------------------------------
// Server index types
// ---------------------------------------------------------------------------

/// Top-level response from the index endpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerIndex {
    #[serde(default)]
    pub drumkits: Vec<ServerKit>,
}

impl ServerIndex {
    /// The entry called `name` (case-insensitive).
    pub fn find(&self, name: &str) -> Option<&ServerKit> {
        let name = name.trim();
        self.drumkits
            .iter()
            .find(|k| k.name.trim().eq_ignore_ascii_case(name))
    }
}

/// One kit available for download (§4.2). Everything past `name` and
/// `file` is optional, and a field of the wrong type reads as absent
/// rather than failing the whole index.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerKit {
    pub name: String,
    pub file: String,
    /// The display size ("5.3 GiB"); the fallback when `bytes` is absent.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub size: Option<String>,
    /// The zip's exact size.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub bytes: Option<u64>,
    /// sha256 of the zip, lowercase hex. Checked before extraction.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub sha256: Option<String>,
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub pieces: Option<u64>,
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub mic_setups: Option<u64>,
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub description: Option<String>,
    #[serde(
        default,
        deserialize_with = "lenient_tags",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub tags: Vec<String>,
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub added: Option<String>,
    /// sha256 of the kit's `drum_samples.json` — the library id — so the
    /// client can tell "Installed" from "Update".
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub manifest_sha256: Option<String>,
}

impl ServerKit {
    /// A kit known only by name and file (the tests' and re-download's
    /// shape).
    pub fn new(name: impl Into<String>, file: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            file: file.into(),
            ..Self::default()
        }
    }

    /// "5.3 GB" from `bytes`, else the index's `size` string.
    pub fn size_text(&self) -> Option<String> {
        self.bytes
            .map(drumkit_library::format_bytes)
            .or_else(|| self.size.clone())
    }

    /// What the library's `installed.json` migration records for this kit.
    pub fn index_match(&self) -> IndexMatch {
        IndexMatch {
            index_name: Some(self.name.clone()),
            index_file: Some(self.file.clone()),
            sha256: self.sha256.clone(),
            description: self.description.clone(),
            index_tags: self.tags.clone(),
        }
    }

    /// The sidecar a download of this kit is installed with.
    pub fn sidecar(&self) -> Sidecar {
        Sidecar {
            source: drumkit_library::SOURCE_PLOK.into(),
            index_name: Some(self.name.clone()),
            index_file: Some(self.file.clone()),
            sha256: self.sha256.as_ref().map(|s| s.to_ascii_lowercase()),
            description: self.description.clone(),
            index_tags: self.tags.clone(),
            downloaded_at: library_marks::format_timestamp(library_marks::now_unix()),
            size_bytes: None,
        }
    }
}

/// `T` if the value deserializes as one, else `None` — one odd field does
/// not cost the user the whole index.
fn lenient<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(v).ok())
}

fn lenient_tags<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Array(items) => items
            .into_iter()
            .filter_map(|i| i.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    })
}

/// Parse an index document.
pub fn parse_index(bytes: &[u8]) -> Result<ServerIndex, String> {
    serde_json::from_slice(bytes).map_err(|e| format!("parse index: {e}"))
}

/// The index cached under `root`, if one was ever fetched.
pub fn read_index_cache(root: &Path) -> Option<ServerIndex> {
    let bytes = std::fs::read(root.join(INDEX_CACHE_FILE)).ok()?;
    parse_index(&bytes).ok()
}

fn write_index_cache(root: &Path, index: &ServerIndex) {
    if !root.is_dir() {
        // Nothing installed yet: do not create the library just for this.
        return;
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(index) {
        let _ = resonance_common::atomic_file::atomic_write(&root.join(INDEX_CACHE_FILE), &bytes);
    }
}

// ---------------------------------------------------------------------------
// Worker protocol
// ---------------------------------------------------------------------------

/// Commands the UI pushes to the worker.
#[derive(Debug, Clone)]
pub enum Command {
    /// Fetch the server index.
    FetchIndex,
    /// Download a kit and install it as a new library entry.
    Download(ServerKit),
    /// Download a kit and install it in place of the kit in
    /// `existing_dir` (Update / Re-download): the directory keeps its
    /// name, and an unchanged manifest keeps its id, slot and marks.
    Redownload {
        kit: ServerKit,
        existing_dir: PathBuf,
    },
    /// Cancel the download of the kit with this name, running or queued.
    /// Acts at once; never queues.
    Cancel(String),
    /// Stop the worker thread.
    Shutdown,
}

/// Current activity, surfaced in the UI.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Idle,
    FetchingIndex,
    Downloading {
        name: String,
        downloaded_bytes: u64,
        /// 0 when the server did not say.
        total_bytes: u64,
        /// Smoothed transfer rate, bytes per second (0 until measured).
        bytes_per_sec: f64,
        /// Seconds left at the current rate, once there is one.
        eta_secs: Option<f64>,
        /// Bytes kept from an earlier, interrupted transfer (Range resume).
        resumed_from: u64,
    },
    /// Checking the zip against the index's sha256.
    Verifying(String),
    Extracting {
        name: String,
        files_done: u64,
        files_total: u64,
        bytes_done: u64,
        bytes_total: u64,
    },
    Done(String),
    Cancelled(String),
    Error(String),
}

impl Status {
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            Status::FetchingIndex
                | Status::Downloading { .. }
                | Status::Verifying(_)
                | Status::Extracting { .. }
        )
    }

    /// The kit this status is about, while one is being worked on.
    pub fn active_kit(&self) -> Option<&str> {
        match self {
            Status::Downloading { name, .. }
            | Status::Verifying(name)
            | Status::Extracting { name, .. } => Some(name),
            _ => None,
        }
    }
}

/// A kit the worker installed.
#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    /// The index name it was downloaded as.
    pub name: String,
    /// Its library id.
    pub id: String,
    /// Its top directory.
    pub dir: PathBuf,
}

/// Shared state the UI reads each frame.
#[derive(Debug, Clone)]
pub struct State {
    pub status: Status,
    pub index: Option<ServerIndex>,
    /// When `index` was last fetched successfully.
    pub index_fetched_at: Option<Instant>,
    pub last_error: Option<String>,
    /// Downloads sent but not started, in order (cancelled ones excluded).
    pub queued: Vec<String>,
    /// The most recent install, and how many installs this worker has
    /// finished (a counter, so an editor sees each one once).
    pub last_installed: Option<Installed>,
    pub installs: u64,
    /// Names sent but not yet taken off the channel, and those of them
    /// cancelled before they started.
    pending: Vec<String>,
    cancelled: HashSet<String>,
    fetch_pending: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            status: Status::Idle,
            index: None,
            index_fetched_at: None,
            last_error: None,
            queued: Vec::new(),
            last_installed: None,
            installs: 0,
            pending: Vec::new(),
            cancelled: HashSet::new(),
            fetch_pending: false,
        }
    }
}

impl State {
    /// Whether the index should be fetched for a tab opening now: never
    /// fetched, or older than [`INDEX_TTL`], and no fetch already queued.
    pub fn index_is_stale(&self) -> bool {
        !self.fetch_pending
            && self
                .index_fetched_at
                .is_none_or(|at| at.elapsed() >= INDEX_TTL)
    }

    /// Whether `name` is downloading, verifying, extracting or queued.
    pub fn is_working_on(&self, name: &str) -> bool {
        self.status.active_kit() == Some(name) || self.queued.iter().any(|q| q == name)
    }

    fn refresh_queued(&mut self) {
        self.queued = self
            .pending
            .iter()
            .filter(|n| !self.cancelled.contains(*n))
            .cloned()
            .collect();
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Where and how the worker downloads. Tests point it at a local server.
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct WorkerConfig {
    /// The server index. Kit files resolve relative to it.
    pub index_url: String,
    /// The per-read stall limit, [`READ_TIMEOUT`] by default. Tests
    /// shorten it to see a stalled transfer fail in milliseconds.
    pub read_timeout: Duration,
    /// Called with each extracted file's index before the next is
    /// extracted. Test hook: lets a test cancel in the middle of an
    /// extraction.
    pub on_extract_entry: Option<ExtractHook>,
    /// Replaces the real free-space probe (bytes available at a path).
    /// Test hook for the disk-space refusal.
    pub free_space: Option<FreeSpaceFn>,
}

/// See [`WorkerConfig::on_extract_entry`].
#[doc(hidden)]
#[derive(Clone)]
pub struct ExtractHook(pub Arc<dyn Fn(usize) + Send + Sync>);

impl std::fmt::Debug for ExtractHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ExtractHook")
    }
}

/// See [`WorkerConfig::free_space`].
#[doc(hidden)]
#[derive(Clone)]
pub struct FreeSpaceFn(pub Arc<FreeSpaceProbe>);

/// Bytes available at a path; `None` when unknown (no check is made).
pub type FreeSpaceProbe = dyn Fn(&Path) -> Option<u64> + Send + Sync;

impl std::fmt::Debug for FreeSpaceFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FreeSpaceFn")
    }
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            index_url: INDEX_URL.to_string(),
            read_timeout: READ_TIMEOUT,
            on_extract_entry: None,
            free_space: None,
        }
    }
}

impl WorkerConfig {
    fn free_space_at(&self, path: &Path) -> Option<u64> {
        match &self.free_space {
            Some(f) => (f.0)(path),
            None => drumkit_library::free_space(path),
        }
    }

    /// The URL of a kit file, relative to the index.
    fn file_url(&self, file: &str) -> String {
        let index_url = self.index_url.as_str();
        let base = index_url
            .rsplit_once('/')
            .map(|(base, _)| base)
            .unwrap_or(index_url);
        format!("{base}/{file}")
    }
}

// ---------------------------------------------------------------------------
// Worker handle
// ---------------------------------------------------------------------------

/// The handle the shared library holds. One per library, so one per
/// process for the default library.
pub struct WorkerHandle {
    pub state: Arc<Mutex<State>>,
    config: WorkerConfig,
    root: Option<PathBuf>,
    library: Weak<SharedKitLibrary>,
    /// The worker thread, once the first command has started it.
    running: Mutex<Option<Running>>,
    /// Raised by `drop`; the worker checks it before each command.
    shutdown: Arc<AtomicBool>,
    /// Raised by `drop` and by a [`Command::Cancel`] of the running
    /// download; checked every chunk. Reset when a download starts.
    abort: Arc<AtomicBool>,
    /// True while the worker is handling a command.
    busy: Arc<AtomicBool>,
    /// How many threads this handle has started (test hook: there is
    /// only ever one).
    threads_started: AtomicUsize,
}

/// A started worker thread and its command channel.
struct Running {
    tx: Sender<Command>,
    join: Option<JoinHandle<()>>,
}

impl WorkerHandle {
    /// A handle for the library `library` rooted at `root`. No thread
    /// until the first command.
    pub(crate) fn new(
        config: WorkerConfig,
        root: Option<PathBuf>,
        state: Arc<Mutex<State>>,
        library: Weak<SharedKitLibrary>,
    ) -> Self {
        Self {
            state,
            config,
            root,
            library,
            running: Mutex::new(None),
            shutdown: Arc::new(AtomicBool::new(false)),
            abort: Arc::new(AtomicBool::new(false)),
            busy: Arc::new(AtomicBool::new(false)),
            threads_started: AtomicUsize::new(0),
        }
    }

    /// Queue `cmd`, starting the worker thread if this is the first one.
    /// [`Command::Cancel`] acts at once instead. A download already
    /// running or queued is not queued twice.
    pub fn send(&self, cmd: Command) {
        match &cmd {
            Command::Cancel(name) => {
                self.cancel(name);
                return;
            }
            Command::Download(kit) | Command::Redownload { kit, .. } => {
                let mut s = self.state.lock();
                if s.status.active_kit() == Some(kit.name.as_str()) {
                    return;
                }
                if s.pending.contains(&kit.name) {
                    // Still on the channel: a cancelled one comes back.
                    s.cancelled.remove(&kit.name);
                    s.refresh_queued();
                    return;
                }
                s.pending.push(kit.name.clone());
                s.refresh_queued();
            }
            Command::FetchIndex => {
                let mut s = self.state.lock();
                if s.fetch_pending {
                    return;
                }
                s.fetch_pending = true;
            }
            Command::Shutdown => {}
        }
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

    /// Cancel the download of `name`: abort it if it is running, drop it
    /// if it is queued.
    fn cancel(&self, name: &str) {
        let mut s = self.state.lock();
        if s.status.active_kit() == Some(name) {
            self.abort.store(true, Ordering::SeqCst);
        }
        if s.pending.iter().any(|n| n == name) {
            s.cancelled.insert(name.to_string());
            s.refresh_queued();
        }
    }

    /// Whether the worker thread has been started.
    pub fn is_running(&self) -> bool {
        self.running.lock().is_some()
    }

    /// How many threads this handle has ever started.
    #[doc(hidden)]
    pub fn threads_started(&self) -> usize {
        self.threads_started.load(Ordering::SeqCst)
    }

    /// Fetch the index on the calling thread, without the worker, and
    /// cache it (in `state` and on disk). For the one-time `installed.json`
    /// migration, which wants the index before its first scan.
    pub(crate) fn fetch_index_now(&self, timeout: Duration) -> Result<(), String> {
        let agent = make_agent(timeout);
        let index = fetch_index(&agent, &self.config.index_url)?;
        if let Some(root) = &self.root {
            write_index_cache(root, &index);
        }
        let mut s = self.state.lock();
        s.index = Some(index);
        s.index_fetched_at = Some(Instant::now());
        Ok(())
    }

    fn start(&self) -> Option<Running> {
        let (tx, rx) = mpsc::channel();
        let worker = Worker {
            config: self.config.clone(),
            root: self.root.clone(),
            library: self.library.clone(),
            state: self.state.clone(),
            shutdown: self.shutdown.clone(),
            abort: self.abort.clone(),
            busy: self.busy.clone(),
        };
        match std::thread::Builder::new()
            .name("drums-download".into())
            .spawn(move || worker_loop(rx, worker))
        {
            Ok(join) => {
                self.threads_started.fetch_add(1, Ordering::SeqCst);
                Some(Running {
                    tx,
                    join: Some(join),
                })
            }
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
    /// The flags are raised before `busy` is read and the worker raises
    /// `busy` before it reads them (all `SeqCst`), so either the worker
    /// sees the shutdown and returns at once — the join is then short —
    /// or we see it busy and detach. The last handle can also be dropped
    /// on the worker thread itself (it holds the library while it
    /// installs); that never joins either.
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.abort.store(true, Ordering::SeqCst);
        let Some(mut running) = self.running.get_mut().take() else {
            return;
        };
        let _ = running.tx.send(Command::Shutdown);
        if let Some(j) = running.join.take() {
            let on_worker = j.thread().id() == std::thread::current().id();
            if on_worker || self.busy.load(Ordering::SeqCst) {
                // Detached: it notices the abort at its next chunk (or the
                // read timeout), cleans up and exits.
                drop(j);
            } else {
                let _ = j.join();
            }
        }
    }
}

/// What the worker thread owns.
struct Worker {
    config: WorkerConfig,
    root: Option<PathBuf>,
    library: Weak<SharedKitLibrary>,
    state: Arc<Mutex<State>>,
    shutdown: Arc<AtomicBool>,
    abort: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
}

impl Worker {
    fn aborted(&self) -> bool {
        self.abort.load(Ordering::SeqCst) || self.shutdown.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------
// Worker loop
// ---------------------------------------------------------------------------

fn make_agent(read_timeout: Duration) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_send_request(Some(read_timeout))
        .timeout_recv_response(Some(read_timeout))
        // No total body budget — large downloads need more time. A
        // stalled body is caught per read by `ReadTimeout` instead.
        .build();
    ureq::Agent::with_parts(
        config,
        read_timeout::ReadTimeout::new(read_timeout),
        ureq::unversioned::resolver::DefaultResolver::default(),
    )
}

fn worker_loop(rx: Receiver<Command>, worker: Worker) {
    let agent = make_agent(worker.config.read_timeout);

    // Leftovers of downloads nobody is running any more.
    if let Some(root) = &worker.root {
        sweep_stale_downloads(root);
    }

    loop {
        let cmd = match rx.recv() {
            Ok(c) => c,
            Err(_) => return,
        };
        if matches!(cmd, Command::Shutdown) {
            return;
        }
        // Raise `busy` before looking at the flags — the other half of
        // the ordering `WorkerHandle::drop` relies on.
        worker.busy.store(true, Ordering::SeqCst);
        if worker.shutdown.load(Ordering::SeqCst) {
            worker.busy.store(false, Ordering::SeqCst);
            return;
        }
        match cmd {
            Command::Shutdown | Command::Cancel(_) => {}
            Command::FetchIndex => run_fetch(&agent, &worker),
            Command::Download(kit) => run_download(&agent, &worker, kit, None),
            Command::Redownload { kit, existing_dir } => {
                run_download(&agent, &worker, kit, Some(existing_dir))
            }
        }
        worker.busy.store(false, Ordering::SeqCst);
    }
}

fn run_fetch(agent: &ureq::Agent, worker: &Worker) {
    let state = &worker.state;
    {
        let mut s = state.lock();
        s.fetch_pending = false;
        s.status = Status::FetchingIndex;
    }
    match fetch_index(agent, &worker.config.index_url) {
        Ok(index) => {
            if let Some(root) = &worker.root {
                write_index_cache(root, &index);
            }
            let mut s = state.lock();
            s.index = Some(index);
            s.index_fetched_at = Some(Instant::now());
            s.status = Status::Idle;
            s.last_error = None;
        }
        Err(e) => set_error(state, &e),
    }
}

fn run_download(
    agent: &ureq::Agent,
    worker: &Worker,
    kit: ServerKit,
    existing_dir: Option<PathBuf>,
) {
    let state = &worker.state;
    {
        let mut s = state.lock();
        if let Some(i) = s.pending.iter().position(|n| *n == kit.name) {
            s.pending.remove(i);
        }
        let skipped = s.cancelled.remove(&kit.name);
        s.refresh_queued();
        if skipped {
            return;
        }
        // A fresh abort flag for this download: a cancel of an earlier
        // one must not carry over.
        worker.abort.store(false, Ordering::SeqCst);
        if worker.shutdown.load(Ordering::SeqCst) {
            return;
        }
        s.status = Status::Downloading {
            name: kit.name.clone(),
            downloaded_bytes: 0,
            total_bytes: kit.bytes.unwrap_or(0),
            bytes_per_sec: 0.0,
            eta_secs: None,
            resumed_from: 0,
        };
    }
    match download_and_install(agent, &kit, existing_dir.as_deref(), worker) {
        Ok(installed) => {
            let mut s = state.lock();
            s.status = Status::Done(kit.name.clone());
            s.last_error = None;
            s.last_installed = Some(installed);
            s.installs += 1;
        }
        Err(Failure::Cancelled) => {
            let mut s = state.lock();
            s.status = Status::Cancelled(kit.name.clone());
        }
        Err(Failure::Error(e)) => set_error(state, &e),
    }
}

fn set_error(state: &Arc<Mutex<State>>, msg: &str) {
    let mut s = state.lock();
    s.last_error = Some(msg.to_string());
    s.status = Status::Error(msg.to_string());
}

/// Why a download did not install.
enum Failure {
    Cancelled,
    Error(String),
}

impl From<String> for Failure {
    fn from(e: String) -> Self {
        Failure::Error(e)
    }
}

// ---------------------------------------------------------------------------
// Network helpers
// ---------------------------------------------------------------------------

fn fetch_index(agent: &ureq::Agent, index_url: &str) -> Result<ServerIndex, String> {
    let mut resp = agent
        .get(index_url)
        .call()
        .map_err(|e| format!("fetch index: {e}"))?;
    let body = resp
        .body_mut()
        .read_to_vec()
        .map_err(|e| format!("read index: {e}"))?;
    parse_index(&body)
}

/// The `.part` file a download of `kit_name` by this process streams into:
/// `<root>/.<Kit>.<pid>.zip.part`. Per process, so two processes never
/// write one file; the same for every download of the kit by this
/// process, so a retry finds what the failed attempt kept.
#[doc(hidden)]
pub fn part_path(root: &Path, kit_name: &str) -> PathBuf {
    root.join(format!(
        ".{}.{}{PART_SUFFIX}",
        sanitize(kit_name),
        std::process::id()
    ))
}

/// Download (resuming an own `.part` when the server allows), verify and
/// install.
fn download_and_install(
    agent: &ureq::Agent,
    kit: &ServerKit,
    existing_dir: Option<&Path>,
    worker: &Worker,
) -> Result<Installed, Failure> {
    let root = worker
        .root
        .clone()
        .ok_or_else(|| "no drum kit library directory (no data dir)".to_string())?;
    std::fs::create_dir_all(&root).map_err(|e| format!("mkdir {}: {e}", root.display()))?;
    let part = part_path(&root, &kit.name);

    // Refuse before a byte moves when the index says how big the zip is.
    let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if let Some(total) = kit.bytes {
        check_space(worker, &root, &kit.name, total, have)?;
    }

    let result = transfer(agent, kit, worker, &root, &part);
    match result {
        Ok(()) => {}
        // A network failure keeps the `.part` for a resume; everything
        // else (a cancel, a full disk, a bad server answer) removes it.
        Err(Transfer::Network(e)) => return Err(Failure::Error(e)),
        Err(Transfer::Other(f)) => {
            let _ = std::fs::remove_file(&part);
            return Err(f);
        }
    }

    // From here on the zip is complete; whatever happens it is deleted.
    let installed = verify_and_install(kit, existing_dir, worker, &part);
    let _ = std::fs::remove_file(&part);
    installed
}

/// How a transfer failed.
enum Transfer {
    /// The connection failed or stalled: the `.part` is kept for a resume.
    Network(String),
    Other(Failure),
}

impl From<Failure> for Transfer {
    fn from(f: Failure) -> Self {
        Transfer::Other(f)
    }
}

fn check_space(
    worker: &Worker,
    root: &Path,
    name: &str,
    total: u64,
    have: u64,
) -> Result<(), Failure> {
    let needed = ((total as f64 * DOWNLOAD_SPACE_FACTOR) as u64).saturating_sub(have);
    match worker.config.free_space_at(root) {
        Some(available) if available < needed => Err(Failure::Error(format!(
            "not enough free space to download \"{name}\": needs {}, {} available ({} short)",
            drumkit_library::format_bytes(needed),
            drumkit_library::format_bytes(available),
            drumkit_library::format_bytes(needed - available),
        ))),
        _ => Ok(()),
    }
}

/// Stream the kit into `part`, resuming from what is there when the server
/// answers a Range request with 206.
fn transfer(
    agent: &ureq::Agent,
    kit: &ServerKit,
    worker: &Worker,
    root: &Path,
    part: &Path,
) -> Result<(), Transfer> {
    let url = worker.config.file_url(&kit.file);
    let have = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);

    let mut request = agent.get(&url);
    if have > 0 {
        request = request.header("Range", format!("bytes={have}-"));
    }
    let resp = match request.call() {
        Ok(r) => r,
        // 416: the part is no prefix the server can continue (or it is
        // already whole and the file changed). Start over.
        Err(ureq::Error::StatusCode(416)) if have > 0 => {
            let _ = std::fs::remove_file(part);
            agent.get(&url).call().map_err(|e| {
                Transfer::Other(Failure::Error(format!("download {}: {e}", kit.name)))
            })?
        }
        Err(e @ ureq::Error::StatusCode(_)) => {
            return Err(Transfer::Other(Failure::Error(format!(
                "download {}: {e}",
                kit.name
            ))))
        }
        Err(e) => return Err(Transfer::Network(format!("download {}: {e}", kit.name))),
    };

    let resumed = resp.status().as_u16() == 206 && have > 0;
    let content_length: Option<u64> = resp
        .headers()
        .get("Content-Length")
        .and_then(|s| s.to_str().ok())
        .and_then(|s| s.parse().ok());
    let start = if resumed { have } else { 0 };
    let total = content_length.map(|n| n + start).or(kit.bytes).unwrap_or(0);
    if kit.bytes.is_none() && total > 0 {
        check_space(worker, root, &kit.name, total, start)?;
    }

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(!resumed)
        .open(part)
        .map_err(|e| Transfer::Other(Failure::Error(format!("create {}: {e}", part.display()))))?;
    if resumed {
        file.seek(std::io::SeekFrom::End(0))
            .map_err(|e| Transfer::Other(Failure::Error(format!("seek part file: {e}"))))?;
    }

    let mut reader = resp.into_body().into_reader();
    let mut buf = vec![0u8; 256 * 1024];
    let mut downloaded = start;
    let mut rate = Rate::new(start);
    loop {
        if worker.aborted() {
            return Err(Transfer::Other(Failure::Cancelled));
        }
        let n = match reader.read(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                // The abort can surface as a read error of a dropped
                // connection; report it as what it is.
                if worker.aborted() {
                    return Err(Transfer::Other(Failure::Cancelled));
                }
                return Err(Transfer::Network(format!("read body: {e}")));
            }
        };
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| Transfer::Other(Failure::Error(format!("write part file: {e}"))))?;
        downloaded += n as u64;
        let (bytes_per_sec, eta_secs) = rate.update(downloaded, total);
        worker.state.lock().status = Status::Downloading {
            name: kit.name.clone(),
            downloaded_bytes: downloaded,
            total_bytes: total,
            bytes_per_sec,
            eta_secs,
            resumed_from: start,
        };
    }
    file.flush()
        .map_err(|e| Transfer::Other(Failure::Error(format!("write part file: {e}"))))?;
    if total > 0 && downloaded < total {
        return Err(Transfer::Network(format!(
            "download {}: connection closed after {} of {}",
            kit.name,
            drumkit_library::format_bytes(downloaded),
            drumkit_library::format_bytes(total)
        )));
    }
    Ok(())
}

/// A smoothed transfer rate and the ETA it gives.
struct Rate {
    window_start: Instant,
    window_bytes: u64,
    bytes_per_sec: f64,
}

impl Rate {
    const WINDOW: Duration = Duration::from_millis(250);

    fn new(start_bytes: u64) -> Self {
        Self {
            window_start: Instant::now(),
            window_bytes: start_bytes,
            bytes_per_sec: 0.0,
        }
    }

    fn update(&mut self, downloaded: u64, total: u64) -> (f64, Option<f64>) {
        let elapsed = self.window_start.elapsed();
        if elapsed >= Self::WINDOW {
            let now = (downloaded - self.window_bytes) as f64 / elapsed.as_secs_f64();
            self.bytes_per_sec = if self.bytes_per_sec == 0.0 {
                now
            } else {
                0.7 * self.bytes_per_sec + 0.3 * now
            };
            self.window_start = Instant::now();
            self.window_bytes = downloaded;
        }
        let eta = (self.bytes_per_sec > 0.0 && total > downloaded)
            .then(|| (total - downloaded) as f64 / self.bytes_per_sec);
        (self.bytes_per_sec, eta)
    }
}

/// sha256 of `path`, lowercase hex, checking the abort flag every chunk.
fn sha256_of(path: &Path, worker: &Worker) -> Result<String, Failure> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("open zip: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        if worker.aborted() {
            return Err(Failure::Cancelled);
        }
        let n = file.read(&mut buf).map_err(|e| format!("read zip: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Check the zip against the index's sha256 (when it gives one) and install
/// it through the library.
fn verify_and_install(
    kit: &ServerKit,
    existing_dir: Option<&Path>,
    worker: &Worker,
    zip: &Path,
) -> Result<Installed, Failure> {
    if let Some(expected) = &kit.sha256 {
        worker.state.lock().status = Status::Verifying(kit.name.clone());
        let actual = sha256_of(zip, worker)?;
        if !actual.eq_ignore_ascii_case(expected.trim()) {
            return Err(Failure::Error(format!(
                "\"{}\" failed verification: the download's sha256 is {actual}, the index says \
                 {expected}. Nothing was installed.",
                kit.name
            )));
        }
    }
    if worker.aborted() {
        return Err(Failure::Cancelled);
    }

    // Held only while installing: a strong reference across a transfer
    // would keep the library (and so this worker) alive after the last
    // editor and instance let go of it.
    let library = worker
        .library
        .upgrade()
        .ok_or_else(|| "the kit library was closed".to_string())?;
    let result = install(&library, kit, existing_dir, worker, zip);
    drop(library);
    result
}

fn install(
    library: &SharedKitLibrary,
    kit: &ServerKit,
    existing_dir: Option<&Path>,
    worker: &Worker,
    zip: &Path,
) -> Result<Installed, Failure> {
    let name = kit.name.clone();
    let state = worker.state.clone();
    let hook = worker.config.on_extract_entry.clone();
    let mut last_file = None;
    let progress = move |p: drumkit_library::ImportProgress| {
        if let Some(h) = &hook {
            if last_file != Some(p.files_done) {
                last_file = Some(p.files_done);
                (h.0)(p.files_done as usize);
            }
        }
        state.lock().status = Status::Extracting {
            name: name.clone(),
            files_done: p.files_done,
            files_total: p.files_total,
            bytes_done: p.bytes_done,
            bytes_total: p.bytes_total,
        };
    };
    let free = worker.config.free_space.clone();
    let make_job = || {
        let job = ImportJob::new().cancel(&worker.abort).progress(progress);
        match free {
            Some(f) => job.free_space(move |p| (f.0)(p)),
            None => job,
        }
    };
    let sidecar = kit.sidecar();
    let outcome = match existing_dir {
        None => library
            .mutate(|lib| lib.install_zip(zip, &kit.name, sidecar, make_job()))
            .map(|o| o.entry().clone()),
        Some(dir) => library.mutate(|lib| lib.install_zip_replacing(zip, dir, sidecar, make_job())),
    };
    let entry = outcome.map_err(|e| match e {
        drumkit_library::LibraryError::Cancelled => Failure::Cancelled,
        e => Failure::Error(format!("install \"{}\": {e}", kit.name)),
    })?;
    library.prune_marks();
    Ok(Installed {
        name: kit.name.clone(),
        id: entry.id,
        dir: entry.dir,
    })
}

// ---------------------------------------------------------------------------
// Leftovers
// ---------------------------------------------------------------------------

/// Suffix of the file a download streams into.
const PART_SUFFIX: &str = ".zip.part";
/// Suffix of the directory the pre-library worker extracted into before
/// moving it into place. Nothing writes these any more; leftovers are
/// still swept.
const STAGING_SUFFIX: &str = ".extracting";

/// A download's leftovers no live process could still be using are
/// removed after this long without a write. A live transfer writes at
/// least once per read timeout or fails, so an hour of silence means its
/// process is gone (or it failed, and a resume an hour later is not worth
/// keeping gigabytes around for).
const STALE_DOWNLOAD_AGE: Duration = Duration::from_secs(60 * 60);

/// The process id in a download leftover's name, if it has one:
/// `.<Kit>.<pid>.zip.part`, or the older `.<Kit>.<pid>-<n>…` (names from
/// before the per-process stem had none).
fn leftover_pid(name: &str) -> Option<u32> {
    let stem = name
        .strip_suffix(PART_SUFFIX)
        .or_else(|| name.strip_suffix(STAGING_SUFFIX))?;
    let (_, tag) = stem.rsplit_once('.')?;
    let pid = match tag.split_once('-') {
        Some((pid, n)) => {
            n.parse::<u64>().ok()?;
            pid
        }
        None => tag,
    };
    pid.parse().ok()
}

/// Remove `.part` files and staging directories in `dir` that no running
/// download can own: never this process's own (it may resume them), those
/// of a process that no longer exists (where that can be told — Linux),
/// and any untouched for [`STALE_DOWNLOAD_AGE`].
fn sweep_stale_downloads(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let me = std::process::id();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_part = name.ends_with(PART_SUFFIX);
        let is_staging = name.ends_with(STAGING_SUFFIX);
        if !name.starts_with('.') || !(is_part || is_staging) {
            continue;
        }
        let pid = leftover_pid(&name);
        if pid == Some(me) {
            continue;
        }
        let owner_gone = pid.is_some_and(process_is_gone);
        let untouched = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > STALE_DOWNLOAD_AGE);
        if !(owner_gone || untouched) {
            continue;
        }
        let path = entry.path();
        let _ = if is_staging {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
    }
}

/// True only when `pid` is known not to be running. Linux can tell from
/// `/proc`; elsewhere this says "don't know" (false) and the age rule
/// alone decides.
fn process_is_gone(pid: u32) -> bool {
    if cfg!(target_os = "linux") {
        Path::new("/proc/self").exists() && !Path::new(&format!("/proc/{pid}")).exists()
    } else {
        false
    }
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
