//! The process-wide model library every amp instance shares
//! (nam-model-library.md §8): one [`Library`] per library root behind a
//! read/write lock, handed out as an `Arc` so the loader threads, the
//! editors, the Tone3000 worker and `file_select`'s text conversions all
//! see the same index and slot table.
//!
//! Resonance hosts every amp in-process from one loaded `.clap` image, so a
//! static registry is the natural "one library" scope; a second host
//! process gets its own copy, kept consistent through the files and their
//! locks (`resonance_common::nam_library`).
//!
//! **Threads.** Nothing here is touched by `process()`. Readers (the loader
//! thread resolving a slot, an editor frame, a host's `value_to_text` on
//! the main thread) take the read lock; `file_select`'s text conversion
//! uses `try_read` so it can never wait. Mutations (rescan, import, delete)
//! run on a clone outside the lock and swap the result in, so a slow
//! rescan never blocks a reader, and a writer mutex keeps two in-process
//! mutations from racing each other's swap.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};

use parking_lot::{Mutex, RwLock, RwLockReadGuard};
use resonance_common::nam_library::{self, Library, LibraryError, ScanReport};

pub struct SharedLibrary {
    root: Option<PathBuf>,
    lib: RwLock<Library>,
    /// Serialises clone-mutate-swap writers inside this process.
    writer: Mutex<()>,
    /// Bumped on every in-process change of the entries, so editors know
    /// when to rebuild their rows.
    revision: AtomicU64,
    scanned: AtomicBool,
    /// Which model each live instance in this process is playing
    /// (instance id → content id), for "used in N open amps".
    usage: Mutex<HashMap<u64, String>>,
}

/// A process-unique id for an amp instance (the usage registry's key).
pub fn next_instance_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

type Registry = Mutex<Vec<(Option<PathBuf>, Weak<SharedLibrary>)>>;

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Vec::new()))
}

/// The shared library for `root` (`None`: no data dir, an empty library).
/// Every caller asking for the same root gets the same instance while any
/// of them holds it.
pub fn shared_for(root: Option<PathBuf>) -> Arc<SharedLibrary> {
    let mut reg = registry().lock();
    reg.retain(|(_, w)| w.strong_count() > 0);
    if let Some(lib) = reg
        .iter()
        .find(|(r, _)| *r == root)
        .and_then(|(_, w)| w.upgrade())
    {
        return lib;
    }
    let lib = Arc::new(SharedLibrary::new(root.clone()));
    reg.push((root, Arc::downgrade(&lib)));
    lib
}

/// The shared library at [`nam_library::default_root`].
pub fn shared() -> Arc<SharedLibrary> {
    shared_for(nam_library::default_root())
}

impl SharedLibrary {
    fn new(root: Option<PathBuf>) -> Self {
        let lib = match &root {
            Some(r) => Library::open(r),
            None => Library::empty(),
        };
        Self {
            root,
            lib: RwLock::new(lib),
            writer: Mutex::new(()),
            revision: AtomicU64::new(1),
            scanned: AtomicBool::new(false),
            usage: Mutex::new(HashMap::new()),
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

    /// Apply `f` to a copy of the library and swap the result in. The
    /// revision is bumped when the entries changed.
    pub fn mutate<R>(&self, f: impl FnOnce(&mut Library) -> R) -> R {
        let _w = self.writer.lock();
        let mut copy = self.lib.read().clone();
        let before = copy.generation();
        let out = f(&mut copy);
        let changed = copy.generation() != before || copy.entries() != self.lib.read().entries();
        *self.lib.write() = copy;
        if changed {
            self.revision.fetch_add(1, Ordering::AcqRel);
        }
        out
    }

    /// Scan the files (hashing only new or changed ones) and update the
    /// slot table.
    pub fn rescan(&self) -> Result<ScanReport, LibraryError> {
        self.scanned.store(true, Ordering::Release);
        if self.root.is_none() {
            return Ok(ScanReport::default());
        }
        self.mutate(|lib| lib.rescan())
    }

    /// Rescan once per process; later calls only pick up other processes'
    /// index writes (one `stat`).
    pub fn ensure_scanned(&self) {
        if self.scanned.swap(true, Ordering::AcqRel) {
            self.refresh();
            return;
        }
        if let Err(e) = self.rescan() {
            tracing::warn!("model library scan failed: {e}");
        }
    }

    /// Pick up another process's `library.json` write, if any.
    pub fn refresh(&self) -> bool {
        let changed = {
            let _w = self.writer.lock();
            self.lib.write().reload_if_changed()
        };
        if changed {
            self.revision.fetch_add(1, Ordering::AcqRel);
        }
        changed
    }
}
