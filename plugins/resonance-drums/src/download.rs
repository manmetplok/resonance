//! The process-wide plok.org download worker (drums-plugin-rework.md §4):
//! fetches the server index, downloads kit zips with progress, verifies
//! them, and installs them through the kit library.
//!
//! There is **one** worker per process. It is owned by the shared kit
//! library ([`crate::library::SharedKitLibrary`]), not by each plugin
//! instance: N drum instances used to mean N `drums-download` threads.
//!
//! - The editor pushes [`Command`]s through [`WorkerHandle::send`].
//!   Downloads queue in [`State`] and run one at a time on the worker
//!   thread.
//! - **Index fetches never queue behind a download.** Each runs on its own
//!   short-lived `drums-index` thread, with a total time budget, so the
//!   plok.org tab is not stuck on "loading" behind a 5 GiB transfer.
//! - Shared [`State`] behind `Arc<Mutex<…>>` is polled by the UI each frame.
//! - [`Command::Cancel`] acts at once, without queueing behind the
//!   transfer it cancels: it raises the abort flag the transfer checks
//!   every chunk (and the extraction every chunk and file), or drops a
//!   queued download before it starts. A cancelled download deletes its
//!   `.part` file.
//!
//! **Lifetime.** While a download runs, the worker holds the library
//! strongly: closing the editor (or deleting the track) that started it
//! does not stop it while the process still has drum instances
//! ([`WorkerConfig::keep_alive`]). When nothing holds the library any more
//! the transfer is *abandoned*: it stops at the next chunk and keeps its
//! `.part` for a resume. Dropping the [`WorkerHandle`] never waits on the
//! network.
//!
//! **Transport.** The HTTP client, the index fetch and cache, the
//! resumable `.part` download (`Range` + `If-Range`, adopted across
//! handles and processes), the free-space check, the timeouts and the
//! sha256 verify are shared with the app's voicebank library:
//! `resonance_common::plok_download`. A transfer that fails on the network
//! or is abandoned keeps its `.part` for a resume; a cancelled one deletes
//! it. The sha256 is computed while the zip streams in (seeded from the
//! kept bytes on a resume), never by re-reading gigabytes.
//!
//! **Install.** A verified zip goes through `Library::install_zip` (or
//! `install_zip_replacing` for an update / re-download), so the library's
//! staging, promote and sidecar logic is the only way a kit lands: the
//! library never lists a half-extracted kit. An update that changes the
//! kit's id carries its favourite, tags and recents to the new id.
//!
//! The worker thread is started by the first download: most processes —
//! headless renders, editors that never open the plok.org tab — never
//! download.

use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use resonance_common::drumkit_library::{self, ImportJob, IndexMatch, Sidecar};
use resonance_common::library_marks;
use resonance_common::plok_download::{
    self as plok, lenient, lenient_tags, DownloadError, HttpConfig, Stop, Target,
};

use crate::library::SharedKitLibrary;

// ---------------------------------------------------------------------------
// Public constants
// ---------------------------------------------------------------------------

/// The distribution server's index, the per-read stall limit, the
/// connect limit, one index fetch's budget, how long a fetched index is
/// fresh, the free space a download needs per byte of zip, and the index
/// cache's file name — all shared with the app's voicebank downloads.
pub use resonance_common::plok_download::{
    CONNECT_TIMEOUT, DOWNLOAD_SPACE_FACTOR, INDEX_CACHE_FILE, INDEX_FETCH_TIMEOUT, INDEX_TTL,
    INDEX_URL, READ_TIMEOUT,
};

/// How long a rescan waits on the network for the index when the
/// `installed.json` migration is about to run without one.
pub const MIGRATION_INDEX_TIMEOUT: Duration = Duration::from_secs(5);

/// How many finished installs [`State::installs_since`] remembers.
const INSTALL_NOTICES: usize = 32;

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

    /// The download of this kit's file (kit files resolve relative to the
    /// index).
    fn target(&self, config: &WorkerConfig) -> Target {
        Target {
            name: self.name.clone(),
            file: self.file.clone(),
            url: config.file_url(&self.file),
            bytes: self.bytes,
            sha256: self.sha256.clone(),
        }
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

/// Parse an index document.
pub fn parse_index(bytes: &[u8]) -> Result<ServerIndex, String> {
    plok::parse_index(bytes).map_err(|e| e.to_string())
}

/// The index cached under `root`, if one was ever fetched.
pub fn read_index_cache(root: &Path) -> Option<ServerIndex> {
    plok::read_index_cache(root)
}

fn write_index_cache(root: &Path, index: &ServerIndex) {
    plok::write_index_cache(root, index);
}

// ---------------------------------------------------------------------------
// Worker protocol
// ---------------------------------------------------------------------------

/// Commands the UI pushes to the worker.
#[derive(Debug, Clone)]
pub enum Command {
    /// Fetch the server index (on its own thread; never queued behind a
    /// download). Sent while a fetch runs, it runs once more afterwards.
    FetchIndex,
    /// Download a kit and install it as a new library entry.
    Download(ServerKit),
    /// Download a kit and install it in place of the kit in
    /// `existing_dir` (Update / Re-download): the directory keeps its
    /// name, and an unchanged manifest keeps its id, slot and marks (a
    /// changed one gets a new id, and the marks move to it).
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

/// Current activity, surfaced in the UI. While a download runs it is the
/// download's; an index fetch shows as [`Status::FetchingIndex`] only when
/// nothing else is going on (see [`State::fetching_index`]).
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

/// A download waiting for the worker.
#[derive(Debug, Clone)]
struct Job {
    kit: ServerKit,
    existing_dir: Option<PathBuf>,
}

/// Shared state the UI reads each frame.
#[derive(Debug, Clone)]
pub struct State {
    pub status: Status,
    pub index: Option<ServerIndex>,
    /// When `index` was last fetched successfully.
    pub index_fetched_at: Option<Instant>,
    /// The last download's error (an index fetch never sets or clears it).
    pub last_error: Option<String>,
    /// The last index fetch's error; `None` after one succeeds.
    pub index_error: Option<String>,
    /// An index fetch is running (on its own thread; a download may run
    /// beside it).
    pub fetching_index: bool,
    /// Downloads sent but not started, in order.
    pub queued: Vec<String>,
    /// The most recent install, and how many installs this worker has
    /// finished (a counter, so an editor sees each one once). Two installs
    /// can finish between two frames: read them with
    /// [`State::installs_since`].
    pub last_installed: Option<Installed>,
    pub installs: u64,
    /// The last [`INSTALL_NOTICES`] installs, each with its `installs`
    /// count.
    recent_installs: VecDeque<(u64, Installed)>,
    jobs: VecDeque<Job>,
    /// A fetch was asked for while one ran: run once more.
    fetch_again: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            status: Status::Idle,
            index: None,
            index_fetched_at: None,
            last_error: None,
            index_error: None,
            fetching_index: false,
            queued: Vec::new(),
            last_installed: None,
            installs: 0,
            recent_installs: VecDeque::new(),
            jobs: VecDeque::new(),
            fetch_again: false,
        }
    }
}

impl State {
    /// Whether the index should be fetched for a tab opening now: never
    /// fetched, or older than [`INDEX_TTL`], and no fetch running.
    pub fn index_is_stale(&self) -> bool {
        !self.fetching_index
            && self
                .index_fetched_at
                .is_none_or(|at| at.elapsed() >= INDEX_TTL)
    }

    /// Whether `name` is downloading, verifying, extracting or queued.
    pub fn is_working_on(&self, name: &str) -> bool {
        self.status.active_kit() == Some(name) || self.queued.iter().any(|q| q == name)
    }

    /// Every install finished after the `seen`-th, oldest first (at most
    /// the last [`INSTALL_NOTICES`]). An editor keeps the `installs` count
    /// it last saw and passes it here: `let new = s.installs_since(seen);
    /// seen = s.installs;`.
    pub fn installs_since(&self, seen: u64) -> Vec<Installed> {
        self.recent_installs
            .iter()
            .filter(|(n, _)| *n > seen)
            .map(|(_, i)| i.clone())
            .collect()
    }

    fn record_install(&mut self, installed: Installed) {
        self.installs += 1;
        self.last_installed = Some(installed.clone());
        self.recent_installs.push_back((self.installs, installed));
        while self.recent_installs.len() > INSTALL_NOTICES {
            self.recent_installs.pop_front();
        }
    }

    fn refresh_queued(&mut self) {
        self.queued = self.jobs.iter().map(|j| j.kit.name.clone()).collect();
    }

    fn remove_job(&mut self, name: &str) -> bool {
        let before = self.jobs.len();
        self.jobs.retain(|j| j.kit.name != name);
        self.refresh_queued();
        self.jobs.len() != before
    }

    /// An index fetch finished.
    fn fetched(&mut self, result: Result<ServerIndex, String>) {
        match result {
            Ok(index) => {
                self.index = Some(index);
                self.index_fetched_at = Some(Instant::now());
                self.index_error = None;
                if self.status == Status::FetchingIndex {
                    self.status = Status::Idle;
                }
            }
            Err(e) => {
                tracing::warn!("plok.org index: {e}");
                self.index_error = Some(e.clone());
                if self.status == Status::FetchingIndex {
                    self.status = Status::Error(e);
                }
            }
        }
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
    /// Refuse plain-HTTP URLs (redirects included). On for the real
    /// server; the tests' loopback server speaks HTTP and turns it off.
    pub https_only: bool,
    /// The per-read stall limit, [`READ_TIMEOUT`] by default. Tests
    /// shorten it to see a stalled transfer fail in milliseconds.
    pub read_timeout: Duration,
    /// The bound on the migration's index fetch,
    /// [`MIGRATION_INDEX_TIMEOUT`] by default.
    pub migration_index_timeout: Duration,
    /// Whether a download keeps going once nothing but the worker holds
    /// the library. `None`: it does not (it is abandoned, keeping its
    /// `.part`). The process-wide library answers "while any drum instance
    /// lives", so deleting the track whose editor started a download does
    /// not stop it.
    pub keep_alive: Option<KeepAliveFn>,
    /// Called with each extracted file's index before the next is
    /// extracted. Test hook: lets a test cancel (or drop the library, or
    /// panic) in the middle of an extraction.
    pub on_extract_entry: Option<ExtractHook>,
    /// Replaces the real free-space probe (bytes available at a path).
    /// Test hook for the disk-space refusal.
    pub free_space: Option<FreeSpaceFn>,
    /// Called with a thread's name before it is spawned; an error fails
    /// the spawn. Test hook for the spawn-failure rollback.
    pub spawn_hook: Option<SpawnHook>,
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

/// See [`WorkerConfig::keep_alive`].
#[doc(hidden)]
#[derive(Clone)]
pub struct KeepAliveFn(pub Arc<dyn Fn() -> bool + Send + Sync>);

impl std::fmt::Debug for KeepAliveFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeepAliveFn")
    }
}

/// See [`WorkerConfig::spawn_hook`].
#[doc(hidden)]
#[derive(Clone)]
pub struct SpawnHook(pub Arc<SpawnCheck>);

/// Called with a thread's name before it is spawned.
pub type SpawnCheck = dyn Fn(&str) -> std::io::Result<()> + Send + Sync;

impl std::fmt::Debug for SpawnHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SpawnHook")
    }
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            index_url: INDEX_URL.to_string(),
            https_only: true,
            read_timeout: READ_TIMEOUT,
            migration_index_timeout: MIGRATION_INDEX_TIMEOUT,
            keep_alive: None,
            on_extract_entry: None,
            free_space: None,
            spawn_hook: None,
        }
    }
}

impl WorkerConfig {
    fn http(&self) -> HttpConfig {
        HttpConfig {
            https_only: self.https_only,
            read_timeout: self.read_timeout,
        }
    }

    fn free_space_at(&self, path: &Path) -> Option<u64> {
        match &self.free_space {
            Some(f) => (f.0)(path),
            None => drumkit_library::free_space(path),
        }
    }

    fn keeps_alive(&self) -> bool {
        self.keep_alive.as_ref().is_some_and(|f| (f.0)())
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

    /// Spawn a named thread, through the test hook.
    fn spawn(
        &self,
        name: &str,
        f: impl FnOnce() + Send + 'static,
    ) -> std::io::Result<JoinHandle<()>> {
        if let Some(hook) = &self.spawn_hook {
            (hook.0)(name)?;
        }
        std::thread::Builder::new().name(name.to_string()).spawn(f)
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
    /// The worker thread, once the first download has started it.
    running: Mutex<Option<Running>>,
    flags: Arc<Flags>,
    /// Names this handle's `.part` files, so two handles (a detached
    /// worker and a re-opened library) never write one file.
    nonce: String,
    /// How many worker threads this handle has started (test hook: there
    /// is only ever one).
    threads_started: AtomicUsize,
    /// How many index fetches this handle has started.
    fetches_started: Arc<AtomicUsize>,
}

/// The flags the handle and its worker share.
#[derive(Default)]
struct Flags {
    /// Raised by `drop`; the worker checks it before each job.
    shutdown: AtomicBool,
    /// Stops the running job; checked every chunk. Raised by a cancel, by
    /// `drop`, and by the worker itself when the job is abandoned. Reset
    /// when a download starts.
    abort: AtomicBool,
    /// The user cancelled the running job (its `.part` goes). An abort
    /// without it is an abandon (its `.part` stays).
    cancelled: AtomicBool,
    /// True while the worker is handling a job.
    busy: AtomicBool,
}

/// A started worker thread and its wake-up channel.
struct Running {
    tx: Sender<Wake>,
    join: Option<JoinHandle<()>>,
}

enum Wake {
    Jobs,
    Shutdown,
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
            flags: Arc::new(Flags::default()),
            nonce: plok::new_nonce(),
            threads_started: AtomicUsize::new(0),
            fetches_started: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(crate) fn config(&self) -> &WorkerConfig {
        &self.config
    }

    /// Queue `cmd`, starting the worker thread for the first download.
    /// [`Command::Cancel`] acts at once instead, and
    /// [`Command::FetchIndex`] runs on its own thread. A download already
    /// running or queued is not queued twice — unless the running one is
    /// being cancelled, when the new one queues behind it.
    pub fn send(&self, cmd: Command) {
        match cmd {
            Command::Cancel(name) => self.cancel(&name),
            Command::FetchIndex => self.start_fetch(),
            Command::Download(kit) => self.queue(kit, None),
            Command::Redownload { kit, existing_dir } => self.queue(kit, Some(existing_dir)),
            Command::Shutdown => {
                if let Some(r) = self.running.lock().as_ref() {
                    let _ = r.tx.send(Wake::Shutdown);
                }
            }
        }
    }

    fn queue(&self, kit: ServerKit, existing_dir: Option<PathBuf>) {
        {
            let mut s = self.state.lock();
            let cancelling = self.flags.cancelled.load(Ordering::SeqCst);
            if s.status.active_kit() == Some(kit.name.as_str()) && !cancelling {
                return;
            }
            if let Some(job) = s.jobs.iter_mut().find(|j| j.kit.name == kit.name) {
                // Already queued: the newer entry wins, but a re-download's
                // target is never dropped by a plain Download.
                job.kit = kit;
                if existing_dir.is_some() {
                    job.existing_dir = existing_dir;
                }
                return;
            }
            s.jobs.push_back(Job {
                kit: kit.clone(),
                existing_dir,
            });
            s.refresh_queued();
        }
        if let Err(e) = self.wake() {
            let mut s = self.state.lock();
            s.remove_job(&kit.name);
            let msg = format!("start download worker: {e}");
            tracing::error!("{msg}");
            s.last_error = Some(msg.clone());
            s.status = Status::Error(msg);
        }
    }

    /// Tell the worker there is work, starting it first if need be.
    fn wake(&self) -> std::io::Result<()> {
        let mut running = self.running.lock();
        if let Some(r) = running.as_ref() {
            if r.tx.send(Wake::Jobs).is_ok() {
                return Ok(());
            }
            // The thread is gone (it can only end on a shutdown): start a
            // new one.
            *running = None;
        }
        let r = self.start()?;
        let _ = r.tx.send(Wake::Jobs);
        *running = Some(r);
        Ok(())
    }

    /// Cancel the download of `name`: abort it if it is running, drop it
    /// if it is queued.
    fn cancel(&self, name: &str) {
        let mut s = self.state.lock();
        if s.status.active_kit() == Some(name) {
            self.flags.cancelled.store(true, Ordering::SeqCst);
            self.flags.abort.store(true, Ordering::SeqCst);
        }
        s.remove_job(name);
    }

    /// Fetch the index on a thread of its own.
    fn start_fetch(&self) {
        {
            let mut s = self.state.lock();
            if s.fetching_index {
                s.fetch_again = true;
                return;
            }
            s.fetching_index = true;
            if !s.status.is_busy() {
                s.status = Status::FetchingIndex;
            }
        }
        let config = self.config.clone();
        let root = self.root.clone();
        let state = self.state.clone();
        let spawned = self
            .config
            .spawn("drums-index", move || fetch_loop(&config, root, &state));
        match spawned {
            Ok(_) => {
                self.fetches_started.fetch_add(1, Ordering::SeqCst);
            }
            Err(e) => {
                let mut s = self.state.lock();
                s.fetching_index = false;
                s.fetch_again = false;
                s.fetched(Err(format!("start index fetch: {e}")));
            }
        }
    }

    /// Whether the download worker thread has been started.
    pub fn is_running(&self) -> bool {
        self.running.lock().is_some()
    }

    /// How many worker threads this handle has ever started.
    #[doc(hidden)]
    pub fn threads_started(&self) -> usize {
        self.threads_started.load(Ordering::SeqCst)
    }

    /// How many index fetch threads this handle has ever started.
    #[doc(hidden)]
    pub fn fetches_started(&self) -> usize {
        self.fetches_started.load(Ordering::SeqCst)
    }

    /// The `.part` a download of `kit` by this handle streams into.
    #[doc(hidden)]
    pub fn part_path(&self, kit: &ServerKit) -> Option<PathBuf> {
        Some(plok::part_path(
            self.root.as_ref()?,
            &kit.target(&self.config),
            &self.nonce,
        ))
    }

    /// Fetch the index for the `installed.json` migration and cache it (in
    /// `state` and on disk), waiting at most `timeout` — the fetch itself
    /// is bounded by the same budget — and returning early once `cancel`
    /// is raised. Never joins: a fetch still running when this returns
    /// finishes on its own thread (and still caches what it got).
    pub(crate) fn fetch_index_now(
        &self,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        let config = self.config.clone();
        let root = self.root.clone();
        let state = self.state.clone();
        self.config
            .spawn("drums-index-migration", move || {
                let agent = make_agent(&config, Some(timeout));
                let result =
                    catch_unwind(AssertUnwindSafe(|| fetch_index(&agent, &config.index_url)))
                        .unwrap_or_else(|_| Err("index fetch panicked".to_string()));
                let outcome = result.as_ref().map(|_| ()).map_err(Clone::clone);
                if let Ok(index) = result {
                    if let Some(root) = &root {
                        write_index_cache(root, &index);
                    }
                    let mut s = state.lock();
                    s.index = Some(index);
                    s.index_fetched_at = Some(Instant::now());
                    s.index_error = None;
                }
                let _ = tx.send(outcome);
            })
            .map_err(|e| format!("start index fetch: {e}"))?;
        self.fetches_started.fetch_add(1, Ordering::SeqCst);
        // A little slack over the fetch's own budget, so its timeout error
        // (not ours) is what is reported.
        let deadline = Instant::now() + timeout + Duration::from_millis(250);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("cancelled".to_string());
            }
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(r) => return r,
                Err(RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!("no answer within {timeout:?}"));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err("the index fetch ended without an answer".to_string());
                }
            }
        }
    }

    fn start(&self) -> std::io::Result<Running> {
        let (tx, rx) = mpsc::channel();
        let worker = Worker {
            config: self.config.clone(),
            root: self.root.clone(),
            library: self.library.clone(),
            state: self.state.clone(),
            flags: self.flags.clone(),
            nonce: self.nonce.clone(),
        };
        let join = self
            .config
            .spawn("drums-download", move || worker_loop(rx, worker))?;
        self.threads_started.fetch_add(1, Ordering::SeqCst);
        Ok(Running {
            tx,
            join: Some(join),
        })
    }
}

impl Drop for WorkerHandle {
    /// Never blocks on the network. Closing a project during a 5 GiB
    /// download used to wait for the download to finish.
    ///
    /// While a download runs the worker holds the library, so this runs
    /// either with the worker idle — it sees the shutdown and returns at
    /// once, and the join is short — or on the worker thread itself, when
    /// it lets go of the library after a job; that never joins. The flags
    /// are raised before `busy` is read and the worker raises `busy`
    /// before it reads them (all `SeqCst`), so a worker that is busy all
    /// the same is detached rather than waited for.
    fn drop(&mut self) {
        self.flags.shutdown.store(true, Ordering::SeqCst);
        self.flags.abort.store(true, Ordering::SeqCst);
        let Some(mut running) = self.running.get_mut().take() else {
            return;
        };
        let _ = running.tx.send(Wake::Shutdown);
        if let Some(j) = running.join.take() {
            let on_worker = j.thread().id() == std::thread::current().id();
            if on_worker || self.flags.busy.load(Ordering::SeqCst) {
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
    flags: Arc<Flags>,
    nonce: String,
}

impl Worker {
    /// Why the running job must stop, if it must: the user cancelled it,
    /// the handle is shutting down, or nothing but this job holds the
    /// library any more (and nothing asks to keep it alive) — then the
    /// abort is raised here, so the extraction stops too.
    fn stop(&self, pin: &Arc<SharedKitLibrary>) -> Option<Failure> {
        if self.flags.cancelled.load(Ordering::SeqCst) {
            return Some(Failure::Cancelled);
        }
        if self.flags.shutdown.load(Ordering::SeqCst) || self.flags.abort.load(Ordering::SeqCst) {
            return Some(Failure::Abandoned);
        }
        if Arc::strong_count(pin) <= 1 && !self.config.keeps_alive() {
            self.flags.abort.store(true, Ordering::SeqCst);
            return Some(Failure::Abandoned);
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Worker loop
// ---------------------------------------------------------------------------

/// An agent with the per-read stall limit; `global` caps a whole call (the
/// index fetches), `None` leaves a multi-GiB download unbounded.
fn make_agent(config: &WorkerConfig, global: Option<Duration>) -> ureq::Agent {
    plok::http::agent(&config.http(), global)
}

fn fetch_index(agent: &ureq::Agent, index_url: &str) -> Result<ServerIndex, String> {
    plok::fetch_index(agent, index_url).map_err(|e| e.to_string())
}

/// The `drums-index` thread: fetch, and again while asked to.
fn fetch_loop(config: &WorkerConfig, root: Option<PathBuf>, state: &Arc<Mutex<State>>) {
    let agent = make_agent(config, Some(INDEX_FETCH_TIMEOUT));
    loop {
        let result = catch_unwind(AssertUnwindSafe(|| fetch_index(&agent, &config.index_url)))
            .unwrap_or_else(|_| Err("index fetch panicked".to_string()));
        if let (Ok(index), Some(root)) = (&result, &root) {
            write_index_cache(root, index);
        }
        let mut s = state.lock();
        s.fetched(result);
        if s.fetch_again {
            s.fetch_again = false;
            if !s.status.is_busy() {
                s.status = Status::FetchingIndex;
            }
            continue;
        }
        s.fetching_index = false;
        return;
    }
}

fn worker_loop(rx: Receiver<Wake>, worker: Worker) {
    // Leftovers of downloads nobody is running any more.
    if let Some(root) = &worker.root {
        plok::sweep_stale_downloads(root);
    }
    let agent = make_agent(&worker.config, None);
    while let Ok(Wake::Jobs) = rx.recv() {
        loop {
            // Raise `busy` before looking at the flags — the other half of
            // the ordering `WorkerHandle::drop` relies on.
            worker.flags.busy.store(true, Ordering::SeqCst);
            if worker.flags.shutdown.load(Ordering::SeqCst) {
                worker.flags.busy.store(false, Ordering::SeqCst);
                return;
            }
            let ran = catch_unwind(AssertUnwindSafe(|| run_next(&agent, &worker)));
            let more = match ran {
                Ok(more) => more,
                Err(panic) => {
                    let what = plok::panic_text(&*panic);
                    tracing::error!("drum kit download worker panicked: {what}");
                    set_error(&worker.state, &format!("download failed: {what}"));
                    true
                }
            };
            worker.flags.busy.store(false, Ordering::SeqCst);
            if !more {
                break;
            }
        }
    }
}

/// Take the next queued download and run it. `false` when there was none.
fn run_next(agent: &ureq::Agent, worker: &Worker) -> bool {
    let job = {
        let mut s = worker.state.lock();
        let Some(job) = s.jobs.pop_front() else {
            return false;
        };
        s.refresh_queued();
        // Popped, flags reset and status set under one lock, so a cancel
        // finds the job either queued or running, never in between.
        worker.flags.abort.store(false, Ordering::SeqCst);
        worker.flags.cancelled.store(false, Ordering::SeqCst);
        s.status = Status::Downloading {
            name: job.kit.name.clone(),
            downloaded_bytes: 0,
            total_bytes: job.kit.bytes.unwrap_or(0),
            bytes_per_sec: 0.0,
            eta_secs: None,
            resumed_from: 0,
        };
        job
    };
    // Held for the whole job: closing the editor that started it does not
    // stop it (see `Worker::stop`).
    let Some(pin) = worker.library.upgrade() else {
        worker.state.lock().status = Status::Cancelled(job.kit.name.clone());
        return true;
    };
    let name = job.kit.name.clone();
    let result = download_and_install(agent, &job, worker, &pin);
    {
        let mut s = worker.state.lock();
        match result {
            Ok(installed) => {
                s.status = Status::Done(name);
                s.last_error = None;
                s.record_install(installed);
            }
            Err(Failure::Cancelled) | Err(Failure::Abandoned) => {
                s.status = Status::Cancelled(name);
            }
            Err(Failure::Error(e)) => {
                tracing::warn!("drum kit download: {e}");
                s.last_error = Some(e.clone());
                s.status = Status::Error(e);
            }
        }
    }
    // A rescan skipped while this job held the library runs now.
    pin.run_wanted_rescan();
    drop(pin);
    true
}

fn set_error(state: &Arc<Mutex<State>>, msg: &str) {
    let mut s = state.lock();
    s.last_error = Some(msg.to_string());
    s.status = Status::Error(msg.to_string());
}

/// Why a download did not install.
#[derive(Debug)]
enum Failure {
    /// The user cancelled it: its `.part` goes.
    Cancelled,
    /// Nobody wants it any more (shutdown, or the library was let go of):
    /// its `.part` stays for a resume.
    Abandoned,
    Error(String),
}

impl From<String> for Failure {
    fn from(e: String) -> Self {
        Failure::Error(e)
    }
}

impl From<DownloadError> for Failure {
    fn from(e: DownloadError) -> Self {
        match e {
            DownloadError::Cancelled => Failure::Cancelled,
            DownloadError::Abandoned => Failure::Abandoned,
            DownloadError::Failed(e) => Failure::Error(e),
            DownloadError::ChecksumMismatch {
                name,
                actual,
                expected,
            } => Failure::Error(format!(
                "\"{name}\" failed verification: the download's sha256 is {actual}, the index \
                 says {expected}. Nothing was installed."
            )),
        }
    }
}

/// The worker's side of one running transfer: the stop conditions, the
/// progress the editor polls, and the free-space probe.
struct Transfer<'a> {
    worker: &'a Worker,
    pin: &'a Arc<SharedKitLibrary>,
    name: &'a str,
}

impl plok::Control for Transfer<'_> {
    fn stop(&self) -> Option<Stop> {
        self.worker.stop(self.pin).map(|f| match f {
            Failure::Cancelled => Stop::Cancelled,
            _ => Stop::Abandoned,
        })
    }

    fn progress(&self, p: plok::Progress) {
        self.worker.state.lock().status = Status::Downloading {
            name: self.name.to_string(),
            downloaded_bytes: p.downloaded_bytes,
            total_bytes: p.total_bytes,
            bytes_per_sec: p.bytes_per_sec,
            eta_secs: p.eta_secs,
            resumed_from: p.resumed_from,
        };
    }

    fn verifying(&self) {
        self.worker.state.lock().status = Status::Verifying(self.name.to_string());
    }

    fn free_space(&self, at: &Path) -> Option<u64> {
        self.worker.config.free_space_at(at)
    }
}

/// Download (adopting and resuming an unowned `.part` of the same file
/// when the server allows), verify and install.
fn download_and_install(
    agent: &ureq::Agent,
    job: &Job,
    worker: &Worker,
    pin: &Arc<SharedKitLibrary>,
) -> Result<Installed, Failure> {
    let kit = &job.kit;
    let root = worker
        .root
        .clone()
        .ok_or_else(|| "no drum kit library directory (no data dir)".to_string())?;
    let ctl = Transfer {
        worker,
        pin,
        name: &kit.name,
    };
    let part = plok::download(
        agent,
        &kit.target(&worker.config),
        &root,
        &worker.nonce,
        &ctl,
    )?;

    let result = match worker.stop(pin) {
        Some(stop) => Err(stop),
        None => install(pin, job, worker, part.path()),
    };
    // A complete zip whose install was abandoned is kept: the next
    // download of the kit finds it whole and only installs.
    match &result {
        Err(Failure::Abandoned) => part.keep(),
        _ => part.remove(),
    }
    result
}

/// Install the verified zip through the library. An update whose kit
/// comes back under a new id (its manifest changed) carries the old id's
/// marks over.
fn install(
    library: &Arc<SharedKitLibrary>,
    job: &Job,
    worker: &Worker,
    zip: &Path,
) -> Result<Installed, Failure> {
    let kit = &job.kit;
    let existing_dir = job.existing_dir.as_deref();
    let old_id = existing_dir.and_then(|d| library.read().by_dir(d).map(|e| e.id.clone()));
    let name = kit.name.clone();
    let state = worker.state.clone();
    let hook = worker.config.on_extract_entry.clone();
    let mut last_file = None;
    let progress = |p: drumkit_library::ImportProgress| {
        if let Some(h) = &hook {
            if last_file != Some(p.files_done) {
                last_file = Some(p.files_done);
                (h.0)(p.files_done as usize);
            }
        }
        // Raises the abort the extraction checks when the job is no
        // longer wanted.
        let _ = worker.stop(library);
        state.lock().status = Status::Extracting {
            name: name.clone(),
            files_done: p.files_done,
            files_total: p.files_total,
            bytes_done: p.bytes_done,
            bytes_total: p.bytes_total,
        };
    };
    let job_opts = ImportJob::new()
        .cancel(&worker.flags.abort)
        .progress(progress);
    let free = worker.config.free_space.clone();
    let import_job = match free {
        Some(f) => job_opts.free_space(move |p| (f.0)(p)),
        None => job_opts,
    };
    let sidecar = kit.sidecar();
    let outcome = match existing_dir {
        None => library
            .mutate(|lib| lib.install_zip(zip, &kit.name, sidecar, import_job))
            .map(|o| o.entry().clone()),
        Some(dir) => library.mutate(|lib| lib.install_zip_replacing(zip, dir, sidecar, import_job)),
    };
    let entry = outcome.map_err(|e| match e {
        drumkit_library::LibraryError::Cancelled => {
            worker.stop(library).unwrap_or(Failure::Cancelled)
        }
        e => Failure::Error(format!("install \"{}\": {e}", kit.name)),
    })?;
    if let Some(old) = old_id.filter(|old| *old != entry.id) {
        library.carry_marks(&old, &entry.id);
    }
    library.prune_marks();
    Ok(Installed {
        name: kit.name.clone(),
        id: entry.id,
        dir: entry.dir,
    })
}
