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
//! **Opening is lazy.** A plugin instance never opens the library; the
//! editor factory does, on the first editor open. Headless renders never
//! read the library at all.
//!
//! **Usage.** Which kit each live instance is playing is read straight
//! from the instances' kit bridges ([`register_instance`]), so "used in N
//! open drum instances" counts instances whose editors are closed too.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use parking_lot::{Mutex, RwLock, RwLockReadGuard};
use resonance_common::drumkit_library::{
    self, Entry, ImportJob, ImportOutcome, InIndexFn, Library, LibraryError, ScanReport,
};
use resonance_common::library_marks::{self, Marks, MarksError, SharedMarks};

use crate::download::{self, ServerKit, State as DownloadState, WorkerConfig, WorkerHandle};

// ---------------------------------------------------------------------------
// Instance usage
// ---------------------------------------------------------------------------

type KitPathCell = Mutex<Option<PathBuf>>;

fn instances() -> &'static Mutex<Vec<Weak<KitPathCell>>> {
    static INSTANCES: OnceLock<Mutex<Vec<Weak<KitPathCell>>>> = OnceLock::new();
    INSTANCES.get_or_init(|| Mutex::new(Vec::new()))
}

/// Count a plugin instance in [`instances_playing`]: `kit_path` is its
/// bridge's "loaded manifest" cell. Held weakly; a dropped instance stops
/// counting by itself.
pub fn register_instance(kit_path: &Arc<KitPathCell>) {
    let mut all = instances().lock();
    all.retain(|w| w.strong_count() > 0);
    all.push(Arc::downgrade(kit_path));
}

/// How many live instances in this process are playing a kit under `dir`.
pub fn instances_playing(dir: &Path) -> usize {
    let mut all = instances().lock();
    all.retain(|w| w.strong_count() > 0);
    all.iter()
        .filter_map(Weak::upgrade)
        .filter(|cell| cell.lock().as_deref().is_some_and(|p| p.starts_with(dir)))
        .count()
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
    /// Favourites, tags and recents (`library_marks`, kind `drumkit`).
    marks: SharedMarks,
    /// The retired `installed.json`, the one-time migration's source.
    installed_json: Option<PathBuf>,
    /// Whether this process has made sure the index is at hand for the
    /// migration (once; it never repeats).
    migration_index_checked: AtomicBool,
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
/// else the platform's.
fn default_roots() -> Roots {
    if let Some(roots) = ROOT_OVERRIDE.get() {
        return roots.clone();
    }
    Roots {
        root: drumkit_library::default_root(),
        marks_dir: library_marks::default_library_dir(),
        installed_json: drumkit_library::default_installed_json(),
        worker: WorkerConfig::default(),
    }
}

/// Point [`shared`] at `roots` for the rest of this process — for tests,
/// which must never read or write the user's data dir. Also keeps the
/// preset bar's library beside it. No environment variable, so no
/// `setenv` race with the test harness's threads. First call wins.
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

/// Isolate this process from the user's data dir: the default library
/// lives under a fresh temp directory with no `installed.json`, and its
/// worker fetches from a closed loopback port. Idempotent; returns the
/// library root. Every editor test hook calls it, so no test frame can
/// scan (or migrate) the real library.
#[doc(hidden)]
pub fn isolate_for_tests() -> PathBuf {
    static BASE: OnceLock<PathBuf> = OnceLock::new();
    let base = BASE.get_or_init(|| {
        let base =
            std::env::temp_dir().join(format!("resonance-drums-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::create_dir_all(&base);
        override_default_roots(Roots {
            root: Some(base.join("drumkits")),
            marks_dir: Some(base.join("library")),
            installed_json: None,
            worker: WorkerConfig {
                index_url: "http://127.0.0.1:9/index.json".to_string(),
                ..WorkerConfig::default()
            },
        });
        base.clone()
    });
    match ROOT_OVERRIDE.get().and_then(|r| r.root.clone()) {
        Some(root) => root,
        None => base.join("drumkits"),
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
    lib
}

/// How long a rescan waits on the network for the index when the
/// `installed.json` migration is about to run without one.
const MIGRATION_INDEX_TIMEOUT: Duration = Duration::from_secs(5);

/// What the plok.org tab offers for an index entry, given the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlokRowState {
    /// Not installed.
    Download,
    /// The kit is installed (its manifest hash, or its name, matches).
    Installed { id: String },
    /// A kit of that name is installed but its manifest differs from the
    /// index's: offer to replace it in place.
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
        let lib = match &roots.root {
            Some(r) => {
                let lib = Library::open(r);
                match &roots.installed_json {
                    Some(path) => lib.with_installed_json(path, Some(in_index_fn(&state, r))),
                    None => lib,
                }
            }
            None => Library::empty(),
        };
        Arc::new_cyclic(|weak| Self {
            root: roots.root.clone(),
            lib: RwLock::new(lib),
            writer: Mutex::new(()),
            revision: AtomicU64::new(1),
            marks,
            installed_json: roots.installed_json.clone(),
            migration_index_checked: AtomicBool::new(false),
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

    /// Changes whenever the entries change in this process.
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    /// The download worker.
    pub fn download(&self) -> &WorkerHandle {
        &self.download
    }

    /// Apply `f` to a copy of the library — its I/O happens with no lock
    /// held — and swap the result in. Waits for any other writer.
    pub fn mutate<R>(&self, f: impl FnOnce(&mut Library) -> R) -> R {
        let guard = self.writer.lock();
        self.mutate_locked(guard, f)
    }

    /// [`mutate`](Self::mutate) unless another writer (an install, an
    /// import) is running: then `None` at once.
    pub fn try_mutate<R>(&self, f: impl FnOnce(&mut Library) -> R) -> Option<R> {
        let guard = self.writer.try_lock()?;
        Some(self.mutate_locked(guard, f))
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
    /// thread. `None` when another writer holds the library (it rescans
    /// when it finishes).
    pub fn rescan(&self) -> Option<Result<ScanReport, LibraryError>> {
        if self.root.is_none() {
            return Some(Ok(ScanReport::default()));
        }
        self.ensure_index_for_migration();
        let report = self.try_mutate(|lib| lib.rescan())?;
        self.prune_marks();
        Some(report)
    }

    /// Pick up another process's `library.json` write: one `stat` when
    /// nothing moved. Never waits on a writer.
    pub fn refresh(&self) -> bool {
        let Some(_w) = self.writer.try_lock() else {
            return false;
        };
        let mut copy = self.lib.read().clone();
        if !copy.reload_if_changed() {
            return false;
        }
        *self.lib.write() = copy;
        self.revision.fetch_add(1, Ordering::AcqRel);
        true
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

    /// How many live drum instances in this process are playing kit `id`.
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

    /// The `installed.json` migration runs once, on the first scan, and
    /// marks a kit `plok` only when the index has its name. Make sure the
    /// index is at hand for it: when `installed.json` lists drum kits and
    /// no index was ever fetched, fetch one now (bounded; a failure just
    /// leaves the kits `local`). Once per process.
    fn ensure_index_for_migration(&self) {
        if self.migration_index_checked.swap(true, Ordering::AcqRel) {
            return;
        }
        let Some(path) = &self.installed_json else {
            return;
        };
        if self.download.state.lock().index.is_some() {
            return;
        }
        let lists_kits = resonance_common::registry::load_registry_from(path)
            .items
            .iter()
            .any(|i| i.content_type == resonance_common::registry::ContentType::Drumkit);
        if lists_kits {
            if let Err(e) = self.download.fetch_index_now(MIGRATION_INDEX_TIMEOUT) {
                tracing::warn!("plok.org index unavailable for the kit migration: {e}");
            }
        }
    }
}

/// [`SharedKitLibrary::plok_row_state`] over a library snapshot.
pub fn plok_row_state(lib: &Library, kit: &ServerKit) -> PlokRowState {
    if let Some(hash) = kit.manifest_sha256.as_deref().map(str::trim) {
        if let Some(e) = lib.entry(&hash.to_ascii_lowercase()) {
            return PlokRowState::Installed { id: e.id.clone() };
        }
    }
    let name = kit.name.trim();
    let by_name = lib.entries().iter().find(|e| {
        e.sidecar
            .as_ref()
            .and_then(|s| s.index_name.as_deref())
            .is_some_and(|n| n.trim().eq_ignore_ascii_case(name))
            || e.name.trim().eq_ignore_ascii_case(name)
            || e.dir_name.trim().eq_ignore_ascii_case(name)
    });
    match by_name {
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
