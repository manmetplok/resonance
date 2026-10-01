//! The process-wide drum-kit library every drums instance shares
//! (drums-plugin-rework.md §3.4): one [`Library`] per (library root,
//! marks dir) behind a read/write lock, the marks store, and the one
//! plok.org download worker, handed out as an `Arc` so every editor sees
//! the same index, the same favourites and the same download. Modelled on
//! `resonance-amp/src/library.rs`.
//!
//! **Threads.** Nothing here is touched by `process()`. Readers take the
//! read lock only for as long as they copy what they need. Every mutation
//! — rescan, import, delete, an install from the download worker — does
//! its file I/O on a clone with no lock held and swaps the result in; a
//! writer mutex that no reader takes keeps two in-process mutations from
//! racing each other's swap. The editor's own mutations use
//! [`SharedKitLibrary::try_mutate`]: an editor never queues behind a
//! multi-gigabyte install, and so an editor closing never waits on one.
//!
//! **A skipped rescan is not lost.** A rescan that finds another writer
//! busy leaves a "rescan wanted" flag; whoever holds the library runs it
//! when they let go (an install, an import, a delete), so changes made
//! outside the process meanwhile are still picked up.
//!
//! **Opening is lazy.** The library is opened on first use — an editor,
//! or an instance's `kit_select` (its text, a name to parse, a state's
//! kit reference). The first open reads the index as the last scan left
//! it and scans once on a thread of its own, since with no editor open
//! nothing else would; a name lookup that misses re-reads the index in
//! case another process rewrote it. Headless builds have no library.
//!
//! **Usage.** Which kit each live instance is playing — or is loading,
//! and so will reload — is read straight from the instances' kit bridges
//! ([`register_bridge`]), so "used in N open drum instances" counts
//! instances whose editors are closed too.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, OnceLock, Weak};

use parking_lot::{Mutex, RwLock, RwLockReadGuard};
use resonance_common::drumkit_library::{
    self, Entry, ImportJob, ImportOutcome, InIndexFn, Library, LibraryError, ScanReport, Source,
};
use resonance_common::library_marks::{self, Marks, MarksError, SharedMarks};

use crate::download::{
    self, KeepAliveFn, ServerKit, State as DownloadState, WorkerConfig, WorkerHandle,
};

// ---------------------------------------------------------------------------
// Instance usage
// ---------------------------------------------------------------------------

type KitPathCell = Mutex<Option<PathBuf>>;
type PendingKitCell = Mutex<Option<(u64, PathBuf)>>;

/// One live instance's kit cells, held weakly.
struct Instance {
    kit_path: Weak<KitPathCell>,
    pending: Option<Weak<PendingKitCell>>,
}

impl Instance {
    fn alive(&self) -> bool {
        self.kit_path.strong_count() > 0
    }

    /// Whether this instance plays, loads or will reload a kit under `dir`.
    fn uses(&self, dir: &Path) -> bool {
        let under = |p: &Path| p.starts_with(dir);
        let loaded = self
            .kit_path
            .upgrade()
            .is_some_and(|c| c.lock().as_deref().is_some_and(under));
        let pending = self
            .pending
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some_and(|c| c.lock().as_ref().is_some_and(|(_, p)| under(p)));
        loaded || pending
    }
}

fn instances() -> &'static Mutex<Vec<Instance>> {
    static INSTANCES: OnceLock<Mutex<Vec<Instance>>> = OnceLock::new();
    INSTANCES.get_or_init(|| Mutex::new(Vec::new()))
}

fn register(kit_path: &Arc<KitPathCell>, pending: Option<&Arc<PendingKitCell>>) {
    let mut all = instances().lock();
    all.retain(Instance::alive);
    let pending = pending.map(Arc::downgrade);
    if let Some(known) = all
        .iter_mut()
        .find(|i| std::ptr::eq(i.kit_path.as_ptr(), Arc::as_ptr(kit_path)))
    {
        if pending.is_some() {
            known.pending = pending;
        }
        return;
    }
    all.push(Instance {
        kit_path: Arc::downgrade(kit_path),
        pending,
    });
}

/// Count a plugin instance in [`instances_playing`] by its bridge's
/// "loaded manifest" cell alone. Prefer [`register_bridge`], which also
/// counts the kit a load is in flight for. Held weakly; a dropped
/// instance stops counting by itself.
pub fn register_instance(kit_path: &Arc<KitPathCell>) {
    register(kit_path, None);
}

/// Count a plugin instance in [`instances_playing`]: the kit it has
/// loaded and the kit a load is in flight for (what
/// [`crate::KitBridge::wanted_kit_path`] reloads). Registering the same
/// bridge again (or after [`register_instance`]) does not count it twice.
pub fn register_bridge(bridge: &crate::KitBridge) {
    register(&bridge.kit_path, Some(&bridge.pending_kit));
}

/// How many live instances in this process are playing, loading or about
/// to reload a kit under `dir`.
pub fn instances_playing(dir: &Path) -> usize {
    let mut all = instances().lock();
    all.retain(Instance::alive);
    all.iter().filter(|i| i.uses(dir)).count()
}

/// How many drum instances this process has.
pub fn live_instances() -> usize {
    let mut all = instances().lock();
    all.retain(Instance::alive);
    all.len()
}

// ---------------------------------------------------------------------------
// The shared library
// ---------------------------------------------------------------------------

pub struct SharedKitLibrary {
    root: Option<PathBuf>,
    lib: RwLock<Library>,
    /// Serialises clone-mutate-swap writers inside this process.
    writer: Mutex<()>,
    /// Bumped on every in-process change of the entries, so editors know
    /// when to rebuild their rows.
    revision: AtomicU64,
    /// A rescan found the writer busy: the writer runs it when done.
    rescan_wanted: AtomicBool,
    /// Favourites, tags and recents (`library_marks`, kind `drumkit`).
    marks: SharedMarks,
    /// The retired `installed.json`, the one-time migration's source.
    installed_json: Option<PathBuf>,
    /// Whether the migration has been handed to the library (it then runs
    /// on the next scan, once). It is held back while the index is not at
    /// hand, so kits are not marked `local` for want of a network.
    migration_attached: AtomicBool,
    /// How many times this process fetched the index for the migration.
    migration_fetches: AtomicU8,
    /// The one plok.org download worker.
    download: WorkerHandle,
}

/// Where a library lives, and the worker it builds.
#[doc(hidden)]
#[derive(Clone, Debug, Default)]
pub struct Roots {
    /// The library root (`drumkits/`). `None`: no data dir — an empty
    /// library whose writes fail.
    pub root: Option<PathBuf>,
    /// The marks directory. `None`: a detached store whose writes fail.
    pub marks_dir: Option<PathBuf>,
    /// `installed.json` to migrate on the first scan. `None`: none.
    pub installed_json: Option<PathBuf>,
    pub worker: WorkerConfig,
}

type Key = (Option<PathBuf>, Option<PathBuf>);
type Registry = Mutex<Vec<(Key, Weak<SharedKitLibrary>)>>;

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Vec::new()))
}

static ROOT_OVERRIDE: OnceLock<Roots> = OnceLock::new();

/// The roots [`shared`] uses: a test override when one was installed,
/// else the platform's. The process-wide library's downloads outlive the
/// editor that started them while the process has drum instances.
fn default_roots() -> Roots {
    // A build with the test hooks is a test build: every drums instance
    // opens the library lazily (its `kit_select` text, a state's kit
    // reference), and no test may read or write the user's data dir.
    #[cfg(feature = "test-hooks")]
    test_hooks::isolate_for_tests();
    if let Some(roots) = ROOT_OVERRIDE.get() {
        return roots.clone();
    }
    Roots {
        root: drumkit_library::default_root(),
        marks_dir: library_marks::default_library_dir(),
        installed_json: drumkit_library::default_installed_json(),
        worker: WorkerConfig {
            keep_alive: Some(KeepAliveFn(Arc::new(|| live_instances() > 0))),
            ..WorkerConfig::default()
        },
    }
}

// The editor's own test entry points (`TestEditor`, `test_render_editor_frame`)
// are the only callers, and they are themselves gated behind `test-hooks`
// (`editor/mod.rs`) — so a build without it carries none of this module.
#[cfg(feature = "test-hooks")]
pub use test_hooks::{isolate_for_tests, override_default_roots};

#[cfg(feature = "test-hooks")]
mod test_hooks {
    use std::fs::File;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;
    use std::time::Duration;

    use super::{Roots, ROOT_OVERRIDE};
    use crate::download::WorkerConfig;

    /// Point [`super::shared`] at `roots` for the rest of this process —
    /// for tests, which must never read or write the user's data dir.
    /// Also keeps the preset bar's library beside it. No environment
    /// variable, so no `setenv` race with the test harness's threads.
    /// First call wins.
    #[doc(hidden)]
    pub fn override_default_roots(roots: Roots) {
        if let Some(root) = &roots.root {
            let presets = root
                .parent()
                .map(|p| p.join("plugin-presets"))
                .unwrap_or_else(|| root.join("plugin-presets"));
            resonance_plugin::presets::override_default_roots(presets, roots.marks_dir.clone());
        }
        let _ = ROOT_OVERRIDE.set(roots);
    }

    const TEST_DIR_PREFIX: &str = "resonance-drums-test-";
    /// Held locked by the process that owns a test dir, for its lifetime.
    const ALIVE_FILE: &str = ".alive";

    /// Isolate this process from the user's data dir: the default library
    /// lives under a fresh temp directory with no `installed.json`, and
    /// its worker fetches from a closed loopback port. Idempotent; returns
    /// the library root. Every editor test hook calls it, so no test
    /// frame can scan (or migrate) the real library.
    ///
    /// The directory is per process and held by a lock on its `.alive`
    /// file until the process exits; the next process to isolate itself
    /// removes every such directory whose lock is free.
    #[doc(hidden)]
    pub fn isolate_for_tests() -> PathBuf {
        static BASE: OnceLock<(PathBuf, Option<File>)> = OnceLock::new();
        let (base, _alive) = BASE.get_or_init(|| {
            let tmp = std::env::temp_dir();
            sweep_dead_test_dirs(&tmp);
            let base = tmp.join(format!("{TEST_DIR_PREFIX}{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&base);
            let _ = std::fs::create_dir_all(&base);
            let alive = File::create(base.join(ALIVE_FILE))
                .ok()
                .filter(|f| f.try_lock().is_ok());
            override_default_roots(Roots {
                root: Some(base.join("drumkits")),
                marks_dir: Some(base.join("library")),
                installed_json: None,
                worker: WorkerConfig {
                    index_url: format!("http://127.0.0.1:{}/index.json", closed_port()),
                    https_only: false,
                    ..WorkerConfig::default()
                },
            });
            (base, alive)
        });
        match ROOT_OVERRIDE.get().and_then(|r| r.root.clone()) {
            Some(root) => root,
            None => base.join("drumkits"),
        }
    }

    /// A loopback port nothing listens on: bound, then let go.
    fn closed_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map(|a| a.port())
            .unwrap_or(9)
    }

    /// Remove the test dirs of processes that are gone: their `.alive`
    /// lock is free (or, for a dir without one, it is an hour old).
    fn sweep_dead_test_dirs(tmp: &Path) {
        let Ok(entries) = std::fs::read_dir(tmp) else {
            return;
        };
        for entry in entries.flatten() {
            if !entry
                .file_name()
                .to_string_lossy()
                .starts_with(TEST_DIR_PREFIX)
            {
                continue;
            }
            let dir = entry.path();
            let age = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .unwrap_or_default();
            let dead = match File::open(dir.join(ALIVE_FILE)) {
                // Not brand new: its owner may be between creating the
                // file and locking it.
                Ok(f) => age > Duration::from_secs(10) && f.try_lock().is_ok(),
                Err(_) => age > Duration::from_secs(3600),
            };
            if dead {
                let _ = std::fs::remove_dir_all(&dir);
            }
        }
    }
}

/// The shared library at the default roots, opened on first use and
/// shared by every caller while any of them holds it.
pub fn shared() -> Arc<SharedKitLibrary> {
    shared_for(default_roots())
}

/// The shared library for `roots`: the same instance for the same (root,
/// marks dir) while anyone holds it. A new one is built with `roots`'
/// `installed_json` and worker config.
pub fn shared_for(roots: Roots) -> Arc<SharedKitLibrary> {
    let key = (roots.root.clone(), roots.marks_dir.clone());
    let mut reg = registry().lock();
    reg.retain(|(_, w)| w.strong_count() > 0);
    if let Some(lib) = reg
        .iter()
        .find(|(k, _)| *k == key)
        .and_then(|(_, w)| w.upgrade())
    {
        return lib;
    }
    let lib = SharedKitLibrary::open(roots);
    reg.push((key, Arc::downgrade(&lib)));
    drop(reg);
    rescan_in_background(&lib);
    lib
}

/// The library just opened reads the index from disk as the last scan
/// left it — kits added, removed or renamed by hand since are not in it,
/// and with no editor open nothing would scan. So the first open scans
/// once, on a thread of its own (never the caller's: the caller may be a
/// host thread asking for `kit_select`'s text, and a scan hashes
/// manifests). It never fetches the plok.org index.
fn rescan_in_background(lib: &Arc<SharedKitLibrary>) {
    if lib.root().is_none() {
        return;
    }
    let lib = lib.clone();
    let spawned = std::thread::Builder::new()
        .name("resonance-drums-library-scan".to_string())
        .spawn(move || {
            if let Some(Err(e)) = lib.rescan_offline() {
                tracing::warn!("drum kit library scan: {e}");
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("could not start the drum kit library scan: {e}");
    }
}

/// How many times a process fetches the index for the migration: the
/// first scan, and once more on a later one.
const MIGRATION_FETCHES: u8 = 2;

/// What the plok.org tab offers for an index entry, given the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlokRowState {
    /// Not installed (a kit of the same name that did not come from
    /// plok.org does not count: a download lands beside it).
    Download,
    /// The kit is installed (its manifest hash, or — for a kit from
    /// plok.org — its name, matches).
    Installed { id: String },
    /// A kit downloaded from plok.org under that name is installed but its
    /// manifest differs from the index's: offer to replace it in place.
    Update { id: String, dir: PathBuf },
}

impl SharedKitLibrary {
    /// A library that is in no registry: tests build these at their own
    /// roots and drop them when done.
    #[doc(hidden)]
    pub fn open(roots: Roots) -> Arc<Self> {
        let marks = roots
            .marks_dir
            .clone()
            .map(|d| {
                SharedMarks::open(d).unwrap_or_else(|e| {
                    tracing::warn!("library marks unreadable: {e}");
                    SharedMarks::detached()
                })
            })
            .unwrap_or_else(SharedMarks::detached);
        let state = Arc::new(Mutex::new(DownloadState::default()));
        if let Some(root) = &roots.root {
            // An index fetched by an earlier session: the plok.org tab and
            // the migration have something before the first fetch.
            if let Some(index) = download::read_index_cache(root) {
                state.lock().index = Some(index);
            }
        }
        // The `installed.json` migration is handed to the library by the
        // first scan that has the index at hand (`migration_to_attach`).
        let lib = match &roots.root {
            Some(r) => Library::open(r),
            None => Library::empty(),
        };
        Arc::new_cyclic(|weak| Self {
            root: roots.root.clone(),
            lib: RwLock::new(lib),
            writer: Mutex::new(()),
            revision: AtomicU64::new(1),
            rescan_wanted: AtomicBool::new(false),
            marks,
            installed_json: roots.installed_json.clone(),
            migration_attached: AtomicBool::new(false),
            migration_fetches: AtomicU8::new(0),
            download: WorkerHandle::new(roots.worker, roots.root, state, weak.clone()),
        })
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// Read access. Never call from the audio thread.
    pub fn read(&self) -> RwLockReadGuard<'_, Library> {
        self.lib.read()
    }

    /// [`read`](Self::read), or `None` at once while a writer swaps the
    /// index in — for callers that must not wait (a parameter's text).
    pub fn try_read(&self) -> Option<RwLockReadGuard<'_, Library>> {
        self.lib.try_read()
    }

    /// Changes whenever the entries change in this process.
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    /// The download worker.
    pub fn download(&self) -> &WorkerHandle {
        &self.download
    }

    /// Apply `f` to a copy of the library — its I/O happens with no lock
    /// held — and swap the result in. Waits for any other writer. Runs a
    /// rescan that was skipped meanwhile afterwards.
    pub fn mutate<R>(&self, f: impl FnOnce(&mut Library) -> R) -> R {
        let guard = self.writer.lock();
        let out = self.mutate_locked(guard, f);
        self.run_wanted_rescan();
        out
    }

    /// [`mutate`](Self::mutate) unless another writer (an install, an
    /// import) is running: then `None` at once.
    pub fn try_mutate<R>(&self, f: impl FnOnce(&mut Library) -> R) -> Option<R> {
        let guard = self.writer.try_lock()?;
        let out = self.mutate_locked(guard, f);
        self.run_wanted_rescan();
        Some(out)
    }

    fn mutate_locked<R>(
        &self,
        _guard: parking_lot::MutexGuard<'_, ()>,
        f: impl FnOnce(&mut Library) -> R,
    ) -> R {
        let mut copy = self.lib.read().clone();
        let before_gen = copy.generation();
        let out = f(&mut copy);
        let changed =
            copy.generation() != before_gen || copy.entries() != self.lib.read().entries();
        *self.lib.write() = copy;
        if changed {
            self.revision.fetch_add(1, Ordering::AcqRel);
        }
        out
    }

    /// Scan the root, update the slot table and run the marks orphan pass.
    /// Blocks for as long as hashing takes: call it from a background
    /// thread. `None` when another writer holds the library: it rescans
    /// when it finishes.
    pub fn rescan(&self) -> Option<Result<ScanReport, LibraryError>> {
        self.rescan_with(&AtomicBool::new(false))
    }

    /// [`rescan`](Self::rescan), returning early from the migration's
    /// index fetch (bounded by [`WorkerConfig::migration_index_timeout`])
    /// once `cancel` is raised — pass the editor job's cancel flag, so
    /// closing the editor never waits on the network.
    pub fn rescan_with(&self, cancel: &AtomicBool) -> Option<Result<ScanReport, LibraryError>> {
        if self.root.is_none() {
            return Some(Ok(ScanReport::default()));
        }
        let attach = self.migration_to_attach(Some(cancel));
        self.scan(attach)
    }

    /// [`rescan`](Self::rescan) without ever fetching the plok.org index
    /// (the `installed.json` migration attaches only when it needs none).
    /// `None` when another writer holds the library: it rescans when it
    /// finishes.
    pub fn rescan_offline(&self) -> Option<Result<ScanReport, LibraryError>> {
        if self.root.is_none() {
            return Some(Ok(ScanReport::default()));
        }
        let attach = self.migration_to_attach(None);
        self.scan(attach)
    }

    /// Re-read the index if another process (another instance's editor,
    /// a scan) rewrote it since: a copy of the entries and one `stat` when
    /// nothing moved. For a
    /// lookup that missed — the name a host or agent asked for may be a
    /// kit this process has not seen yet. `false` when nothing changed,
    /// or a writer here holds the library (it publishes its own result).
    pub fn reload_if_changed(&self) -> bool {
        if self.root.is_none() {
            return false;
        }
        let Some(guard) = self.writer.try_lock() else {
            return false;
        };
        // A copy of the entries (a few dozen) and one `stat`; the swap
        // bumps the revision only when the entries changed.
        let reloaded = self.mutate_locked(guard, |lib| lib.reload_if_changed());
        self.run_wanted_rescan();
        reloaded
    }

    /// Whether a skipped rescan is waiting for the current writer.
    #[doc(hidden)]
    pub fn rescan_wanted(&self) -> bool {
        self.rescan_wanted.load(Ordering::SeqCst)
    }

    fn scan(&self, attach: Option<Migration>) -> Option<Result<ScanReport, LibraryError>> {
        // Raised before trying the lock, so a writer letting go after our
        // try always sees it.
        self.rescan_wanted.store(true, Ordering::SeqCst);
        let guard = self.writer.try_lock()?;
        self.rescan_wanted.store(false, Ordering::SeqCst);
        let attached = attach.is_some();
        let report = self.mutate_locked(guard, |lib| {
            if let Some(m) = attach {
                let opened = std::mem::replace(lib, Library::empty());
                *lib = opened.with_installed_json(m.installed_json, Some(m.in_index));
            }
            lib.rescan()
        });
        if attached {
            self.migration_attached.store(true, Ordering::SeqCst);
        }
        self.prune_marks();
        self.run_wanted_rescan();
        Some(report)
    }

    /// Run the rescan a busy writer made someone skip. Called by every
    /// writer when it lets go, and by the download worker after each job.
    /// Never fetches the index.
    pub(crate) fn run_wanted_rescan(&self) {
        if self.root.is_none() || !self.rescan_wanted.swap(false, Ordering::SeqCst) {
            return;
        }
        let attach = self.migration_to_attach(None);
        if let Some(Err(e)) = self.scan(attach) {
            tracing::warn!("drum kit library rescan: {e}");
        }
    }

    /// Measure the kits with no known size (a kit copied in by hand has
    /// no sidecar size; Drummica is 2,835 files) and record each. Stops
    /// between kits once `cancel` is set. Returns how many it recorded.
    pub fn measure_unsized(&self, cancel: &AtomicBool) -> usize {
        let dirs = self.lib.read().unsized_dirs();
        let mut recorded = 0;
        for dir in dirs {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let Ok(bytes) = drumkit_library::measure_size(&dir) else {
                continue;
            };
            match self.try_mutate(|lib| lib.record_size(&dir, bytes)) {
                Some(Ok(())) => recorded += 1,
                Some(Err(e)) => tracing::warn!("record kit size: {e}"),
                None => break,
            }
        }
        recorded
    }

    /// Import a kit folder or `.zip` (copied into the root, D2). `None`
    /// when another writer holds the library.
    pub fn import(
        &self,
        src: &Path,
        job: ImportJob<'_>,
    ) -> Option<Result<ImportOutcome, LibraryError>> {
        let out = self.try_mutate(|lib| lib.import(src, job))?;
        self.prune_marks();
        Some(out)
    }

    /// Delete the kit whose top directory is `dir` (`remove_dir_all`),
    /// then rescan; its marks are kept as orphans for the retention
    /// window. `None` when another writer holds the library.
    pub fn delete(&self, dir: &Path) -> Option<Result<Entry, LibraryError>> {
        let out = self.try_mutate(|lib| lib.delete(dir))?;
        self.prune_marks();
        Some(out)
    }

    /// The library entry a loaded manifest belongs to: the kit whose
    /// directory holds it.
    pub fn entry_for_manifest(&self, manifest: &Path) -> Option<Entry> {
        let lib = self.lib.read();
        let found = lib
            .entries()
            .iter()
            .find(|e| e.manifest_path == manifest)
            .or_else(|| lib.entries().iter().find(|e| manifest.starts_with(&e.dir)))
            .cloned();
        found
    }

    /// How many live drum instances in this process are playing (or
    /// loading) kit `id`.
    pub fn usage_count(&self, id: &str) -> usize {
        let dirs: Vec<PathBuf> = self
            .lib
            .read()
            .entries()
            .iter()
            .filter(|e| e.id == id)
            .map(|e| e.dir.clone())
            .collect();
        dirs.iter().map(|d| instances_playing(d)).sum()
    }

    /// What the plok.org tab shows for `kit`.
    pub fn plok_row_state(&self, kit: &ServerKit) -> PlokRowState {
        plok_row_state(&self.lib.read(), kit)
    }

    // -- Marks ---------------------------------------------------------------

    pub fn marks(&self) -> &SharedMarks {
        &self.marks
    }

    pub fn marks_of(&self, id: &str) -> Marks {
        self.marks.marks(&drumkit_library::mark_key(id))
    }

    pub fn marks_generation(&self) -> u64 {
        self.marks.generation()
    }

    pub fn refresh_marks(&self) -> bool {
        self.marks.refresh()
    }

    pub fn toggle_favorite(&self, id: &str) -> Result<Marks, MarksError> {
        self.marks.toggle_favorite(&drumkit_library::mark_key(id))
    }

    pub fn add_tag(&self, id: &str, tag: &str) -> Result<Marks, MarksError> {
        self.marks.add_tag(&drumkit_library::mark_key(id), tag)
    }

    pub fn remove_tag(&self, id: &str, tag: &str) -> Result<Marks, MarksError> {
        self.marks.remove_tag(&drumkit_library::mark_key(id), tag)
    }

    /// Record a user pick of kit `id` now (never a project-open restore,
    /// and never browsing with ◀/▶).
    pub fn record_use(&self, id: &str) -> Result<Marks, MarksError> {
        self.marks
            .record_use(&drumkit_library::mark_key(id), library_marks::now_unix())
    }

    /// Move the marks of kit `from_id` onto `to_id`: an update replaced
    /// the kit and its manifest changed, so it has a new id. Merged into
    /// whatever `to_id` already has (favourite and tags kept, the later
    /// last use, the use counts added); the old item is removed.
    pub(crate) fn carry_marks(&self, from_id: &str, to_id: &str) {
        if self.marks.dir().as_os_str().is_empty() {
            return;
        }
        self.marks.refresh();
        let from = drumkit_library::mark_key(from_id);
        let old = self.marks.marks(&from);
        if old.is_default() {
            return;
        }
        let to = drumkit_library::mark_key(to_id);
        let carried = self.marks.update(&to, |m| {
            m.favorite |= old.favorite;
            for tag in &old.tags {
                if !m.has_tag(tag) {
                    m.tags.push(tag.clone());
                }
            }
            m.last_used = m.last_used.max(old.last_used);
            m.use_count = m.use_count.saturating_add(old.use_count);
            if m.rating.is_none() {
                m.rating = old.rating;
            }
            for (k, v) in &old.extra {
                m.extra.entry(k.clone()).or_insert_with(|| v.clone());
            }
            m.orphaned_at = None;
        });
        match carried {
            Ok(_) => {
                if let Err(e) = self.marks.update(&from, |m| *m = Marks::default()) {
                    tracing::warn!("library marks: clear the replaced kit's marks: {e}");
                }
            }
            Err(e) => tracing::warn!("library marks: carry marks to the updated kit: {e}"),
        }
    }

    /// The orphan pass: stamp marks whose kit is gone, drop them after the
    /// retention window. Writes nothing when nothing changes.
    pub(crate) fn prune_marks(&self) {
        if self.marks.dir().as_os_str().is_empty() {
            return;
        }
        let live: HashSet<String> = self
            .lib
            .read()
            .entries()
            .iter()
            .map(|e| e.id.clone())
            .collect();
        if let Err(e) = self.marks.prune_orphans(
            drumkit_library::KIND,
            |id| live.contains(id),
            library_marks::now_unix(),
        ) {
            tracing::warn!("library marks prune failed: {e}");
        }
    }

    /// The `installed.json` migration runs once, on the scan it is handed
    /// to, and marks a kit `plok` only when the index has its name. So it
    /// is handed over only once the index is at hand — or when there is
    /// nothing for it to look up. Without an index, `cancel: Some` (a
    /// scan that may use the network) fetches one, bounded by
    /// [`WorkerConfig::migration_index_timeout`], at most
    /// [`MIGRATION_FETCHES`] times per process; until one arrives (that
    /// way, or by the plok.org tab's fetch) the migration waits, and the
    /// kits read as `local` without being written as such.
    fn migration_to_attach(&self, cancel: Option<&AtomicBool>) -> Option<Migration> {
        if self.migration_attached.load(Ordering::SeqCst) {
            return None;
        }
        let path = self.installed_json.clone()?;
        let root = self.root.clone()?;
        let in_index = in_index_fn(&self.download.state, &root);
        let ready = self.download.state.lock().index.is_some() || !lists_drum_kits(&path);
        let ready = ready
            || cancel.is_some_and(|cancel| {
                if self.migration_fetches.fetch_add(1, Ordering::SeqCst) >= MIGRATION_FETCHES {
                    return false;
                }
                let timeout = self.download.config().migration_index_timeout;
                match self.download.fetch_index_now(timeout, cancel) {
                    Ok(()) => true,
                    Err(e) => {
                        tracing::warn!(
                            "plok.org index unavailable for the kit migration ({e}); it waits \
                             for a later scan"
                        );
                        false
                    }
                }
            });
        ready.then_some(Migration {
            installed_json: path,
            in_index,
        })
    }
}

/// The migration handed to the library on a scan.
struct Migration {
    installed_json: PathBuf,
    in_index: Arc<InIndexFn>,
}

/// Whether `installed.json` lists any drum kit. Read directly: the
/// registry's loader quarantines a corrupt file, and this only reads.
fn lists_drum_kits(path: &Path) -> bool {
    use resonance_common::registry::{ContentType, InstalledRegistry};
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    serde_json::from_slice::<InstalledRegistry>(&bytes)
        .map(|reg| reg.items_of(&ContentType::Drumkit).next().is_some())
        .unwrap_or(false)
}

/// [`SharedKitLibrary::plok_row_state`] over a library snapshot. Only a kit
/// that came from plok.org under this index entry (its sidecar names it,
/// or it is a plok.org kit of that name) is ever offered as an Update: a
/// local or imported kit that happens to share the name is never replaced.
pub fn plok_row_state(lib: &Library, kit: &ServerKit) -> PlokRowState {
    if let Some(hash) = kit.manifest_sha256.as_deref().map(str::trim) {
        if let Some(e) = lib.entry(&hash.to_ascii_lowercase()) {
            return PlokRowState::Installed { id: e.id.clone() };
        }
    }
    let name = kit.name.trim();
    let same = |s: &str| s.trim().eq_ignore_ascii_case(name);
    let this_download = |e: &Entry| {
        e.sidecar
            .as_ref()
            .and_then(|s| s.index_name.as_deref())
            .is_some_and(same)
            || (e.source == Source::Plok && (same(&e.name) || same(&e.dir_name)))
    };
    match lib.entries().iter().find(|e| this_download(e)) {
        None => PlokRowState::Download,
        Some(e) if kit.manifest_sha256.is_some() => PlokRowState::Update {
            id: e.id.clone(),
            dir: e.dir.clone(),
        },
        Some(e) => PlokRowState::Installed { id: e.id.clone() },
    }
}

/// The migration's index lookup: the worker's cached index, else the one
/// cached on disk; `None` when neither has the name.
fn in_index_fn(state: &Arc<Mutex<DownloadState>>, root: &Path) -> Arc<InIndexFn> {
    let state = state.clone();
    let root = root.to_path_buf();
    Arc::new(move |name: &str| {
        let index = state
            .lock()
            .index
            .clone()
            .or_else(|| download::read_index_cache(&root))?;
        index.find(name).map(ServerKit::index_match)
    })
}
