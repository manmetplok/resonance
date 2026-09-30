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

/// Normalise a free tag: lowercase, `[a-z0-9-]` only (whitespace and `_`
/// become `-`, runs collapse, other characters drop), trimmed of `-`, at
/// most [`MAX_TAG_LEN`] bytes. `None` when nothing is left.
pub fn normalize_tag(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    for ch in vocab::fold_accents(raw).chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if (c.is_whitespace() || c == '-' || c == '_') && !out.ends_with('-') {
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

/// Read and parse the document. `Ok(None)` for a missing file; a file that
/// does not parse is quarantined (`marks.json.corrupt`) and reads as empty,
/// so one bad write can never wedge every later one.
fn read_doc(path: &Path) -> Result<Option<MarksDoc>, MarksError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_err("read", path)(e)),
    };
    match serde_json::from_slice::<MarksDoc>(&bytes) {
        Ok(doc) => Ok(Some(doc)),
        Err(e) => {
            tracing::error!("library marks {} unreadable ({e}); starting empty", path.display());
            quarantine_corrupt(path);
            Ok(None)
        }
    }
}

/// An exclusive advisory lock on `marks.lock`, released on drop.
struct DirLock {
    file: File,
}

impl DirLock {
    fn acquire(dir: &Path) -> Result<Self, MarksError> {
        std::fs::create_dir_all(dir).map_err(io_err("mkdir", dir))?;
        let path = dir.join(MARKS_LOCK_FILE);
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(io_err("open", &path))?;
        file.lock().map_err(io_err("lock", &path))?;
        Ok(Self { file })
    }
}

impl Drop for DirLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// The marks store: an in-memory copy of `marks.json` plus the locked
/// read-modify-write that changes it.
///
/// Reads are served from memory. Call [`reload_if_changed`](Self::reload_if_changed)
/// (cheap: one `stat`) to pick up other processes' writes; every mutation
/// re-reads under the lock anyway, so a stale copy never loses anyone
/// else's change.
#[derive(Debug)]
pub struct MarksStore {
    dir: PathBuf,
    doc: MarksDoc,
    stamp: Option<FileStamp>,
}

impl MarksStore {
    /// Open the store in `dir` (the directory holding `marks.json`). A
    /// missing file is an empty store; nothing is created until the first
    /// write.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, MarksError> {
        let dir = dir.into();
        let path = dir.join(MARKS_FILE);
        let stamp = stat(&path);
        let doc = read_doc(&path)?.unwrap_or_default();
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
    /// Returns whether the in-memory copy changed.
    pub fn reload_if_changed(&mut self) -> Result<bool, MarksError> {
        if self.dir.as_os_str().is_empty() {
            return Ok(false);
        }
        let path = self.path();
        let now = stat(&path);
        if now == self.stamp {
            return Ok(false);
        }
        let doc = read_doc(&path)?.unwrap_or_default();
        let changed = doc.generation != self.doc.generation || doc.items != self.doc.items;
        self.doc = doc;
        self.stamp = stat(&path);
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
    /// after `f` runs, and an item left at its defaults is removed.
    /// Returns the item's new marks.
    pub fn update(&mut self, key: &str, f: impl FnOnce(&mut Marks)) -> Result<Marks, MarksError> {
        if split_key(key).is_none() {
            return Err(MarksError::BadKey(key.to_string()));
        }
        let mut result = Marks::default();
        self.transact(|items| {
            let mut m = items.remove(key).unwrap_or_default();
            f(&mut m);
            m.tags = normalize_tags(&m.tags);
            if !m.is_default() {
                items.insert(key.to_string(), m.clone());
            }
            result = m;
            true
        })?;
        Ok(result)
    }

    pub fn set_favorite(&mut self, key: &str, favorite: bool) -> Result<Marks, MarksError> {
        self.update(key, |m| m.favorite = favorite)
    }

    /// Flip the favourite flag; returns the new marks.
    pub fn toggle_favorite(&mut self, key: &str) -> Result<Marks, MarksError> {
        self.update(key, |m| m.favorite = !m.favorite)
    }

    /// Replace the item's personal tags (normalised, deduplicated).
    pub fn set_tags<S: AsRef<str>>(&mut self, key: &str, tags: &[S]) -> Result<Marks, MarksError> {
        let tags = normalize_tags(tags);
        self.update(key, |m| m.tags = tags)
    }

    /// Add one tag (a no-op if it is already there or normalises to nothing).
    pub fn add_tag(&mut self, key: &str, tag: &str) -> Result<Marks, MarksError> {
        let tag = normalize_tag(tag);
        self.update(key, |m| {
            if let Some(tag) = tag {
                if !m.has_tag(&tag) {
                    m.tags.push(tag);
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
    /// load, never for a project-open restore.
    pub fn record_use(&mut self, key: &str, now: i64) -> Result<Marks, MarksError> {
        self.update(key, |m| {
            m.last_used = Some(now);
            m.use_count = m.use_count.saturating_add(1);
        })
    }

    /// Orphan pass for one kind: an item whose id `is_live` rejects is
    /// stamped `orphaned_at = now` the first time it is seen missing and
    /// removed once that is [`ORPHAN_RETENTION_SECS`] old; an item that is
    /// live again loses its stamp. Writes (under the lock) only when
    /// something changed. Returns how many items were removed.
    pub fn prune_orphans(
        &mut self,
        kind: &str,
        is_live: impl Fn(&str) -> bool,
        now: i64,
    ) -> Result<usize, MarksError> {
        let mut removed = 0usize;
        self.transact(|items| {
            let mut changed = false;
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

    /// The locked read-modify-write every mutation goes through. `apply`
    /// edits the freshly read items and returns whether it changed
    /// anything; nothing is written when it did not.
    fn transact(
        &mut self,
        apply: impl FnOnce(&mut BTreeMap<String, Marks>) -> bool,
    ) -> Result<(), MarksError> {
        if self.dir.as_os_str().is_empty() {
            return Err(MarksError::NoDataDir);
        }
        let _lock = DirLock::acquire(&self.dir)?;
        let path = self.path();
        let mut doc = read_doc(&path)?.unwrap_or_default();
        if apply(&mut doc.items) {
            doc.generation = doc.generation.wrapping_add(1);
            doc.version = MARKS_VERSION;
            let bytes = serde_json::to_vec_pretty(&doc)?;
            atomic_write(&path, &bytes)?;
        }
        self.doc = doc;
        self.stamp = stat(&path);
        Ok(())
    }
}
