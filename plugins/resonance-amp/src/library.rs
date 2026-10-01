//! The process-wide model library every amp instance shares
//! (nam-model-library.md §8): one [`Library`] per (library root, marks
//! dir) behind a read/write lock, handed out as an `Arc` so the loader
//! threads, the editors, the Tone3000 worker and `file_select`'s text
//! conversions all see the same index and slot table.
//!
//! Resonance hosts every amp in-process from one loaded `.clap` image, so a
//! static registry is the natural "one library" scope; a second host
//! process gets its own copy, kept consistent through the files and their
//! locks (`resonance_common::nam_library`).
//!
//! **Threads.** Nothing here is touched by `process()`. Readers (the loader
//! thread resolving a slot, an editor frame, a host's `value_to_text` on
//! the main thread) take the read lock only for as long as they copy what
//! they need; `file_select`'s text conversion uses `try_read` so it can
//! never wait. Every mutation — rescan, import, delete, a refresh from
//! another process's write — does its file I/O on a clone with no lock
//! held and swaps the result in, so a slow rescan never blocks a reader;
//! a writer mutex that no reader takes keeps two in-process mutations from
//! racing each other's swap. The marks are a
//! [`SharedMarks`](resonance_common::library_marks::SharedMarks), which
//! follows the same rule.
//!
//! **Activation is read-only.** `initialize` only re-reads the cached
//! index ([`SharedLibrary::refresh`]) and resolves against it. Scanning
//! (hashing new files, allocating slots, pruning marks) happens lazily and
//! off the main thread: when an editor opens, and on the loader thread
//! when a slot it is asked for is not in the index.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

use parking_lot::{Mutex, RwLock, RwLockReadGuard};
use resonance_common::library_marks::{self, Marks, MarksError, SharedMarks};
use resonance_common::nam_library::{self, Library, LibraryError, ScanReport};

pub struct SharedLibrary {
    root: Option<PathBuf>,
    lib: RwLock<Library>,
    /// Serialises clone-mutate-swap writers inside this process.
    writer: Mutex<()>,
    /// Bumped on every in-process change of the entries, so editors know
    /// when to rebuild their rows.
    revision: AtomicU64,
    /// Which model each live instance in this process is playing
    /// (instance id → content id), for "used in N open amps".
    usage: Mutex<HashMap<u64, String>>,
    /// Favourites, tags and recents (`library_marks`, kind `amp-model`),
    /// the one store every library kind shares. Readers never wait on its
    /// I/O.
    marks: SharedMarks,
    /// Content ids of files outside the index, by (path, size, mtime), so a
    /// re-activation does not re-hash an external model.
    hashes: Mutex<HashMap<PathBuf, (u64, u64, String)>>,
    /// When the loader last rescanned for a slot it could not find.
    last_miss_scan: Mutex<Option<Instant>>,
}

/// A process-unique id for an amp instance (the usage registry's key).
pub fn next_instance_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

type Key = (Option<PathBuf>, Option<PathBuf>);
type Registry = Mutex<Vec<(Key, Weak<SharedLibrary>)>>;

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Vec::new()))
}

/// The shared library for `root` (`None`: no data dir, an empty library)
/// with its marks in `marks_dir` (`None`: a detached store whose writes
/// fail). Every caller asking for the same pair gets the same instance
/// while any of them holds it.
pub fn shared_for(root: Option<PathBuf>, marks_dir: Option<PathBuf>) -> Arc<SharedLibrary> {
    let key = (root, marks_dir);
    let mut reg = registry().lock();
    reg.retain(|(_, w)| w.strong_count() > 0);
    if let Some(lib) = reg
        .iter()
        .find(|(k, _)| *k == key)
        .and_then(|(_, w)| w.upgrade())
    {
        return lib;
    }
    let lib = Arc::new(SharedLibrary::new(key.0.clone(), key.1.clone()));
    reg.push((key, Arc::downgrade(&lib)));
    lib
}

/// The roots [`shared`] uses: a test override when one was installed,
/// else [`nam_library::default_root`] and
/// [`library_marks::default_library_dir`].
fn default_roots() -> Key {
    if let Some(roots) = ROOT_OVERRIDE.get() {
        return roots.clone();
    }
    (nam_library::default_root(), library_marks::default_library_dir())
}

static ROOT_OVERRIDE: OnceLock<Key> = OnceLock::new();

/// Point every instance built with `ResonanceAmp::new()` in this process at
/// `root` / `marks_dir`, for a test that has to go through the CLAP entry
/// point (where it cannot pass a library). No environment variable, so no
/// `setenv` race with the test harness's threads. First call wins.
#[doc(hidden)]
pub fn override_default_roots(root: PathBuf, marks_dir: PathBuf) {
    // The editor's preset bar reads the preset library too: keep it in
    // the same private place, next to the models.
    let presets = root
        .parent()
        .map(|p| p.join("plugin-presets"))
        .unwrap_or_else(|| root.join("plugin-presets"));
    resonance_plugin::presets::override_default_roots(presets, Some(marks_dir.clone()));
    let _ = ROOT_OVERRIDE.set((Some(root), Some(marks_dir)));
}

/// The shared library at the default roots.
pub fn shared() -> Arc<SharedLibrary> {
    let (root, marks) = default_roots();
    shared_for(root, marks)
}

/// How long the loader waits between rescans for slots it cannot find.
const MISS_SCAN_INTERVAL: Duration = Duration::from_secs(2);

impl SharedLibrary {
    fn new(root: Option<PathBuf>, marks_dir: Option<PathBuf>) -> Self {
        let lib = match &root {
            Some(r) => Library::open(r),
            None => Library::empty(),
        };
        let marks = marks_dir
            .map(|d| {
                SharedMarks::open(d).unwrap_or_else(|e| {
                    tracing::warn!("library marks unreadable: {e}");
                    SharedMarks::detached()
                })
            })
            .unwrap_or_else(SharedMarks::detached);
        Self {
            root,
            lib: RwLock::new(lib),
            writer: Mutex::new(()),
            revision: AtomicU64::new(1),
            usage: Mutex::new(HashMap::new()),
            marks,
            hashes: Mutex::new(HashMap::new()),
            last_miss_scan: Mutex::new(None),
        }
    }

    /// Record what `instance` is playing (`None`: nothing, or it is gone).
    pub fn set_usage(&self, instance: u64, id: Option<&str>) {
        let mut usage = self.usage.lock();
        match id {
            Some(id) => {
                usage.insert(instance, id.to_string());
            }
            None => {
                usage.remove(&instance);
            }
        }
    }

    /// How many live instances in this process are playing `id`.
    pub fn usage_count(&self, id: &str) -> usize {
        self.usage.lock().values().filter(|v| *v == id).count()
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// Read access. Never call from the audio thread.
    pub fn read(&self) -> RwLockReadGuard<'_, Library> {
        self.lib.read()
    }

    /// Non-blocking read access, for text conversions a host may call at
    /// any time.
    pub fn try_read(&self) -> Option<RwLockReadGuard<'_, Library>> {
        self.lib.try_read()
    }

    /// Changes whenever the entries change in this process.
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    /// Apply `f` to a copy of the library — its I/O happens with no lock
    /// held — and swap the result in. The revision is bumped when the
    /// entries changed.
    pub fn mutate<R>(&self, f: impl FnOnce(&mut Library) -> R) -> R {
        let _w = self.writer.lock();
        let mut copy = self.lib.read().clone();
        let before_gen = copy.generation();
        let out = f(&mut copy);
        let changed = copy.generation() != before_gen || copy.entries() != self.lib.read().entries();
        *self.lib.write() = copy;
        if changed {
            self.revision.fetch_add(1, Ordering::AcqRel);
        }
        out
    }

    /// Scan the files (hashing only new or changed ones), update the slot
    /// table and run the marks orphan pass. Blocks for as long as hashing
    /// takes: call it from a background thread (the editor's rescan
    /// thread, the loader, the Tone3000 worker), never from `initialize`
    /// or an editor frame. Touches nothing on disk when nothing changed.
    pub fn rescan(&self) -> Result<ScanReport, LibraryError> {
        if self.root.is_none() {
            return Ok(ScanReport::default());
        }
        let report = self.mutate(|lib| lib.rescan())?;
        self.prune_marks();
        Ok(report)
    }

    /// [`rescan`](Self::rescan) for a loader that could not find a slot,
    /// at most once per [`MISS_SCAN_INTERVAL`] so a lane automating
    /// `file_select` over empty slots does not hash the library over and
    /// over. Returns whether it scanned.
    pub fn rescan_for_miss(&self) -> bool {
        {
            let mut last = self.last_miss_scan.lock();
            if last.is_some_and(|t| t.elapsed() < MISS_SCAN_INTERVAL) {
                return false;
            }
            *last = Some(Instant::now());
        }
        if let Err(e) = self.rescan() {
            tracing::warn!("model library rescan failed: {e}");
        }
        true
    }

    /// Pick up another process's `library.json` write, if any: one `stat`
    /// when nothing moved; the parse happens on a copy with no lock held.
    /// Read-only: never scans, never writes.
    ///
    /// Never waits on a writer: activation calls this, and a writer may be
    /// a full rescan hashing new files. With one in flight the current
    /// snapshot is served (the rescan publishes a fresher one when done).
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

    /// The content id of `path`: the index's, when it has the file and it
    /// is unchanged; else a hash cached per (path, size, mtime) for this
    /// process, so an external model is hashed once, not on every
    /// activation.
    pub fn content_id(&self, path: &Path) -> Option<String> {
        let meta = std::fs::metadata(path).ok()?;
        let (size, mtime) = (meta.len(), nam_library::file_mtime_ns(&meta));
        if let Some(e) = self.lib.read().by_path(path) {
            if e.size_bytes == size && e.mtime_ns == mtime {
                return Some(e.id.clone());
            }
        }
        if let Some((s, m, id)) = self.hashes.lock().get(path) {
            if *s == size && *m == mtime {
                return Some(id.clone());
            }
        }
        let id = nam_library::hash_file(path).ok()?;
        self.hashes
            .lock()
            .insert(path.to_path_buf(), (size, mtime, id.clone()));
        Some(id)
    }

    // -- Marks ---------------------------------------------------------------

    /// The marks store. Its reads never wait on I/O; paint paths may call
    /// them every frame.
    pub fn marks(&self) -> &SharedMarks {
        &self.marks
    }

    /// The marks of model `id`.
    pub fn marks_of(&self, id: &str) -> Marks {
        self.marks.marks(&nam_library::mark_key(id))
    }

    /// The store's write counter (lock-free).
    pub fn marks_generation(&self) -> u64 {
        self.marks.generation()
    }

    /// Pick up another process's marks write (one `stat` when unchanged).
    pub fn refresh_marks(&self) -> bool {
        self.marks.refresh()
    }

    pub fn toggle_favorite(&self, id: &str) -> Result<Marks, MarksError> {
        self.marks.toggle_favorite(&nam_library::mark_key(id))
    }

    pub fn set_favorite(&self, id: &str, on: bool) -> Result<Marks, MarksError> {
        self.marks.set_favorite(&nam_library::mark_key(id), on)
    }

    pub fn set_tags(&self, id: &str, tags: &[String]) -> Result<Marks, MarksError> {
        self.marks.set_tags(&nam_library::mark_key(id), tags)
    }

    pub fn add_tag(&self, id: &str, tag: &str) -> Result<Marks, MarksError> {
        self.marks.add_tag(&nam_library::mark_key(id), tag)
    }

    pub fn remove_tag(&self, id: &str, tag: &str) -> Result<Marks, MarksError> {
        self.marks.remove_tag(&nam_library::mark_key(id), tag)
    }

    /// Record a user pick of `id` now (never a project-open restore, D10,
    /// and never browsing with ◀/▶).
    pub fn record_use(&self, id: &str) -> Result<Marks, MarksError> {
        self.marks
            .record_use(&nam_library::mark_key(id), library_marks::now_unix())
    }

    /// The orphan pass (§7.2): stamp marks whose model is gone, drop them
    /// after the retention window. Writes nothing when nothing changes.
    fn prune_marks(&self) {
        if self.marks.dir().as_os_str().is_empty() {
            return;
        }
        let live: std::collections::HashSet<String> = self
            .lib
            .read()
            .entries()
            .iter()
            .map(|e| e.id.clone())
            .collect();
        if let Err(e) = self.marks.prune_orphans(
            nam_library::KIND,
            |id| live.contains(id),
            library_marks::now_unix(),
        ) {
            tracing::warn!("library marks prune failed: {e}");
        }
    }
}
