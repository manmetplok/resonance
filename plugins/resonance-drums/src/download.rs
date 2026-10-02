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
//! **Resume.** A transfer that fails on the network (a dropped
//! connection, a stall, a 5xx/408/429 answer) or is abandoned keeps its
//! `.part`, named after the index entry's *file* (plus a hash of its size
//! and sha256) and a per-handle random nonce:
//! `.<file>-<hash>.<nonce>.zip.part`, with a `.meta` beside it holding the
//! URL, the server's validators (ETag / Last-Modified) and the total. The
//! part is held under an exclusive advisory lock (`File::try_lock`) while
//! it is written; an unlocked part is nobody's, and the next download of
//! that kit — in this process or another — adopts it (renaming it to its
//! own nonce) and asks for `Range: bytes=<len>-` with `If-Range`. It
//! appends only on a 206 whose `Content-Range` starts at that length and
//! names the expected total; a 200 (the file changed) or a mismatch starts
//! over, and a 416 naming exactly the part's length means the part is
//! already whole. A part without a validator is never resumed.
//!
//! **Verify.** The sha256 is computed while the zip streams in (seeded
//! from the kept bytes on a resume), never by re-reading gigabytes.
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
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
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

/// How long name resolution and connecting may take.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The whole budget of one index fetch (resolve, connect, headers, body).
pub const INDEX_FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a rescan waits on the network for the index when the
/// `installed.json` migration is about to run without one.
pub const MIGRATION_INDEX_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a fetched index is fresh: opening the plok.org tab within this
/// of the last fetch does not fetch again (Refresh always does).
pub const INDEX_TTL: Duration = Duration::from_secs(10 * 60);

/// A download needs this much free space per byte of zip: the zip and the
/// extracted kit coexist until the zip is deleted (§4.1).
pub const DOWNLOAD_SPACE_FACTOR: f64 = 2.1;

/// The fetched index, cached under the library root (hidden, so a scan
/// never sees it) for the `installed.json` migration and an offline tab.
pub const INDEX_CACHE_FILE: &str = ".plok-index.json";

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
    let path = root.join(INDEX_CACHE_FILE);
    match serde_json::to_vec_pretty(index) {
        Ok(bytes) => {
            if let Err(e) = resonance_common::atomic_file::atomic_write(&path, &bytes) {
                tracing::warn!("plok.org index cache {}: {e}", path.display());
            }
        }
        Err(e) => tracing::warn!("plok.org index cache: serialize: {e}"),
    }
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
            nonce: new_nonce(),
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
        Some(own_part_path(self.root.as_ref()?, kit, &self.nonce))
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

/// A per-handle random tag: the hasher keys of `RandomState` are random
/// per process, the counter and the clock make it unique within one.
fn new_nonce() -> String {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    h.write_u32(std::process::id());
    if let Ok(t) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        h.write_u128(t.as_nanos());
    }
    format!("{:016x}", h.finish())
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
    let read_timeout = config.read_timeout;
    let agent_config = ureq::Agent::config_builder()
        .https_only(config.https_only)
        // Statuses are classified by hand: a 416 carries the information
        // a resume needs, and a 5xx keeps the `.part`.
        .http_status_as_error(false)
        .timeout_global(global)
        .timeout_resolve(Some(CONNECT_TIMEOUT))
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_send_request(Some(read_timeout))
        .timeout_recv_response(Some(read_timeout))
        // No total body budget for a download. A stalled body is caught
        // per read by `ReadTimeout` instead.
        .build();
    ureq::Agent::with_parts(
        agent_config,
        read_timeout::ReadTimeout::new(read_timeout),
        ureq::unversioned::resolver::DefaultResolver::default(),
    )
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
        sweep_stale_downloads(root);
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
                    let what = panic_text(&panic);
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

fn panic_text(panic: &Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "internal error".to_string())
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

// ---------------------------------------------------------------------------
// Network helpers
// ---------------------------------------------------------------------------

fn fetch_index(agent: &ureq::Agent, index_url: &str) -> Result<ServerIndex, String> {
    let mut resp = agent
        .get(index_url)
        .call()
        .map_err(|e| format!("fetch index: {e}"))?;
    let status = resp.status().as_u16();
    if status != 200 {
        return Err(format!("fetch index: the server answered {status}"));
    }
    let body = resp
        .body_mut()
        .read_to_vec()
        .map_err(|e| format!("read index: {e}"))?;
    parse_index(&body)
}

/// How a transfer failed.
enum Transfer {
    /// The connection failed or stalled, or the server is (for now)
    /// unwilling: the `.part` is kept for a resume.
    Network(String),
    Other(Failure),
}

impl From<Failure> for Transfer {
    fn from(f: Failure) -> Self {
        Transfer::Other(f)
    }
}

/// A non-success answer: a server error, a timeout or rate limit is worth
/// retrying (the `.part` stays); any other 4xx is a definite no.
fn status_failure(name: &str, code: u16) -> Transfer {
    let msg = format!("download {name}: the server answered {code}");
    if code >= 500 || code == 408 || code == 429 {
        Transfer::Network(msg)
    } else {
        Transfer::Other(Failure::Error(msg))
    }
}

fn request_failure(name: &str, e: ureq::Error) -> Transfer {
    let msg = format!("download {name}: {e}");
    match e {
        ureq::Error::StatusCode(code) => status_failure(name, code),
        ureq::Error::RequireHttpsOnly(_)
        | ureq::Error::BadUri(_)
        | ureq::Error::Http(_)
        | ureq::Error::InvalidProxyUrl => Transfer::Other(Failure::Error(msg)),
        _ => Transfer::Network(msg),
    }
}

fn io_failure(what: &str, path: &Path, e: std::io::Error) -> Failure {
    Failure::Error(format!("{what} {}: {e}", path.display()))
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
    std::fs::create_dir_all(&root).map_err(|e| io_failure("mkdir", &root, e))?;
    let url = worker.config.file_url(&kit.file);
    let mut part = PartFile::acquire(&root, kit, &url, &worker.nonce)?;

    // Refuse before a byte moves when the index says how big the zip is.
    if let Some(total) = kit.bytes {
        if let Err(e) = check_space(worker, &root, &kit.name, total, part.len()) {
            // Kept bytes stay for when there is room; an empty part goes.
            if part.len() == 0 {
                part.remove();
            } else {
                part.keep();
            }
            return Err(e);
        }
    }

    let hasher = match transfer(agent, kit, worker, pin, &root, &mut part) {
        Ok(h) => h,
        // A network failure or an abandon keeps the `.part` for a resume;
        // a cancel, a full disk or a definite refusal removes it.
        Err(Transfer::Network(e)) => {
            part.keep();
            return Err(Failure::Error(e));
        }
        Err(Transfer::Other(Failure::Abandoned)) => {
            part.keep();
            return Err(Failure::Abandoned);
        }
        Err(Transfer::Other(f)) => {
            part.remove();
            return Err(f);
        }
    };

    if let Some(expected) = &kit.sha256 {
        worker.state.lock().status = Status::Verifying(kit.name.clone());
        let actual = hex(&hasher.finalize());
        if !actual.eq_ignore_ascii_case(expected.trim()) {
            part.remove();
            return Err(Failure::Error(format!(
                "\"{}\" failed verification: the download's sha256 is {actual}, the index says \
                 {expected}. Nothing was installed.",
                kit.name
            )));
        }
    }

    let result = match worker.stop(pin) {
        Some(stop) => Err(stop),
        None => install(pin, job, worker, &part.path),
    };
    // A complete zip whose install was abandoned is kept: the next
    // download of the kit finds it whole and only installs.
    match &result {
        Err(Failure::Abandoned) => part.keep(),
        _ => part.remove(),
    }
    result
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

fn header(resp: &ureq::http::Response<ureq::Body>, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
}

/// `Content-Range: bytes <start>-<end>/<total>` or `bytes */<total>`:
/// (start, total), each `None` when absent or `*`.
fn content_range(resp: &ureq::http::Response<ureq::Body>) -> (Option<u64>, Option<u64>) {
    let Some(v) = header(resp, "Content-Range") else {
        return (None, None);
    };
    let Some(rest) = v.strip_prefix("bytes ") else {
        return (None, None);
    };
    let (range, total) = rest.split_once('/').unwrap_or((rest, "*"));
    let start = range
        .split_once('-')
        .and_then(|(s, _)| s.trim().parse().ok());
    (start, total.trim().parse().ok())
}

/// Stream the kit into `part`, resuming from what is there when the server
/// answers a validated Range request with a matching 206. Returns the
/// sha256 of the whole file, computed as it streamed.
fn transfer(
    agent: &ureq::Agent,
    kit: &ServerKit,
    worker: &Worker,
    pin: &Arc<SharedKitLibrary>,
    root: &Path,
    part: &mut PartFile,
) -> Result<Sha256, Transfer> {
    let url = part.meta.url.clone();
    let get = || {
        agent
            .get(&url)
            // A zip is not compressed again in transit, and a Range must
            // count bytes of the file itself.
            .header("Accept-Encoding", "identity")
    };
    let mut have = part.len();
    if have > 0 && part.meta.if_range().is_none() {
        // Nothing to tell the server which version these bytes are of.
        part.truncate()?;
        have = 0;
    }

    let mut resp = if have > 0 {
        let validator = part.meta.if_range().unwrap_or_default();
        get()
            .header("Range", format!("bytes={have}-"))
            .header("If-Range", validator)
            .call()
            .map_err(|e| request_failure(&kit.name, e))?
    } else {
        get().call().map_err(|e| request_failure(&kit.name, e))?
    };

    let mut start = 0;
    if have > 0 {
        let code = resp.status().as_u16();
        let (range_start, range_total) = content_range(&resp);
        let total_ok =
            |t: u64| kit.bytes.is_none_or(|b| b == t) && part.meta.total.is_none_or(|m| m == t);
        match code {
            // The part is already the whole file the server has.
            416 if range_total == Some(have) && kit.bytes.is_none_or(|b| b == have) => {
                let hasher = seed_hash(part, have, worker, pin)?;
                publish_progress(worker, kit, have, have, 0.0, None, have);
                return Ok(hasher);
            }
            206 if range_start == Some(have) && range_total.is_none_or(total_ok) => {
                start = have;
            }
            // 200 (the file changed: If-Range failed), 416 for another
            // size, or a 206 that does not continue these bytes: over.
            200 => {}
            c if c >= 400 && c != 416 => return Err(status_failure(&kit.name, c)),
            _ => {
                drop(resp);
                part.truncate()?;
                resp = get().call().map_err(|e| request_failure(&kit.name, e))?;
            }
        }
    }
    let code = resp.status().as_u16();
    if start == 0 && code != 200 {
        return Err(status_failure(&kit.name, code));
    }

    let content_length: Option<u64> = header(&resp, "Content-Length").and_then(|s| s.parse().ok());
    let total = content_length.map(|n| n + start).or(kit.bytes).unwrap_or(0);
    if kit.bytes.is_none() && total > 0 {
        check_space(worker, root, &kit.name, total, start)?;
    }

    let mut hasher = if start > 0 {
        seed_hash(part, start, worker, pin)?
    } else {
        part.truncate()?;
        Sha256::new()
    };
    // What the next resume validates against.
    if start == 0 {
        part.meta.etag = header(&resp, "ETag");
        part.meta.last_modified = header(&resp, "Last-Modified");
    }
    part.meta.total = (total > 0).then_some(total);
    part.write_meta()?;

    let mut reader = resp.into_body().into_reader();
    let mut buf = vec![0u8; 256 * 1024];
    let mut downloaded = start;
    let mut rate = Rate::new(start);
    let file = part.file();
    loop {
        if let Some(stop) = worker.stop(pin) {
            return Err(Transfer::Other(stop));
        }
        let n = match reader.read(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                // The abort can surface as a read error of a dropped
                // connection; report it as what it is.
                if let Some(stop) = worker.stop(pin) {
                    return Err(Transfer::Other(stop));
                }
                return Err(Transfer::Network(format!("read body: {e}")));
            }
        };
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| Transfer::Other(Failure::Error(format!("write part file: {e}"))))?;
        hasher.update(&buf[..n]);
        downloaded += n as u64;
        let (bytes_per_sec, eta_secs) = rate.update(downloaded, total);
        publish_progress(
            worker,
            kit,
            downloaded,
            total,
            bytes_per_sec,
            eta_secs,
            start,
        );
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
    Ok(hasher)
}

fn publish_progress(
    worker: &Worker,
    kit: &ServerKit,
    downloaded: u64,
    total: u64,
    bytes_per_sec: f64,
    eta_secs: Option<f64>,
    resumed_from: u64,
) {
    worker.state.lock().status = Status::Downloading {
        name: kit.name.clone(),
        downloaded_bytes: downloaded,
        total_bytes: total,
        bytes_per_sec,
        eta_secs,
        resumed_from,
    };
}

/// The sha256 state after the first `len` bytes of the part (a resume
/// continues it), checking the stop conditions every chunk. Leaves the
/// file positioned at `len`, truncated to it.
fn seed_hash(
    part: &mut PartFile,
    len: u64,
    worker: &Worker,
    pin: &Arc<SharedKitLibrary>,
) -> Result<Sha256, Transfer> {
    let path = part.path.clone();
    let err = |what: &str, e: std::io::Error| Transfer::Other(io_failure(what, &path, e));
    let file = part.file();
    file.set_len(len).map_err(|e| err("truncate", e))?;
    file.seek(SeekFrom::Start(0)).map_err(|e| err("seek", e))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut left = len;
    while left > 0 {
        if let Some(stop) = worker.stop(pin) {
            return Err(Transfer::Other(stop));
        }
        let want = buf.len().min(left as usize);
        let n = file.read(&mut buf[..want]).map_err(|e| err("read", e))?;
        if n == 0 {
            return Err(Transfer::Other(Failure::Error(format!(
                "read {}: shorter than expected",
                path.display()
            ))));
        }
        hasher.update(&buf[..n]);
        left -= n as u64;
    }
    file.seek(SeekFrom::Start(len))
        .map_err(|e| err("seek", e))?;
    Ok(hasher)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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

// ---------------------------------------------------------------------------
// Part files
// ---------------------------------------------------------------------------

/// Suffix of the file a download streams into.
const PART_SUFFIX: &str = ".zip.part";
/// Suffix of a part's validator record, beside it.
const META_SUFFIX: &str = ".meta";
/// Suffix of the directory the pre-library worker extracted into before
/// moving it into place. Nothing writes these any more; leftovers are
/// swept.
const STAGING_SUFFIX: &str = ".extracting";

/// An unlocked part (nobody is writing it) is removed after this long
/// without a write: a resume an hour later is not worth keeping gigabytes
/// around for.
const STALE_DOWNLOAD_AGE: Duration = Duration::from_secs(60 * 60);

/// A `.meta` with no part beside it is removed after this long (a part is
/// created right after its meta).
const ORPHAN_META_AGE: Duration = Duration::from_secs(60);

/// What a `.part` is a download of, and how to tell the server which
/// version of the file its bytes are.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
struct PartMeta {
    url: String,
    file: String,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    etag: Option<String>,
    #[serde(default)]
    last_modified: Option<String>,
    /// The whole file's size, once a server (or the index) said.
    #[serde(default)]
    total: Option<u64>,
    /// The handle writing it (informational: liveness is the lock).
    #[serde(default)]
    nonce: String,
}

impl PartMeta {
    fn new(kit: &ServerKit, url: &str, nonce: &str) -> Self {
        Self {
            url: url.to_string(),
            file: kit.file.clone(),
            sha256: kit.sha256.as_ref().map(|s| s.trim().to_ascii_lowercase()),
            etag: None,
            last_modified: None,
            total: kit.bytes,
            nonce: nonce.to_string(),
        }
    }

    /// The `If-Range` validator: a strong ETag, else Last-Modified.
    fn if_range(&self) -> Option<String> {
        self.etag
            .clone()
            .filter(|e| !e.starts_with("W/"))
            .or_else(|| self.last_modified.clone())
    }

    /// Whether a kept part with this record can continue a download of
    /// `kit` from `url`.
    fn continues(&self, kit: &ServerKit, url: &str) -> bool {
        let sha = kit.sha256.as_ref().map(|s| s.trim().to_ascii_lowercase());
        self.url == url
            && self.file == kit.file
            && (sha.is_none() || self.sha256 == sha)
            && kit.bytes.is_none_or(|b| self.total.is_none_or(|t| t == b))
            && self.if_range().is_some()
    }
}

/// `.<file>-<hash>`: the index entry's file name (not the display name —
/// "Kit (A)" and "Kit A" sanitize alike) and a hash of the file, its size
/// and its sha256, so a new version of the file never continues an old
/// part.
fn part_stem(kit: &ServerKit) -> String {
    let file = kit.file.trim();
    let base = file.rsplit('/').next().unwrap_or(file);
    let base = base.strip_suffix(".zip").unwrap_or(base);
    let mut h = Sha256::new();
    h.update(file.as_bytes());
    h.update([0]);
    h.update(kit.bytes.unwrap_or(0).to_le_bytes());
    h.update([0]);
    h.update(
        kit.sha256
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
            .as_bytes(),
    );
    let digest = hex(&h.finalize());
    format!(".{}-{}", sanitize(base), &digest[..12])
}

fn own_part_path(root: &Path, kit: &ServerKit, nonce: &str) -> PathBuf {
    root.join(format!("{}.{nonce}{PART_SUFFIX}", part_stem(kit)))
}

fn meta_path_of(part: &Path) -> PathBuf {
    let mut s = part.as_os_str().to_owned();
    s.push(META_SUFFIX);
    PathBuf::from(s)
}

fn read_meta(path: &Path) -> Option<PartMeta> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Whether `file` is now locked by us.
enum Lock {
    Held,
    Busy,
}

fn try_lock(file: &File) -> std::io::Result<Lock> {
    match file.try_lock() {
        Ok(()) => Ok(Lock::Held),
        Err(std::fs::TryLockError::WouldBlock) => Ok(Lock::Busy),
        // No locks on this filesystem (NFS without lockd, some FUSE): go
        // on unlocked, as the library and the marks store do.
        Err(std::fs::TryLockError::Error(e)) if library_marks::lock_error_is_unsupported(&e) => {
            Ok(Lock::Held)
        }
        Err(std::fs::TryLockError::Error(e)) => Err(e),
    }
}

/// The `.part` this job writes, open and locked for as long as it lives.
struct PartFile {
    path: PathBuf,
    meta_path: PathBuf,
    file: Option<File>,
    meta: PartMeta,
}

impl PartFile {
    /// Adopt an unowned part of the same file (renamed to this handle's
    /// name), or start a new one. Parts of the file that are locked belong
    /// to a live download and are left alone; unlocked ones that cannot
    /// continue this download are removed.
    fn acquire(root: &Path, kit: &ServerKit, url: &str, nonce: &str) -> Result<Self, Failure> {
        let ours = own_part_path(root, kit, nonce);
        let prefix = format!("{}.", part_stem(kit));
        let mut candidates: Vec<PathBuf> = std::fs::read_dir(root)
            .map(|d| {
                d.flatten()
                    .filter(|e| {
                        let n = e.file_name();
                        let n = n.to_string_lossy();
                        n.starts_with(&prefix) && n.ends_with(PART_SUFFIX)
                    })
                    .map(|e| e.path())
                    .collect()
            })
            .unwrap_or_default();
        // Our own first: it is the likeliest to continue.
        candidates.sort_by_key(|p| *p != ours);

        let ours_meta = meta_path_of(&ours);
        let mut adopted = None;
        for path in candidates {
            let Ok(file) = OpenOptions::new().read(true).write(true).open(&path) else {
                continue;
            };
            if !matches!(try_lock(&file), Ok(Lock::Held)) {
                continue;
            }
            let meta_path = meta_path_of(&path);
            let meta = read_meta(&meta_path).filter(|m| m.continues(kit, url));
            match meta {
                Some(meta) if adopted.is_none() => {
                    if path != ours {
                        if std::fs::rename(&path, &ours).is_err() {
                            // Taken from under us; leave it.
                            continue;
                        }
                        let _ = std::fs::rename(&meta_path, &ours_meta);
                    }
                    adopted = Some((file, meta));
                }
                _ => {
                    remove_logged(&path);
                    remove_logged(&meta_path);
                }
            }
        }

        let (file, mut meta) = match adopted {
            Some(found) => found,
            None => {
                let meta = PartMeta::new(kit, url, nonce);
                // The record first: a sweep never takes a fresh part with
                // a record for an abandoned one.
                write_meta_file(&ours_meta, &meta)?;
                let file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&ours)
                    .map_err(|e| io_failure("create", &ours, e))?;
                match try_lock(&file) {
                    Ok(Lock::Held) => {}
                    Ok(Lock::Busy) => {
                        return Err(Failure::Error(format!(
                            "{} is locked by another download",
                            ours.display()
                        )))
                    }
                    Err(e) => return Err(io_failure("lock", &ours, e)),
                }
                (file, meta)
            }
        };
        meta.nonce = nonce.to_string();
        let part = Self {
            path: ours,
            meta_path: ours_meta,
            file: Some(file),
            meta,
        };
        part.write_meta()?;
        Ok(part)
    }

    fn file(&mut self) -> &mut File {
        self.file
            .as_mut()
            .expect("the part file is open until dropped")
    }

    fn len(&self) -> u64 {
        self.file
            .as_ref()
            .and_then(|f| f.metadata().ok())
            .map(|m| m.len())
            .unwrap_or(0)
    }

    /// Start the file over (the server's copy is another, or the part had
    /// no validator).
    fn truncate(&mut self) -> Result<(), Failure> {
        let path = self.path.clone();
        let file = self.file();
        file.set_len(0)
            .map_err(|e| io_failure("truncate", &path, e))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|e| io_failure("seek", &path, e))?;
        self.meta.etag = None;
        self.meta.last_modified = None;
        Ok(())
    }

    fn write_meta(&self) -> Result<(), Failure> {
        write_meta_file(&self.meta_path, &self.meta)
    }

    /// Keep the part (and its record) for a resume; the lock goes.
    fn keep(mut self) {
        if let Some(f) = self.file.take() {
            let _ = f.sync_data();
        }
    }

    /// Delete the part and its record.
    fn remove(mut self) {
        remove_logged(&self.path);
        remove_logged(&self.meta_path);
        self.file.take();
    }
}

fn write_meta_file(path: &Path, meta: &PartMeta) -> Result<(), Failure> {
    let bytes = serde_json::to_vec(meta).map_err(|e| format!("part record: {e}"))?;
    resonance_common::atomic_file::atomic_write(path, &bytes)
        .map_err(|e| Failure::Error(format!("write {}: {e}", path.display())))
}

fn remove_logged(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!("remove download leftover {}: {e}", path.display()),
    }
}

fn age_of(meta: &std::fs::Metadata) -> Option<Duration> {
    meta.modified().ok().and_then(|t| t.elapsed().ok())
}

/// Remove download leftovers no running download can own:
///
/// - a `.part` nobody holds the lock of, when it has no record (it cannot
///   be resumed) or has not been written for [`STALE_DOWNLOAD_AGE`] —
///   never one that is locked (a live download, in any process);
/// - a record whose part is gone;
/// - the pre-library worker's `.extracting` directories.
fn sweep_stale_downloads(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if name.ends_with(STAGING_SUFFIX) {
            if let Err(e) = std::fs::remove_dir_all(&path) {
                tracing::warn!("remove download leftover {}: {e}", path.display());
            }
        } else if name.ends_with(PART_SUFFIX) {
            let Ok(file) = OpenOptions::new().read(true).write(true).open(&path) else {
                continue;
            };
            if !matches!(try_lock(&file), Ok(Lock::Held)) {
                continue;
            }
            let meta_path = meta_path_of(&path);
            let stale = file
                .metadata()
                .ok()
                .and_then(|m| age_of(&m))
                .is_some_and(|age| age > STALE_DOWNLOAD_AGE);
            if stale || !meta_path.exists() {
                remove_logged(&path);
                remove_logged(&meta_path);
            }
        } else if let Some(part) = name.strip_suffix(META_SUFFIX) {
            if part.ends_with(PART_SUFFIX) && !dir.join(part).exists() {
                let old = entry
                    .metadata()
                    .ok()
                    .and_then(|m| age_of(&m))
                    .is_some_and(|age| age > ORPHAN_META_AGE);
                if old {
                    remove_logged(&path);
                }
            }
        }
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
