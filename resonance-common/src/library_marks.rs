//! Per-user library marks: favourites, personal tags and recents for every
//! kind of library asset (NAM amp models, plugin presets, later IRs, kits …).
//!
//! One file for every kind, `$XDG_DATA_HOME/resonance/library/marks.json`
//! (overridable with [`LIBRARY_DIR_ENV`]), keyed by `"<kind>:<id>"`. The store
//! only ever sees the key as an opaque string; each kind decides what goes
//! after the first `:` (nam-model-library.md §4.3, plugin-preset-library.md
//! §4.2 / §4.5).
//!
//! ```json
//! { "version": 1,
//!   "generation": 412,
//!   "items": {
//!     "amp-model:9f2c…": { "favorite": true, "tags": ["djent"], "last_used": "2026-09-30T14:02:11Z", "use_count": 12 }
//!   } }
//! ```
//!
//! **Concurrency.** Every mutation takes an exclusive [`File::lock`] on
//! `marks.lock` next to the file, re-reads `marks.json`, applies only its own
//! delta, bumps `generation` and atomically replaces the file. Two processes
//! that star different items within the same millisecond both win; the same
//! item is last-writer-wins. The OS releases the lock if a process dies. The
//! lock is held for the length of one small read and write, and it is a
//! GUI/main-thread operation: it must never be taken on an audio thread.
//!
//! **Freshness.** [`MarksStore::reload_if_changed`] re-reads the file when
//! its size or mtime moved, and [`FreshnessPoll`] is the shared poll-based
//! change detector (no watcher dependency) both libraries use.
//!
//! **Orphans.** An item whose asset is gone is kept for
//! [`ORPHAN_RETENTION_SECS`] (90 days) so a delete followed by a re-download
//! or re-import keeps its star, then dropped by [`MarksStore::prune_orphans`].

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::atomic_file::{atomic_write, quarantine_corrupt, AtomicWriteError};

mod freshness;
mod stamp;
pub mod vocab;

pub use freshness::{
    fingerprint, Fingerprint, FreshnessPoll, BAR_POLL_INTERVAL, BROWSER_POLL_INTERVAL,
};
pub use stamp::{format_timestamp, now_unix, parse_timestamp};

/// Environment variable that overrides the library directory (the one
/// holding `marks.json`). Tests use it to stay out of the user's data dir;
/// it mirrors `RESONANCE_PLUGIN_PRESET_DIR` / `RESONANCE_AMP_MODEL_DIR`.
pub const LIBRARY_DIR_ENV: &str = "RESONANCE_LIBRARY_DIR";

/// Subdirectory of the platform data dir that holds the marks file.
pub const LIBRARY_SUBDIR: &str = "resonance/library";

/// The marks file's name inside the library directory.
pub const MARKS_FILE: &str = "marks.json";

/// The advisory lock file's name inside the library directory.
pub const MARKS_LOCK_FILE: &str = "marks.lock";

/// Schema version written to `marks.json`.
pub const MARKS_VERSION: u32 = 1;

/// How long a mark whose asset has disappeared is kept before
/// [`MarksStore::prune_orphans`] drops it: 90 days.
pub const ORPHAN_RETENTION_SECS: i64 = 90 * 24 * 60 * 60;

/// Longest tag [`normalize_tag`] keeps, in bytes (tags are ASCII).
pub const MAX_TAG_LEN: usize = 32;

/// The kind names the fleet uses. A kind is `[a-z0-9-]+`, and it is the
/// part of a key before the first `:`.
pub mod kind {
    /// A NAM model; the id is the sha256 of the file bytes.
    pub const AMP_MODEL: &str = "amp-model";
    /// A plugin preset; the id is `"<clap id>:<preset id>"`.
    pub const PLUGIN_PRESET: &str = "plugin-preset";
}

/// Build the store key `"<kind>:<id>"`.
pub fn mark_key(kind: &str, id: &str) -> String {
    format!("{kind}:{id}")
}

/// Split a store key at its first `:` into `(kind, id)`. `None` for a key
/// with no `:` or an empty kind.
pub fn split_key(key: &str) -> Option<(&str, &str)> {
    let (kind, id) = key.split_once(':')?;
    (!kind.is_empty()).then_some((kind, id))
}

/// The library directory: [`LIBRARY_DIR_ENV`] if set and non-empty, else
/// `<data dir>/resonance/library`. `None` only on a platform with no data
/// dir at all.
pub fn default_library_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(LIBRARY_DIR_ENV).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    dirs::data_dir().map(|d| d.join(LIBRARY_SUBDIR))
}

/// Normalise a free tag or facet value: accents folded, lowercase
/// `[a-z0-9-]` only, every run of anything else collapsed to one `-`
/// (`"R&B"` → `"r-b"`, `"Drum & Bass"` → `"drum-bass"`), trimmed of `-`, at
/// most [`MAX_TAG_LEN`] bytes. `None` when nothing is left. The one slug
/// rule for every library kind (the preset library's `normalize_facet`
/// spells it the same way).
pub fn normalize_tag(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    for c in vocab::fold(raw).chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    let mut tag: String = trimmed.chars().take(MAX_TAG_LEN).collect();
    while tag.ends_with('-') {
        tag.pop();
    }
    (!tag.is_empty()).then_some(tag)
}

/// Normalise, dedupe and keep the first-seen order of a tag list.
pub fn normalize_tags<I, S>(tags: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out: Vec<String> = Vec::new();
    for t in tags {
        if let Some(t) = normalize_tag(t.as_ref()) {
            if !out.contains(&t) {
                out.push(t);
            }
        }
    }
    out
}

/// One item's marks. Every field defaults to "unmarked"; an item whose
/// fields are all at their defaults is deleted rather than stored.
///
/// Timestamps are Unix seconds (UTC) in memory and RFC 3339 strings on disk.
/// Fields this build does not know are kept in `extra` and written back, so
/// a newer build's additions survive an older build's write.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Marks {
    #[serde(default, skip_serializing_if = "is_false")]
    pub favorite: bool,
    /// Personal tags, normalised by [`normalize_tag`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// When the user last picked the item (a browser/bar pick or a
    /// control-API load), never a project-open restore.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "stamp::serde_opt"
    )]
    pub last_used: Option<i64>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub use_count: u32,
    /// Reserved (plugin-preset-library.md D4): not offered in v1, kept so
    /// adding it later is additive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<u8>,
    /// When [`MarksStore::prune_orphans`] first saw the asset missing.
    /// Cleared when it reappears.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "stamp::serde_opt"
    )]
    pub orphaned_at: Option<i64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl Marks {
    /// Whether nothing about the item is marked. `orphaned_at` alone does
    /// not count: an unmarked orphan has nothing worth keeping.
    pub fn is_default(&self) -> bool {
        !self.favorite
            && self.tags.is_empty()
            && self.last_used.is_none()
            && self.use_count == 0
            && self.rating.is_none()
            && self.extra.is_empty()
    }

    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }

    /// `last_used` as the RFC 3339 string the file stores, for consumers
    /// that carry it as text.
    pub fn last_used_rfc3339(&self) -> Option<String> {
        self.last_used.and_then(format_timestamp)
    }
}

/// A marks store shared between threads — an editor, a host-side browser,
/// an index's freshness poll — with every method on `&self`, so it can sit
/// in an `Arc` and back a read-only trait object (a preset index's marks
/// source) while writers use the same instance.
///
/// **Readers never wait on I/O.** Reads are served from an immutable
/// snapshot (`Arc<MarksStore>`) that is swapped in after a write or reload
/// completes; the generation is an atomic. Writes do their file lock, read,
/// write and fsync with no in-memory lock held, serialised among this
/// process's writers by a separate mutex that no reader takes. A paint path
/// can call [`marks`](Self::marks), [`snapshot`](Self::snapshot) and
/// [`generation`](Self::generation) every frame.
#[derive(Debug)]
pub struct SharedMarks {
    dir: PathBuf,
    snapshot: std::sync::RwLock<Arc<MarksStore>>,
    generation: AtomicU64,
    /// Serialises this process's writers (and reloads) with each other.
    writer: std::sync::Mutex<()>,
}

impl SharedMarks {
    pub fn new(store: MarksStore) -> Self {
        Self {
            dir: store.dir.clone(),
            generation: AtomicU64::new(store.generation()),
            snapshot: std::sync::RwLock::new(Arc::new(store)),
            writer: std::sync::Mutex::new(()),
        }
    }

    /// Open the store in `dir` (reads only; nothing is created).
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, MarksError> {
        MarksStore::open(dir).map(Self::new)
    }

    /// A store that lives nowhere and whose writes fail.
    pub fn detached() -> Self {
        Self::new(MarksStore::detached())
    }

    /// Open the store at [`default_library_dir`], or a detached one when
    /// there is none (or it cannot be read), so callers need no `Option`.
    pub fn open_default_or_detached() -> Self {
        Self::new(MarksStore::open_default().unwrap_or_else(|e| {
            tracing::warn!("library marks unavailable: {e}");
            MarksStore::detached()
        }))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Path of `marks.json`.
    pub fn path(&self) -> PathBuf {
        self.dir.join(MARKS_FILE)
    }

    /// The current in-memory copy, for reads of more than one item (tag
    /// completion, iteration). Never blocks on I/O.
    pub fn snapshot(&self) -> Arc<MarksStore> {
        self.snapshot
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Swap the snapshot in, then publish its generation (Release,
    /// paired with [`generation`](Self::generation)'s Acquire): a reader
    /// that sees generation N and then takes the snapshot gets one at least
    /// as new as N, so a cache keyed on the generation never pins an older
    /// snapshot under the newer number.
    fn install(&self, store: MarksStore) {
        let generation = store.generation();
        *self.snapshot.write().unwrap_or_else(|p| p.into_inner()) = Arc::new(store);
        self.generation.store(generation, Ordering::Release);
    }

    /// The marks of `key`, or the defaults.
    pub fn marks(&self, key: &str) -> Marks {
        self.snapshot().marks(key)
    }

    pub fn is_favorite(&self, key: &str) -> bool {
        self.snapshot().is_favorite(key)
    }

    /// The write counter as last read (lock-free).
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// The refresh hook: re-read the file if another writer changed it
    /// (one `stat` when not). Returns whether anything changed. The read
    /// happens outside every in-memory lock.
    pub fn refresh(&self) -> bool {
        if self.dir.as_os_str().is_empty() {
            return false;
        }
        let _w = self.writer.lock().unwrap_or_else(|p| p.into_inner());
        let current = self.snapshot();
        let mut next = MarksStore {
            dir: current.dir.clone(),
            doc: MarksDoc::default(),
            stamp: current.stamp,
        };
        next.doc = current.doc.clone();
        match next.reload_if_changed() {
            Ok(true) => {
                self.install(next);
                true
            }
            _ => false,
        }
    }

    fn write(
        &self,
        op: impl FnOnce(&mut MarksStore) -> Result<Marks, MarksError>,
    ) -> Result<Marks, MarksError> {
        let _w = self.writer.lock().unwrap_or_else(|p| p.into_inner());
        let current = self.snapshot();
        let mut next = MarksStore {
            dir: current.dir.clone(),
            doc: current.doc.clone(),
            stamp: current.stamp,
        };
        let out = op(&mut next)?;
        self.install(next);
        Ok(out)
    }

    /// [`MarksStore::update`], without blocking readers.
    pub fn update(&self, key: &str, f: impl Fn(&mut Marks)) -> Result<Marks, MarksError> {
        self.write(|s| s.update(key, f))
    }

    pub fn set_favorite(&self, key: &str, favorite: bool) -> Result<Marks, MarksError> {
        self.update(key, |m| m.favorite = favorite)
    }

    /// Flip the favourite flag relative to the latest copy on disk.
    pub fn toggle_favorite(&self, key: &str) -> Result<Marks, MarksError> {
        self.write(|s| s.toggle_favorite(key))
    }

    pub fn set_tags<S: AsRef<str>>(&self, key: &str, tags: &[S]) -> Result<Marks, MarksError> {
        let tags = normalize_tags(tags);
        self.update(key, |m| m.tags = tags.clone())
    }

    pub fn add_tag(&self, key: &str, tag: &str) -> Result<Marks, MarksError> {
        self.write(|s| s.add_tag(key, tag))
    }

    pub fn remove_tag(&self, key: &str, tag: &str) -> Result<Marks, MarksError> {
        self.write(|s| s.remove_tag(key, tag))
    }

    pub fn record_use(&self, key: &str, now: i64) -> Result<Marks, MarksError> {
        self.write(|s| s.record_use(key, now))
    }

    /// [`MarksStore::prune_orphans`], without blocking readers.
    pub fn prune_orphans(
        &self,
        kind: &str,
        is_live: impl Fn(&str) -> bool,
        now: i64,
    ) -> Result<usize, MarksError> {
        let mut removed = 0;
        self.write(|s| {
            removed = s.prune_orphans(kind, is_live, now)?;
            Ok(Marks::default())
        })?;
        Ok(removed)
    }
}

/// The on-disk document.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MarksDoc {
    #[serde(default = "marks_version")]
    version: u32,
    #[serde(default)]
    generation: u64,
    #[serde(default)]
    items: BTreeMap<String, Marks>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}

fn marks_version() -> u32 {
    MARKS_VERSION
}

impl Default for MarksDoc {
    fn default() -> Self {
        Self {
            version: MARKS_VERSION,
            generation: 0,
            items: BTreeMap::new(),
            extra: BTreeMap::new(),
        }
    }
}

/// Failure reading or writing the marks store.
#[derive(Debug, Error)]
pub enum MarksError {
    /// No `$XDG_DATA_HOME` (or platform equivalent) and no override.
    #[error("no data dir for the library marks store")]
    NoDataDir,
    #[error("{op} {}: {source}", path.display())]
    Io {
        op: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("serialize marks: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error(transparent)]
    Write(#[from] AtomicWriteError),
    /// A key with no `<kind>:` prefix.
    #[error("invalid mark key {0:?}: expected \"<kind>:<id>\"")]
    BadKey(String),
}

fn io_err<'a>(op: &'static str, path: &'a Path) -> impl FnOnce(std::io::Error) -> MarksError + 'a {
    move |source| MarksError::Io {
        op,
        path: path.to_path_buf(),
        source,
    }
}

/// Size + mtime of the marks file as last read, to skip re-parsing an
/// unchanged file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    mtime: Option<std::time::SystemTime>,
}

fn stat(path: &Path) -> Option<FileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(FileStamp {
        len: meta.len(),
        mtime: meta.modified().ok(),
    })
}

/// How a read of `marks.json` went.
enum ReadDoc {
    Missing,
    Ok(MarksDoc),
    /// It exists but does not parse.
    Corrupt(String),
}

fn read_doc(path: &Path) -> Result<ReadDoc, MarksError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ReadDoc::Missing),
        Err(e) => return Err(io_err("read", path)(e)),
    };
    Ok(match serde_json::from_slice::<MarksDoc>(&bytes) {
        Ok(doc) => ReadDoc::Ok(doc),
        Err(e) => ReadDoc::Corrupt(e.to_string()),
    })
}

/// Whether a `lock()` failure means "this filesystem has no locks" (NFS
/// without lockd, some FUSE mounts) rather than a real error.
pub fn lock_error_is_unsupported(e: &std::io::Error) -> bool {
    if e.kind() == std::io::ErrorKind::Unsupported {
        return true;
    }
    // ENOLCK: 37 on Linux, 77 on macOS/BSD.
    #[cfg(target_os = "linux")]
    let enolck = 37;
    #[cfg(not(target_os = "linux"))]
    let enolck = 77;
    e.raw_os_error() == Some(enolck)
}

/// Take an exclusive advisory lock on `file`. On a filesystem without
/// locks, warn once per process and carry on unlocked (best effort) rather
/// than make the library unusable there.
pub(crate) fn lock_or_best_effort(file: &File, path: &Path) -> std::io::Result<bool> {
    static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    match file.lock() {
        Ok(()) => Ok(true),
        Err(e) if lock_error_is_unsupported(&e) => {
            if !WARNED.swap(true, Ordering::Relaxed) {
                tracing::warn!(
                    "{}: file locks are not supported here ({e}); library writes proceed unlocked",
                    path.display()
                );
            }
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

/// An exclusive advisory lock on `marks.lock`, released on drop.
struct DirLock {
    file: File,
    locked: bool,
}

impl DirLock {
    /// Creates the directory and the lock file: call it only when a write
    /// is about to happen.
    fn acquire(dir: &Path) -> Result<Self, MarksError> {
        std::fs::create_dir_all(dir).map_err(io_err("mkdir", dir))?;
        let path = dir.join(MARKS_LOCK_FILE);
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(io_err("open", &path))?;
        let locked = lock_or_best_effort(&file, &path).map_err(io_err("lock", &path))?;
        Ok(Self { file, locked })
    }
}

impl Drop for DirLock {
    fn drop(&mut self) {
        if self.locked {
            let _ = self.file.unlock();
        }
    }
}

/// The marks store: an in-memory copy of `marks.json` plus the locked
/// read-modify-write that changes it. Single-owner (`&mut self` writes);
/// share one across threads with [`SharedMarks`].
///
/// Reads are served from memory. Call [`reload_if_changed`](Self::reload_if_changed)
/// (cheap: one `stat`) to pick up other processes' writes; every mutation
/// re-reads under the lock anyway, so a stale copy never loses anyone
/// else's change.
#[derive(Debug, Clone)]
pub struct MarksStore {
    dir: PathBuf,
    doc: MarksDoc,
    stamp: Option<FileStamp>,
}

impl MarksStore {
    /// Open the store in `dir` (the directory holding `marks.json`). A
    /// missing file is an empty store; nothing is created until the first
    /// write. A file that does not parse reads as empty here and is
    /// quarantined by the next write, under the lock.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, MarksError> {
        let dir = dir.into();
        let path = dir.join(MARKS_FILE);
        let stamp = stat(&path);
        let doc = match read_doc(&path)? {
            ReadDoc::Ok(doc) => doc,
            ReadDoc::Missing => MarksDoc::default(),
            ReadDoc::Corrupt(e) => {
                tracing::error!("library marks {} unreadable ({e}); reading as empty", path.display());
                MarksDoc::default()
            }
        };
        Ok(Self { dir, doc, stamp })
    }

    /// Open the store at [`default_library_dir`].
    pub fn open_default() -> Result<Self, MarksError> {
        Self::open(default_library_dir().ok_or(MarksError::NoDataDir)?)
    }

    /// An empty store that lives nowhere and whose writes fail: for a
    /// platform with no data dir, so callers need no `Option`.
    pub fn detached() -> Self {
        Self {
            dir: PathBuf::new(),
            doc: MarksDoc::default(),
            stamp: None,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Path of `marks.json`.
    pub fn path(&self) -> PathBuf {
        self.dir.join(MARKS_FILE)
    }

    /// The document's write counter, as last read. Bumped by every write
    /// from any process.
    pub fn generation(&self) -> u64 {
        self.doc.generation
    }

    /// Re-read the file if another writer changed it since the last read.
    /// Returns whether the in-memory copy changed. The stamp is taken
    /// before the read, so a write landing mid-read is seen next time; a
    /// file that does not parse is left alone (a writer may be mid-way) and
    /// the old copy kept.
    pub fn reload_if_changed(&mut self) -> Result<bool, MarksError> {
        if self.dir.as_os_str().is_empty() {
            return Ok(false);
        }
        let path = self.path();
        let now = stat(&path);
        if now == self.stamp {
            return Ok(false);
        }
        let doc = match read_doc(&path)? {
            ReadDoc::Ok(doc) => doc,
            ReadDoc::Missing => MarksDoc::default(),
            ReadDoc::Corrupt(_) => return Ok(false),
        };
        let changed = doc.generation != self.doc.generation || doc.items != self.doc.items;
        self.doc = doc;
        self.stamp = now;
        Ok(changed)
    }

    /// The marks of `key`, if any are stored.
    pub fn get(&self, key: &str) -> Option<&Marks> {
        self.doc.items.get(key)
    }

    /// The marks of `key`, or the defaults.
    pub fn marks(&self, key: &str) -> Marks {
        self.get(key).cloned().unwrap_or_default()
    }

    pub fn is_favorite(&self, key: &str) -> bool {
        self.get(key).is_some_and(|m| m.favorite)
    }

    /// Every stored item, in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Marks)> {
        self.doc.items.iter().map(|(k, m)| (k.as_str(), m))
    }

    /// Every stored item of one kind, as `(id, marks)`.
    pub fn iter_kind<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = (&'a str, &'a Marks)> {
        self.iter()
            .filter_map(move |(k, m)| match split_key(k) {
                Some((kk, id)) if kk == kind => Some((id, m)),
                _ => None,
            })
    }

    /// Ids of `kind` used most recently first, at most `limit` of them.
    pub fn recent(&self, kind: &str, limit: usize) -> Vec<(String, i64)> {
        let mut out: Vec<(String, i64)> = self
            .iter_kind(kind)
            .filter_map(|(id, m)| m.last_used.map(|t| (id.to_string(), t)))
            .collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out.truncate(limit);
        out
    }

    /// Every personal tag across every kind with how many items carry it,
    /// most used first, then by name.
    pub fn tag_counts(&self) -> Vec<(String, usize)> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for (_, m) in self.iter() {
            for t in &m.tags {
                *counts.entry(t.as_str()).or_default() += 1;
            }
        }
        let mut out: Vec<(String, usize)> =
            counts.into_iter().map(|(t, n)| (t.to_string(), n)).collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out
    }

    /// Tag completion for a `+ tag` field: tags already used anywhere in the
    /// library (most used first), then the seeded facet vocabulary
    /// ([`vocab::all_seeded`]), each matching `prefix` after normalisation
    /// and without duplicates; `exclude` (the item's current tags) is left
    /// out. An empty prefix lists the used tags only.
    pub fn complete_tag(&self, prefix: &str, exclude: &[String], limit: usize) -> Vec<String> {
        let prefix = normalize_tag(prefix).unwrap_or_default();
        let mut out: Vec<String> = Vec::new();
        let push = |t: &str, out: &mut Vec<String>| {
            if out.len() < limit
                && t.starts_with(prefix.as_str())
                && !exclude.iter().any(|e| e == t)
                && !out.iter().any(|o| o == t)
            {
                out.push(t.to_string());
            }
        };
        for (t, _) in self.tag_counts() {
            push(&t, &mut out);
        }
        if !prefix.is_empty() {
            for t in vocab::all_seeded() {
                push(t, &mut out);
            }
        }
        out
    }

    /// Apply `f` to the marks of `key` under the lock: take the lock,
    /// re-read the file, change only this item, bump `generation`, write
    /// atomically, and refresh the in-memory copy. Tags are normalised
    /// after `f` runs, and an item left at its defaults is removed. An
    /// update that changes nothing writes nothing. `f` may run twice (a
    /// dry run on the in-memory copy, then under the lock). Returns the
    /// item's new marks.
    pub fn update(&mut self, key: &str, f: impl Fn(&mut Marks)) -> Result<Marks, MarksError> {
        if split_key(key).is_none() {
            return Err(MarksError::BadKey(key.to_string()));
        }
        let mut result = Marks::default();
        self.transact(|items| {
            let old = items.get(key).cloned();
            let mut m = old.clone().unwrap_or_default();
            f(&mut m);
            m.tags = normalize_tags(&m.tags);
            let new = (!m.is_default()).then(|| m.clone());
            result = m;
            if new == old {
                return false;
            }
            match new {
                Some(m) => {
                    items.insert(key.to_string(), m);
                }
                None => {
                    items.remove(key);
                }
            }
            true
        })?;
        Ok(result)
    }

    pub fn set_favorite(&mut self, key: &str, favorite: bool) -> Result<Marks, MarksError> {
        self.update(key, |m| m.favorite = favorite)
    }

    /// Flip the favourite flag of the latest copy on disk; returns the new
    /// marks.
    pub fn toggle_favorite(&mut self, key: &str) -> Result<Marks, MarksError> {
        // Not `update`: its dry run would flip the in-memory copy and the
        // locked pass the on-disk one, which may differ.
        if split_key(key).is_none() {
            return Err(MarksError::BadKey(key.to_string()));
        }
        let mut result = Marks::default();
        self.transact_always(|items| {
            let mut m = items.get(key).cloned().unwrap_or_default();
            m.favorite = !m.favorite;
            if m.is_default() {
                items.remove(key);
            } else {
                items.insert(key.to_string(), m.clone());
            }
            result = m;
        })?;
        Ok(result)
    }

    /// Replace the item's personal tags (normalised, deduplicated).
    pub fn set_tags<S: AsRef<str>>(&mut self, key: &str, tags: &[S]) -> Result<Marks, MarksError> {
        let tags = normalize_tags(tags);
        self.update(key, |m| m.tags = tags.clone())
    }

    /// Add one tag (a no-op if it is already there or normalises to nothing).
    pub fn add_tag(&mut self, key: &str, tag: &str) -> Result<Marks, MarksError> {
        let tag = normalize_tag(tag);
        self.update(key, |m| {
            if let Some(tag) = &tag {
                if !m.has_tag(tag) {
                    m.tags.push(tag.clone());
                }
            }
        })
    }

    pub fn remove_tag(&mut self, key: &str, tag: &str) -> Result<Marks, MarksError> {
        let tag = normalize_tag(tag).unwrap_or_default();
        self.update(key, |m| m.tags.retain(|t| *t != tag))
    }

    /// Record a user pick at `now` (Unix seconds): sets `last_used` and
    /// bumps `use_count`. Call it for a browser/bar pick or a control-API
    /// load, never for a project-open restore or for browsing.
    pub fn record_use(&mut self, key: &str, now: i64) -> Result<Marks, MarksError> {
        if split_key(key).is_none() {
            return Err(MarksError::BadKey(key.to_string()));
        }
        let mut result = Marks::default();
        self.transact_always(|items| {
            let m = items.entry(key.to_string()).or_default();
            m.last_used = Some(now);
            m.use_count = m.use_count.saturating_add(1);
            result = m.clone();
        })?;
        Ok(result)
    }

    /// Orphan pass for one kind: an item whose id `is_live` rejects is
    /// stamped `orphaned_at = now` the first time it is seen missing and
    /// removed once that is [`ORPHAN_RETENTION_SECS`] old; an item that is
    /// live again loses its stamp. Touches nothing on disk — not even the
    /// lock file — when nothing changes. Returns how many items were
    /// removed.
    pub fn prune_orphans(
        &mut self,
        kind: &str,
        is_live: impl Fn(&str) -> bool,
        now: i64,
    ) -> Result<usize, MarksError> {
        let mut removed = 0usize;
        self.transact(|items| {
            let mut changed = false;
            removed = 0;
            items.retain(|key, m| {
                let Some((k, id)) = split_key(key) else {
                    return true;
                };
                if k != kind {
                    return true;
                }
                if is_live(id) {
                    if m.orphaned_at.take().is_some() {
                        changed = true;
                    }
                    return true;
                }
                match m.orphaned_at {
                    None => {
                        m.orphaned_at = Some(now);
                        changed = true;
                        true
                    }
                    Some(since) if now - since >= ORPHAN_RETENTION_SECS => {
                        removed += 1;
                        changed = true;
                        false
                    }
                    Some(_) => true,
                }
            });
            changed
        })?;
        Ok(removed)
    }

    /// The read-modify-write every conditional mutation goes through.
    /// `apply` edits items and returns whether it changed anything. It is
    /// first run on a copy of the in-memory state: when that changes
    /// nothing, nothing on disk is touched (no directory, no lock file, no
    /// write). Otherwise the lock is taken, the file re-read, `apply` run
    /// again on the fresh items, and the result written if it still
    /// changes something.
    fn transact(
        &mut self,
        mut apply: impl FnMut(&mut BTreeMap<String, Marks>) -> bool,
    ) -> Result<(), MarksError> {
        if self.dir.as_os_str().is_empty() {
            return Err(MarksError::NoDataDir);
        }
        let mut dry = self.doc.items.clone();
        if !apply(&mut dry) && stat(&self.path()) == self.stamp {
            return Ok(());
        }
        self.locked_write(|items| apply(items))
    }

    /// A mutation that always writes (its effect cannot be a no-op).
    fn transact_always(
        &mut self,
        apply: impl FnOnce(&mut BTreeMap<String, Marks>),
    ) -> Result<(), MarksError> {
        if self.dir.as_os_str().is_empty() {
            return Err(MarksError::NoDataDir);
        }
        let mut apply = Some(apply);
        self.locked_write(|items| {
            if let Some(f) = apply.take() {
                f(items);
            }
            true
        })
    }

    fn locked_write(
        &mut self,
        apply: impl FnOnce(&mut BTreeMap<String, Marks>) -> bool,
    ) -> Result<(), MarksError> {
        let _lock = DirLock::acquire(&self.dir)?;
        let path = self.path();
        let mut doc = match read_doc(&path)? {
            ReadDoc::Ok(doc) => doc,
            ReadDoc::Missing => MarksDoc::default(),
            ReadDoc::Corrupt(e) => {
                // Under the lock, so no writer is mid-install: this really
                // is a bad file. Keep it for the user, start empty.
                tracing::error!("library marks {} unreadable ({e}); starting empty", path.display());
                quarantine_corrupt(&path);
                MarksDoc::default()
            }
        };
        if apply(&mut doc.items) {
            doc.generation = doc.generation.wrapping_add(1);
            // A newer build's file keeps its version.
            doc.version = doc.version.max(MARKS_VERSION);
            let bytes = serde_json::to_vec_pretty(&doc)?;
            atomic_write(&path, &bytes)?;
        }
        self.doc = doc;
        self.stamp = stat(&path);
        Ok(())
    }
}
