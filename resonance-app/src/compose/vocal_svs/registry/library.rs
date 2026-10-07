//! The voicebank library's download side (vocal-mcp-control-and-quality.md
//! §5): the plok.org index, and installs running on their own threads.
//!
//! - **Index.** Fetched on a short-lived thread (never on the update loop)
//!   with the shared `plok_download` client, cached in memory for
//!   [`INDEX_TTL`] and on disk under the user voicebank folder
//!   ([`INDEX_CACHE_FILE`]) for an offline Library tab. A hermetic test app
//!   has no index URL until a test gives it one, so no test reaches the
//!   network.
//! - **Installs.** One thread per install ([`super::install`]): fetch the
//!   index if it is stale, download the bank (resumable, sha256-verified),
//!   unpack it into the user voicebank folder, and fetch the vocoder its
//!   `dsconfig.yaml` names when it ships none and none is installed.
//!   Progress and the outcome land in the shared [`LibraryState`]; the
//!   update loop collects finished installs with
//!   [`VoicebankLibrary::take_finished`] (from the tick), rescans the
//!   registry, and only then resolves a control job — so a client that
//!   waited on `vocal.voicebank_install` finds the bank listed.
//! - **Concurrent installs.** Each install names its `.part` files with a
//!   nonce of its own, so two never share one. Two banks that need the
//!   same missing vocoder do not both fetch it: the first claims the
//!   vocoder id (`VoicebankLibrary::claim_vocoder`) and the second waits
//!   for it, then finds it installed.
//! - **Leftovers.** [`VoicebankLibrary::sweep_leftovers`] (at startup and
//!   when the overlay opens) clears what interrupted downloads, installs
//!   and deletes left in the voicebank and vocoder folders, and puts back
//!   a bank whose replace was cut between its two renames.
//! - **Ids.** An index entry whose id is not a plain folder name is
//!   dropped when the index is read, and an install or a Delete of such an
//!   id is refused ([`plok_download::is_safe_id`](resonance_common::plok_download::is_safe_id)).
//!
//! Nothing here touches the open project: no undo entry, no revision.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::BTreeSet;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use resonance_common::plok_download::package;
use resonance_common::plok_download::{self as plok, HttpConfig, INDEX_FETCH_TIMEOUT, INDEX_TTL};

pub use resonance_common::plok_download::{INDEX_CACHE_FILE, INDEX_URL};

use super::plok_index::PlokIndex;
use super::VoicebankRoots;
use crate::control_jobs::JobBoard;

/// Bytes available at a path; `None` when unknown (no check is made).
pub type FreeSpaceProbe = dyn Fn(&Path) -> Option<u64> + Send + Sync;

/// Where and how the library downloads. Tests point it at a local server.
#[derive(Clone)]
pub struct LibraryConfig {
    /// The index; `None` = no index (a hermetic test app's default).
    pub index_url: Option<String>,
    pub http: HttpConfig,
    /// Replaces the real free-space probe (test hook).
    pub free_space: Option<Arc<FreeSpaceProbe>>,
}

impl std::fmt::Debug for LibraryConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LibraryConfig")
            .field("index_url", &self.index_url)
            .field("http", &self.http)
            .finish_non_exhaustive()
    }
}

impl LibraryConfig {
    /// The real server.
    pub fn plok() -> Self {
        Self {
            index_url: Some(INDEX_URL.to_string()),
            http: HttpConfig::default(),
            free_space: None,
        }
    }

    /// No index at all: every fetch fails without touching the network.
    pub fn offline() -> Self {
        Self {
            index_url: None,
            http: HttpConfig::default(),
            free_space: None,
        }
    }
}

/// What an install is doing.
#[derive(Debug, Clone, PartialEq)]
pub enum InstallPhase {
    Starting,
    FetchingIndex,
    /// Waiting for another install that is fetching the same vocoder.
    WaitingFor(String),
    /// Downloading `what` (the bank's or the vocoder's name).
    Downloading {
        what: String,
        done: u64,
        /// 0 when unknown.
        total: u64,
    },
    Verifying(String),
    Extracting {
        what: String,
        done: u64,
        total: u64,
    },
}

impl InstallPhase {
    /// A 0..1 fraction for a progress bar, when one is known.
    pub fn fraction(&self) -> Option<f32> {
        match self {
            InstallPhase::Downloading { done, total, .. } if *total > 0 => {
                Some((*done as f64 / *total as f64) as f32 * 0.9)
            }
            InstallPhase::Verifying(_) => Some(0.9),
            InstallPhase::Extracting { done, total, .. } if *total > 0 => {
                Some(0.9 + (*done as f64 / *total as f64) as f32 * 0.1)
            }
            _ => None,
        }
    }

    /// One line for the GUI.
    pub fn label(&self) -> String {
        use resonance_common::drumkit_library::format_bytes;
        match self {
            InstallPhase::Starting => "Starting\u{2026}".into(),
            InstallPhase::FetchingIndex => "Reading the plok.org index\u{2026}".into(),
            InstallPhase::WaitingFor(what) => format!("Waiting for {what}\u{2026}"),
            InstallPhase::Downloading { what, done, total } if *total > 0 => format!(
                "Downloading {what}: {} of {}",
                format_bytes(*done),
                format_bytes(*total)
            ),
            InstallPhase::Downloading { what, done, .. } => {
                format!("Downloading {what}: {}", format_bytes(*done))
            }
            InstallPhase::Verifying(what) => format!("Verifying {what}\u{2026}"),
            InstallPhase::Extracting { what, .. } => format!("Unpacking {what}\u{2026}"),
        }
    }
}

/// What an install did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallOutcome {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub dir: PathBuf,
    pub terms_url: Option<String>,
    pub credit: Option<String>,
    pub commercial: Option<String>,
    /// The vocoder the bank renders with (its own, or a shared one).
    pub vocoder: Option<String>,
    pub vocoder_installed: bool,
    /// The shared vocoder was downloaded by this install.
    pub vocoder_fetched: bool,
    /// The bank was already installed as the index lists it: nothing was
    /// downloaded.
    pub already_installed: bool,
}

/// How an install failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallErrorKind {
    /// No index, or the index has no such bank.
    NotFound,
    /// The upstream file's sha256 is not the index's.
    UpstreamChanged,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallError {
    pub kind: InstallErrorKind,
    pub message: String,
}

impl InstallError {
    pub(super) fn new(kind: InstallErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

/// One install, running or finished but not yet collected.
#[derive(Debug, Clone)]
pub struct InstallJob {
    pub bank_id: String,
    pub name: String,
    pub phase: InstallPhase,
    pub cancel: Arc<AtomicBool>,
    /// The control job (`vocal.voicebank_install`) this install resolves.
    pub control_job: Option<u64>,
    pub result: Option<Result<InstallOutcome, InstallError>>,
}

/// The shared state the install threads write and the GUI reads.
#[derive(Debug, Default)]
pub struct LibraryState {
    pub index: Option<PlokIndex>,
    pub index_fetched_at: Option<Instant>,
    /// The last index fetch's error; `None` after one succeeds.
    pub index_error: Option<String>,
    pub fetching_index: bool,
    /// Running installs, and finished ones the tick has not collected yet
    /// (a bank can have one of each: a finished install whose control job
    /// still waits to be resolved, and a new one started before the tick).
    pub jobs: Vec<InstallJob>,
    /// Bumped on every change but an install's progress, so a view can
    /// cache what it derives.
    pub revision: u64,
    /// Bumped when a running install's phase or progress moves: a view
    /// patches that row's status rather than rebuilding everything.
    pub progress_revision: u64,
}

impl LibraryState {
    /// Bank `bank_id`'s install: the running one, else the newest finished.
    pub fn job(&self, bank_id: &str) -> Option<&InstallJob> {
        self.live_job(bank_id)
            .or_else(|| self.jobs.iter().rev().find(|j| j.bank_id == bank_id))
    }

    /// Bank `bank_id`'s running install.
    pub fn live_job(&self, bank_id: &str) -> Option<&InstallJob> {
        self.jobs
            .iter()
            .find(|j| j.bank_id == bank_id && j.result.is_none())
    }

    pub fn is_installing(&self, bank_id: &str) -> bool {
        self.live_job(bank_id).is_some()
    }

    fn index_is_fresh(&self) -> bool {
        self.index_fetched_at.is_some_and(|at| at.elapsed() < INDEX_TTL)
    }
}

struct Inner {
    config: Mutex<LibraryConfig>,
    state: Mutex<LibraryState>,
    /// The shared vocoders an install is fetching right now, by id; an
    /// install that needs one of them waits on `vocoder_free`.
    vocoders: Mutex<BTreeSet<String>>,
    vocoder_free: Condvar,
    /// Held by a leftover sweep while it runs; an install thread passes
    /// through it before it writes anything, so a sweep that found the
    /// library idle never removes a new install's staging folder.
    sweep_gate: Mutex<()>,
}

/// The voicebank library's downloads. Cheap to clone; every clone is the
/// same library.
#[derive(Clone)]
pub struct VoicebankLibrary {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for VoicebankLibrary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoicebankLibrary").finish_non_exhaustive()
    }
}

/// Lock, recovering from a poisoned mutex: an install thread that
/// panicked leaves the state consistent at every statement.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl VoicebankLibrary {
    pub fn new(config: LibraryConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                config: Mutex::new(config),
                state: Mutex::new(LibraryState::default()),
                vocoders: Mutex::new(BTreeSet::new()),
                vocoder_free: Condvar::new(),
                sweep_gate: Mutex::new(()),
            }),
        }
    }

    pub fn config(&self) -> LibraryConfig {
        lock(&self.inner.config).clone()
    }

    /// Replace the configuration (tests point it at a local server). The
    /// in-memory index is dropped with the old server.
    pub fn set_config(&self, config: LibraryConfig) {
        *lock(&self.inner.config) = config;
        let mut s = self.state();
        s.index = None;
        s.index_fetched_at = None;
        s.index_error = None;
        s.revision += 1;
    }

    /// Claim shared vocoder `id` for this install to fetch, waiting while
    /// another install holds it. `None` when `cancel` is set while
    /// waiting. The claim is released when dropped.
    pub(super) fn claim_vocoder(&self, id: &str, cancel: &AtomicBool) -> Option<VocoderClaim> {
        let mut held = lock(&self.inner.vocoders);
        while held.contains(id) {
            if cancel.load(Ordering::SeqCst) {
                return None;
            }
            held = self
                .inner
                .vocoder_free
                .wait_timeout(held, Duration::from_millis(100))
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        held.insert(id.to_owned());
        Some(VocoderClaim {
            lib: self.clone(),
            id: id.to_owned(),
        })
    }

    /// Whether another install holds a claim on vocoder `id`.
    pub(super) fn vocoder_claimed(&self, id: &str) -> bool {
        lock(&self.inner.vocoders).contains(id)
    }

    /// Wait for any running leftover sweep to finish (an install thread
    /// calls this before it writes anything).
    pub(super) fn pass_sweep_gate(&self) {
        drop(lock(&self.inner.sweep_gate));
    }

    /// Clear what interrupted downloads, installs and deletes left in the
    /// user voicebank and vocoder folders of `roots`, on a thread of its
    /// own (see [`Self::sweep_leftovers_now`]).
    pub fn sweep_leftovers(&self, roots: &VoicebankRoots) {
        let lib = self.clone();
        let roots = roots.clone();
        let spawned = std::thread::Builder::new()
            .name("voicebank-sweep".into())
            .spawn(move || lib.sweep_leftovers_now(&roots));
        if let Err(e) = spawned {
            tracing::warn!("start voicebank leftover sweep: {e}");
        }
    }

    /// [`Self::sweep_leftovers`] on this thread: download parts nobody
    /// holds ([`plok::sweep_stale_downloads`]), and staging / aside /
    /// deleting folders ([`package::sweep_install_leftovers`]) — this
    /// process's own only while no install runs, another process's once
    /// it is gone — restoring a bank whose replace was cut short.
    pub fn sweep_leftovers_now(&self, roots: &VoicebankRoots) {
        let _gate = lock(&self.inner.sweep_gate);
        let idle = !self.has_live_installs();
        for dir in [roots.banks.as_deref(), roots.vocoders.as_deref()]
            .into_iter()
            .flatten()
        {
            plok::sweep_stale_downloads(dir);
            package::sweep_install_leftovers(dir, idle);
        }
    }

    /// The shared state, locked. Hold it briefly: the install threads
    /// write it every chunk.
    pub fn state(&self) -> MutexGuard<'_, LibraryState> {
        lock(&self.inner.state)
    }

    pub fn revision(&self) -> u64 {
        self.state().revision
    }

    /// The index as last fetched, or as cached on disk under `banks`.
    pub fn index(&self, banks: Option<&Path>) -> Option<PlokIndex> {
        if let Some(index) = &self.state().index {
            return Some(index.clone());
        }
        // Read the cache with the lock released: the install threads
        // write the state every chunk.
        let cached = banks.and_then(plok::read_index_cache::<PlokIndex>)?;
        let mut s = self.state();
        if s.index.is_none() {
            s.index = Some(cached);
            s.revision += 1;
        }
        s.index.clone()
    }

    /// Fetch the index on a thread of its own when it is stale (or always,
    /// with `force`) and no fetch runs. `banks` is where it is cached.
    pub fn refresh_index(&self, banks: Option<PathBuf>, force: bool) {
        {
            let mut s = self.state();
            if s.fetching_index || (!force && s.index_is_fresh()) {
                return;
            }
            s.fetching_index = true;
            s.revision += 1;
        }
        let lib = self.clone();
        let spawned = std::thread::Builder::new()
            .name("voicebank-index".into())
            .spawn(move || {
                let result = lib.fetch_index_now(banks.as_deref());
                let mut s = lib.state();
                s.fetching_index = false;
                if let Err(e) = result {
                    s.index_error = Some(e);
                }
                s.revision += 1;
            });
        if let Err(e) = spawned {
            let mut s = self.state();
            s.fetching_index = false;
            s.index_error = Some(format!("start index fetch: {e}"));
        }
    }

    /// Fetch the index on this thread (an install's, or the index
    /// thread), store it and cache it under `banks`.
    pub(super) fn fetch_index_now(&self, banks: Option<&Path>) -> Result<PlokIndex, String> {
        let config = self.config();
        let Some(url) = config.index_url.clone() else {
            return Err("no voicebank index is configured".into());
        };
        let agent = plok::http::agent(&config.http, Some(INDEX_FETCH_TIMEOUT));
        let fetched = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            plok::fetch_index::<PlokIndex>(&agent, &url).map_err(|e| e.to_string())
        }))
        .unwrap_or_else(|_| Err("index fetch panicked".into()));
        let mut s = self.state();
        match &fetched {
            Ok(index) => {
                if let Some(banks) = banks {
                    if std::fs::create_dir_all(banks).is_ok() {
                        plok::write_index_cache(banks, index);
                    }
                }
                s.index = Some(index.clone());
                s.index_fetched_at = Some(Instant::now());
                s.index_error = None;
            }
            Err(e) => {
                tracing::warn!("voicebank index: {e}");
                s.index_error = Some(e.clone());
            }
        }
        s.revision += 1;
        fetched
    }

    /// The index for an install starting now: the in-memory one while
    /// fresh, else a fetch on this thread (falling back to the stale or
    /// cached one when the fetch fails).
    pub(super) fn index_for_install(&self, banks: Option<&Path>) -> Result<PlokIndex, String> {
        {
            let s = self.state();
            if s.index_is_fresh() {
                if let Some(index) = &s.index {
                    return Ok(index.clone());
                }
            }
        }
        match self.fetch_index_now(banks) {
            Ok(index) => Ok(index),
            Err(e) => self.index(banks).ok_or(e),
        }
    }

    /// Start installing bank `id` (and its vocoder) into the user folders
    /// of `roots`, on a thread of its own. `control` is the control job it
    /// resolves, with the board its progress goes to. Refused while the
    /// same bank is installing, and without a user voicebank folder.
    pub fn start_install(
        &self,
        id: &str,
        roots: VoicebankRoots,
        control: Option<(u64, Arc<JobBoard>)>,
    ) -> Result<(), String> {
        if roots.banks.is_none() {
            return Err("there is no user voicebank folder (no data directory)".into());
        }
        if !plok::is_safe_id(id) {
            return Err(format!("{id:?} is not a voicebank id"));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut s = self.state();
            if s.is_installing(id) {
                return Err(format!("voicebank {id:?} is already being installed"));
            }
            // A finished install the tick has not collected yet stays when
            // a control job waits on it: the tick still resolves that job.
            s.jobs
                .retain(|j| j.bank_id != id || j.control_job.is_some());
            let name = s
                .index
                .as_ref()
                .and_then(|i| i.voicebank(id))
                .map(|b| b.name.clone())
                .unwrap_or_else(|| id.to_string());
            s.jobs.push(InstallJob {
                bank_id: id.to_string(),
                name,
                phase: InstallPhase::Starting,
                cancel: cancel.clone(),
                control_job: control.as_ref().map(|(job, _)| *job),
                result: None,
            });
            s.revision += 1;
        }
        let lib = self.clone();
        let bank_id = id.to_string();
        let spawned = std::thread::Builder::new()
            .name("voicebank-install".into())
            .spawn(move || {
                let board = control.as_ref().map(|(job, board)| (*job, board.clone()));
                let nonce = plok::new_nonce();
                lib.pass_sweep_gate();
                let run = super::install::Install {
                    lib: &lib,
                    bank_id: &bank_id,
                    nonce: &nonce,
                    roots: &roots,
                    cancel: &cancel,
                    board: board.as_ref().map(|(job, b)| (*job, b.as_ref())),
                };
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run.run()))
                    .unwrap_or_else(|panic| {
                        Err(InstallError::new(
                            InstallErrorKind::Failed,
                            format!("install failed: {}", plok::panic_text(&*panic)),
                        ))
                    });
                lib.finish(&bank_id, result);
            });
        if let Err(e) = spawned {
            let mut s = self.state();
            s.jobs.retain(|j| j.bank_id != id || j.result.is_some());
            return Err(format!("start install: {e}"));
        }
        Ok(())
    }

    fn finish(&self, bank_id: &str, result: Result<InstallOutcome, InstallError>) {
        let mut s = self.state();
        if let Some(job) = s
            .jobs
            .iter_mut()
            .find(|j| j.bank_id == bank_id && j.result.is_none())
        {
            job.result = Some(result);
        }
        s.revision += 1;
    }

    /// Update a running install's phase.
    pub(super) fn set_phase(&self, bank_id: &str, phase: InstallPhase) {
        let mut s = self.state();
        if let Some(job) = s
            .jobs
            .iter_mut()
            .find(|j| j.bank_id == bank_id && j.result.is_none())
        {
            if job.phase != phase {
                job.phase = phase;
                s.progress_revision += 1;
            }
        }
    }

    /// Cancel the install of `bank_id`: it stops at the next chunk and
    /// leaves nothing behind.
    pub fn cancel(&self, bank_id: &str) {
        let s = self.state();
        if let Some(job) = s.live_job(bank_id) {
            job.cancel.store(true, Ordering::SeqCst);
        }
    }

    /// Whether any install is still running.
    pub fn has_live_installs(&self) -> bool {
        self.state().jobs.iter().any(|j| j.result.is_none())
    }

    /// Whether any install has finished and waits to be collected.
    pub fn has_finished(&self) -> bool {
        self.state().jobs.iter().any(|j| j.result.is_some())
    }

    /// Take the finished installs, oldest first.
    pub fn take_finished(&self) -> Vec<InstallJob> {
        let mut s = self.state();
        let (done, live): (Vec<_>, Vec<_>) = std::mem::take(&mut s.jobs)
            .into_iter()
            .partition(|j| j.result.is_some());
        s.jobs = live;
        if !done.is_empty() {
            s.revision += 1;
        }
        done
    }
}

/// A claim on a shared vocoder an install is fetching
/// (`VoicebankLibrary::claim_vocoder`); released when dropped.
pub(super) struct VocoderClaim {
    lib: VoicebankLibrary,
    id: String,
}

impl Drop for VocoderClaim {
    fn drop(&mut self) {
        lock(&self.lib.inner.vocoders).remove(&self.id);
        self.lib.inner.vocoder_free.notify_all();
    }
}

/// Delete the installed user bank in `dir`: a folder directly under the
/// user voicebank folder `banks` whose name is a safe id
/// ([`plok::is_safe_id`]). Refuses anything else.
pub fn delete_user_bank(banks: &Path, dir: &Path) -> Result<(), String> {
    let inside = dir.parent().is_some_and(|p| p == banks);
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !inside || !plok::is_safe_id(&name) || !dir.is_dir() {
        return Err(format!(
            "{} is not a bank in the voicebank folder",
            dir.display()
        ));
    }
    // Set it aside first, so a rescan mid-delete never half-sees it (and
    // a sweep finishes it if this process dies mid-delete).
    let trash = banks.join(package::aside_name(&name, package::Aside::Deleting));
    std::fs::rename(dir, &trash).map_err(|e| format!("delete {}: {e}", dir.display()))?;
    match std::fs::remove_dir_all(&trash) {
        Ok(()) => Ok(()),
        // A concurrent sweep took the rest.
        Err(_) if !trash.exists() => Ok(()),
        Err(e) => Err(format!("delete {}: {e}", dir.display())),
    }
}
