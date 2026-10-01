//! The per-user drum-kit library (drums-plugin-rework.md §3): an index of
//! every installed kit with what its manifest says about it, content ids,
//! and the stable **slot table** the drums' `kit_select` parameter will
//! index (§5.1). Modelled on [`crate::nam_library`].
//!
//! ```text
//! <root>/                               (env override: RESONANCE_DRUMKIT_DIR)
//!   Drummica/                           one kit = a directory directly under the root
//!     drummica/drum_samples.json        its manifest, at depth 0 or 1
//!     kit.meta.json                     provenance sidecar, in the top directory
//!   IT Techno/ …
//!   .staging/                           in-flight imports / extractions
//!   library.json                        index + slot table (a cache)
//!   library.lock                        File::lock for the writes to it
//! ```
//!
//! The directories and their sidecars are the truth; `library.json` is a
//! cache of manifest hashes and summaries, measured sizes and the slot
//! table. Deleting it costs a rescan and re-assigns slots in name order.
//!
//! **Identity** (D1) is the sha256 of the `drum_samples.json` bytes: cheap
//! to compute, the same through a rename or move, and different when the
//! kit's contents change. It is computed once per (manifest path, size,
//! mtime). Marks are keyed `drumkit:<id>`.
//!
//! **Slots** follow the NAM library's rules exactly (never reused below
//! the high-water mark; a kit that comes back gets its old slot while it
//! is free), so a `kit_select` value in a preset or automation lane never
//! recalls a different kit.
//!
//! **Import** (D2) copies into the root through `.staging/` and renames,
//! so a scan never lists a half-copied kit; it takes a cancel flag, a
//! progress callback and a disk check ([`ImportJob`]).
//!
//! **`installed.json`** (D3): on the first scan of a library given an
//! `installed.json` path ([`Library::with_installed_json`]), each drumkit
//! item whose directory is a kit with no sidecar gets one, carrying its
//! `installed_at`. Its source is `plok` when the supplied predicate says
//! the name is in the plok.org index, else `local`. It runs once per
//! index (a flag in `library.json`) and never changes `installed.json`.
//!
//! **Concurrency**: every write of `library.json` runs under an exclusive
//! `File::lock` on `library.lock`, re-reading the index under the lock
//! first. Nothing here may run on an audio thread.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::atomic_file::{atomic_write, quarantine_corrupt, AtomicWriteError};

mod install;
mod manifest;
mod sidecar;

pub use install::{free_space, measure_size, ImportJob, ImportProgress};
pub use manifest::{
    sample_paths, summarize, Articulation, ManifestError, ManifestSummary, MicSetupInfo, Piece,
    MANIFEST_FILE, META_KEY,
};
pub use sidecar::{
    read_sidecar, sidecar_path, write_sidecar, Sidecar, SIDECAR_FILE, SOURCE_IMPORTED,
    SOURCE_LOCAL, SOURCE_PLOK,
};

/// The marks kind of a kit (`"drumkit:<sha256>"`).
pub const KIND: &str = crate::library_marks::kind::DRUMKIT;

/// Overrides the library root. Tests use it (or the explicit-root API) to
/// stay out of the user's data dir.
pub const DRUMKIT_DIR_ENV: &str = "RESONANCE_DRUMKIT_DIR";

/// The root under the platform data dir.
pub const KIT_SUBDIR: &str = "resonance/drumkits";

/// In-flight imports and extractions, under the root. Never scanned.
pub const STAGING_DIR: &str = ".staging";

pub const LIBRARY_FILE: &str = "library.json";
pub const LOCK_FILE: &str = "library.lock";

/// How many slots `kit_select` can address (`0..=MAX_SLOT`).
pub const SLOT_COUNT: u32 = 1000;
pub const MAX_SLOT: u32 = SLOT_COUNT - 1;

/// Import needs this much free space per byte copied.
pub const IMPORT_SPACE_FACTOR: f64 = 1.1;

const INDEX_VERSION: u32 = 1;

/// The library root: [`DRUMKIT_DIR_ENV`] if set and non-empty, else
/// `<data dir>/resonance/drumkits`.
pub fn default_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(DRUMKIT_DIR_ENV).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    dirs::data_dir().map(|d| d.join(KIT_SUBDIR))
}

/// Where the retired installed-content registry lives
/// (`<data dir>/resonance/installed.json`), the migration source.
pub fn default_installed_json() -> Option<PathBuf> {
    crate::registry::registry_path()
}

/// The mark key of a kit id.
pub fn mark_key(id: &str) -> String {
    crate::library_marks::mark_key(KIND, id)
}

/// sha256 of a manifest's bytes, lowercase hex: the kit id.
pub fn hash_manifest(path: &Path) -> std::io::Result<String> {
    crate::nam_library::hash_file(path)
}

/// The manifest of the kit in `kit_dir`: `kit_dir/drum_samples.json`, else
/// the first (by name) non-hidden subdirectory holding one.
pub fn find_manifest(kit_dir: &Path) -> Option<PathBuf> {
    let direct = kit_dir.join(MANIFEST_FILE);
    if direct.is_file() {
        return Some(direct);
    }
    let mut subs: Vec<PathBuf> = std::fs::read_dir(kit_dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && !is_hidden(&e.file_name()))
        .map(|e| e.path())
        .collect();
    subs.sort();
    subs.into_iter()
        .map(|s| s.join(MANIFEST_FILE))
        .find(|m| m.is_file())
}

/// The sample files `manifest` names that do not exist (paths resolved
/// against the manifest's directory). Stats every file: run it off the
/// UI thread for a large kit.
pub fn missing_files(manifest: &Path) -> Result<Vec<PathBuf>, LibraryError> {
    let bytes = std::fs::read(manifest).map_err(io_err("read", manifest))?;
    let dir = manifest.parent().unwrap_or(Path::new(""));
    let paths = sample_paths(&bytes, dir).map_err(|e| LibraryError::NotAKit {
        path: manifest.to_path_buf(),
        reason: e.0,
    })?;
    Ok(paths.into_iter().filter(|p| !p.is_file()).collect())
}

/// "8.5 GB", "5.0 MB", "412 KB".
pub fn format_bytes(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}

fn is_hidden(name: &std::ffi::OsStr) -> bool {
    name.to_string_lossy().starts_with('.')
}

/// Where a kit came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// Downloaded from plok.org.
    Plok,
    /// Copied in by Import.
    Imported,
    /// Anything else (copied in by hand; no sidecar).
    Local,
}

impl Source {
    pub fn label(&self) -> &'static str {
        match self {
            Source::Plok => SOURCE_PLOK,
            Source::Imported => SOURCE_IMPORTED,
            Source::Local => SOURCE_LOCAL,
        }
    }

    fn from_sidecar(s: Option<&Sidecar>) -> Self {
        match s.map(|s| s.source.as_str()) {
            Some(SOURCE_PLOK) => Source::Plok,
            Some(SOURCE_IMPORTED) => Source::Imported,
            _ => Source::Local,
        }
    }
}

/// Health of one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryStatus {
    Ok,
    /// The manifest did not parse as a kit; the reason.
    ManifestError(String),
    /// This many sample files the manifest names are missing. Only known
    /// after [`Library::check_missing_files`]; a scan never stats samples.
    MissingFiles(usize),
    /// Same manifest bytes as the kit in this directory, which holds the
    /// slot.
    DuplicateOf(PathBuf),
}

/// One installed kit, as the browser, the drums and the control API see it.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// sha256 of the manifest bytes.
    pub id: String,
    /// The `kit_select` value, if this entry holds one.
    pub slot: Option<u32>,
    /// Sidecar `index_name` › `_meta.name` › the top directory's name.
    pub name: String,
    /// The kit's top directory (directly under the root).
    pub dir: PathBuf,
    /// The top directory's name.
    pub dir_name: String,
    /// The manifest, absolute.
    pub manifest_path: PathBuf,
    /// The manifest relative to the root (`Drummica/drummica/drum_samples.json`):
    /// the portable half of a kit reference.
    pub rel_path: PathBuf,
    pub source: Source,
    /// The sidecar, if the kit has one.
    pub sidecar: Option<Sidecar>,
    /// Sidecar `downloaded_at`, else when the index first saw the kit.
    pub added_at: i64,
    /// Sidecar `size_bytes`, else a size measured by
    /// [`Library::record_size`]; `None` until measured.
    pub size_bytes: Option<u64>,
    pub pieces: Vec<Piece>,
    /// Setup key → position, brand, mic, channel.
    pub mic_setups: BTreeMap<String, MicSetupInfo>,
    pub articulations: Vec<Articulation>,
    pub layers_max: u32,
    pub rr_max: u32,
    pub sample_count: u64,
    pub status: EntryStatus,
}

impl Entry {
    pub fn mark_key(&self) -> String {
        mark_key(&self.id)
    }

    pub fn is_ok(&self) -> bool {
        self.status == EntryStatus::Ok
    }

    /// Whether the kit can be loaded at all (its manifest parsed and it is
    /// not a duplicate). A kit with missing files is still loadable.
    pub fn is_loadable(&self) -> bool {
        matches!(self.status, EntryStatus::Ok | EntryStatus::MissingFiles(_))
    }

    pub fn description(&self) -> Option<&str> {
        self.sidecar.as_ref().and_then(|s| s.description.as_deref())
    }

    /// The plok.org index's tags (content tags; personal tags are marks).
    pub fn index_tags(&self) -> &[String] {
        self.sidecar
            .as_ref()
            .map(|s| s.index_tags.as_slice())
            .unwrap_or(&[])
    }

    /// A friendly label for a mic setup key: "Shure Beta 91 · KickIn", or
    /// the raw key when the kit does not describe it.
    pub fn mic_label(&self, setup_key: &str) -> String {
        self.mic_setups
            .get(setup_key)
            .map(MicSetupInfo::label)
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| setup_key.to_string())
    }

    /// The display name of a piece key (`_meta.pieces`), else the key.
    pub fn piece_name<'a>(&'a self, key: &'a str) -> &'a str {
        self.pieces
            .iter()
            .find(|p| p.key == key)
            .map(|p| p.name.as_str())
            .unwrap_or(key)
    }
}

/// One kit as the index remembers it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct KitRecord {
    dir: PathBuf,
    manifest: PathBuf,
    size: u64,
    mtime_ns: u64,
    id: String,
    first_seen: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    summary: Option<ManifestSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sidecar: Option<Sidecar>,
    /// Bytes on disk, from [`Library::record_size`]. Reset when the
    /// manifest changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    measured_size: Option<u64>,
}

/// `library.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct IndexDoc {
    version: u32,
    generation: u64,
    next_slot: u32,
    /// slot → id.
    slots: BTreeMap<u32, String>,
    /// id → the slot it held before it disappeared.
    #[serde(default)]
    retired: BTreeMap<String, u32>,
    /// Whether the one-time `installed.json` migration has run.
    #[serde(default)]
    installed_json_migrated: bool,
    kits: Vec<KitRecord>,
}

impl Default for IndexDoc {
    fn default() -> Self {
        Self {
            version: INDEX_VERSION,
            generation: 0,
            next_slot: 0,
            slots: BTreeMap::new(),
            retired: BTreeMap::new(),
            installed_json_migrated: false,
            kits: Vec::new(),
        }
    }
}

/// Failure reading, scanning or changing the library.
#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("no drum kit library directory (no data dir)")]
    NoRoot,
    #[error("{op} {}: {source}", path.display())]
    Io {
        op: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("serialize library index: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error(transparent)]
    Write(#[from] AtomicWriteError),
    #[error("{} is not a drum kit: {reason}", path.display())]
    NotAKit { path: PathBuf, reason: String },
    #[error("{} is not a kit of the drum kit library", path.display())]
    OutsideLibrary { path: PathBuf },
    #[error(
        "not enough free space: needs {}, {} available ({} short)",
        format_bytes(*needed),
        format_bytes(*available),
        format_bytes(needed.saturating_sub(*available))
    )]
    InsufficientSpace { needed: u64, available: u64 },
    #[error("cancelled")]
    Cancelled,
    #[error("zip: {0}")]
    Zip(String),
}

fn io_err<'a>(
    op: &'static str,
    path: &'a Path,
) -> impl FnOnce(std::io::Error) -> LibraryError + 'a {
    move |source| LibraryError::Io {
        op,
        path: path.to_path_buf(),
        source,
    }
}

/// What a rescan found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanReport {
    /// Ids that gained a slot.
    pub added: Vec<String>,
    /// Ids whose slot was freed (the kit is gone).
    pub removed: Vec<String>,
    /// Manifests hashed this time (new or changed).
    pub hashed: usize,
    /// Sidecars written by the `installed.json` migration.
    pub migrated: usize,
    /// Whether `library.json` was rewritten.
    pub changed: bool,
}

/// What an import did.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportOutcome {
    /// Copied (or extracted) into the root.
    Added(Entry),
    /// A kit with the same manifest is already in the library; nothing was
    /// left behind.
    AlreadyPresent(Entry),
}

impl ImportOutcome {
    pub fn entry(&self) -> &Entry {
        match self {
            ImportOutcome::Added(e) | ImportOutcome::AlreadyPresent(e) => e,
        }
    }
}

/// Says whether a kit name is in the plok.org index, for the
/// `installed.json` migration's `source`.
pub type InIndexFn = dyn Fn(&str) -> bool + Send + Sync;

#[derive(Clone)]
struct Migration {
    installed_json: PathBuf,
    in_index: Option<Arc<InIndexFn>>,
}

impl std::fmt::Debug for Migration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Migration")
            .field("installed_json", &self.installed_json)
            .field("in_index", &self.in_index.is_some())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    len: u64,
    mtime: Option<std::time::SystemTime>,
}

fn stat_stamp(path: &Path) -> Option<Stamp> {
    let m = std::fs::metadata(path).ok()?;
    Some(Stamp {
        len: m.len(),
        mtime: m.modified().ok(),
    })
}

fn mtime_ns(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// The kits under `root`: (top directory, manifest), by directory name.
fn scan_kits(root: &Path) -> Vec<(PathBuf, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && !is_hidden(&e.file_name()))
        .map(|e| e.path())
        .collect();
    dirs.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
    dirs.into_iter()
        .filter_map(|d| find_manifest(&d).map(|m| (d, m)))
        .collect()
}

/// The kit library: the index plus its derived entries.
#[derive(Debug, Clone)]
pub struct Library {
    root: Option<PathBuf>,
    doc: IndexDoc,
    stamp: Option<Stamp>,
    entries: Vec<Entry>,
    by_id: HashMap<String, usize>,
    by_slot: HashMap<u32, usize>,
    by_dir: HashMap<PathBuf, usize>,
    /// Missing-sample counts by id, from [`Library::check_missing_files`].
    /// In memory only: a scan never stats samples.
    missing: HashMap<String, usize>,
    migration: Option<Migration>,
}

impl Library {
    /// A library with no root: empty, and every write fails with
    /// [`LibraryError::NoRoot`].
    pub fn empty() -> Self {
        Self {
            root: None,
            doc: IndexDoc::default(),
            stamp: None,
            entries: Vec::new(),
            by_id: HashMap::new(),
            by_slot: HashMap::new(),
            by_dir: HashMap::new(),
            missing: HashMap::new(),
            migration: None,
        }
    }

    /// Load the cached index under `root` without scanning. A missing
    /// index is an empty library; a corrupt one reads as empty (the next
    /// rescan quarantines and rebuilds it).
    pub fn open(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let path = root.join(LIBRARY_FILE);
        let stamp = stat_stamp(&path);
        let doc = read_index(&path, false);
        let mut lib = Self {
            root: Some(root),
            doc,
            stamp,
            ..Self::empty()
        };
        lib.rebuild_entries();
        lib
    }

    /// [`open`](Self::open) then [`rescan`](Self::rescan).
    pub fn open_and_scan(root: impl Into<PathBuf>) -> Result<Self, LibraryError> {
        let mut lib = Self::open(root);
        lib.rescan()?;
        Ok(lib)
    }

    /// Migrate the drumkit items of `installed_json` into sidecars on the
    /// next scan, once per index (see the module docs). `in_index` says
    /// whether a kit name is in the plok.org index (→ `source = plok`);
    /// without one every migrated kit is `local`.
    pub fn with_installed_json(
        mut self,
        installed_json: impl Into<PathBuf>,
        in_index: Option<Arc<InIndexFn>>,
    ) -> Self {
        self.migration = Some(Migration {
            installed_json: installed_json.into(),
            in_index,
        });
        self
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub fn staging_dir(&self) -> Option<PathBuf> {
        self.root.as_ref().map(|r| r.join(STAGING_DIR))
    }

    /// The paths a [`crate::library_marks::FreshnessPoll`] should watch:
    /// the root (a kit added or removed) and the index.
    pub fn watch_paths(&self) -> Vec<PathBuf> {
        match &self.root {
            Some(r) => vec![r.clone(), r.join(LIBRARY_FILE)],
            None => Vec::new(),
        }
    }

    /// The index's write counter.
    pub fn generation(&self) -> u64 {
        self.doc.generation
    }

    /// Every entry, slotted ones first in slot order, then the rest
    /// (duplicates, the overflow past 1000) by directory.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Total known bytes on disk (kits not yet measured count 0).
    pub fn total_bytes(&self) -> u64 {
        self.entries.iter().filter_map(|e| e.size_bytes).sum()
    }

    /// The canonical entry of a kit id.
    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.by_id.get(id).map(|&i| &self.entries[i])
    }

    /// [`entry`](Self::entry) (the spec's name).
    pub fn by_id(&self, id: &str) -> Option<&Entry> {
        self.entry(id)
    }

    /// The entry that holds `slot`.
    pub fn by_slot(&self, slot: u32) -> Option<&Entry> {
        self.by_slot.get(&slot).map(|&i| &self.entries[i])
    }

    /// The entry whose top directory is `dir` (exact path match).
    pub fn by_dir(&self, dir: &Path) -> Option<&Entry> {
        self.by_dir.get(dir).map(|&i| &self.entries[i])
    }

    /// The entry whose manifest is `rel` relative to the root.
    pub fn by_rel_path(&self, rel: &Path) -> Option<&Entry> {
        self.entries.iter().find(|e| e.rel_path == rel)
    }

    /// The slot of `id`, if it holds one.
    pub fn slot_of(&self, id: &str) -> Option<u32> {
        self.entry(id).and_then(|e| e.slot)
    }

    /// The directories of kits with no known size, for a background
    /// [`measure_size`] → [`record_size`](Self::record_size) pass.
    pub fn unsized_dirs(&self) -> Vec<PathBuf> {
        self.entries
            .iter()
            .filter(|e| e.size_bytes.is_none())
            .map(|e| e.dir.clone())
            .collect()
    }

    /// Resolve free text to a slotted entry: an exact name (folded: case
    /// and diacritics), then an id prefix of at least 6 hex digits, then a
    /// unique name prefix, then a unique name substring. `None` when
    /// nothing or several match.
    pub fn find(&self, text: &str) -> Option<&Entry> {
        let t = text.trim();
        if t.is_empty() {
            return None;
        }
        use crate::library_marks::vocab::fold;
        let lower = fold(t);
        let slotted = || self.entries.iter().filter(|e| e.slot.is_some());
        let mut exact = slotted().filter(|e| fold(&e.name) == lower);
        match (exact.next(), exact.next()) {
            (Some(e), None) => return Some(e),
            (Some(_), Some(_)) => return None,
            _ => {}
        }
        if t.len() >= 6 && t.chars().all(|c| c.is_ascii_hexdigit()) {
            let hex = t.to_ascii_lowercase();
            let mut hits = slotted().filter(|e| e.id.starts_with(&hex));
            if let (Some(e), None) = (hits.next(), hits.next()) {
                return Some(e);
            }
        }
        let unique = |pred: &dyn Fn(&Entry) -> bool| {
            let mut hits = slotted().filter(|e| pred(e));
            match (hits.next(), hits.next()) {
                (Some(e), None) => Some(e),
                _ => None,
            }
        };
        unique(&|e: &Entry| fold(&e.name).starts_with(&lower))
            .or_else(|| unique(&|e: &Entry| fold(&e.name).contains(&lower)))
    }

    /// Re-read `library.json` if another writer changed it. Returns
    /// whether the entries changed. One `stat` when nothing moved.
    pub fn reload_if_changed(&mut self) -> bool {
        let Some(root) = &self.root else {
            return false;
        };
        let path = root.join(LIBRARY_FILE);
        let now = stat_stamp(&path);
        if now == self.stamp {
            return false;
        }
        let doc = read_index(&path, false);
        let changed = doc.generation != self.doc.generation;
        self.doc = doc;
        self.stamp = now;
        if changed {
            self.rebuild_entries();
        }
        changed
    }

    /// Run `f` on the index re-read under the `library.lock` file lock;
    /// when it returns `true` the index is written with a new generation.
    fn locked<R>(
        &mut self,
        root: &Path,
        f: impl FnOnce(&mut IndexDoc) -> Result<(R, bool), LibraryError>,
    ) -> Result<R, LibraryError> {
        let lock_path = root.join(LOCK_FILE);
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(io_err("open", &lock_path))?;
        let locked = crate::library_marks::lock_or_best_effort(&lock, &lock_path)
            .map_err(io_err("lock", &lock_path))?;
        let result = (|| {
            let index_path = root.join(LIBRARY_FILE);
            let mut doc = read_index(&index_path, true);
            let (out, changed) = f(&mut doc)?;
            if changed {
                doc.generation = doc.generation.wrapping_add(1);
                doc.version = INDEX_VERSION;
                let bytes = serde_json::to_vec_pretty(&doc)?;
                atomic_write(&index_path, &bytes)?;
            }
            self.doc = doc;
            self.stamp = stat_stamp(&index_path);
            self.rebuild_entries();
            Ok(out)
        })();
        if locked {
            let _ = lock.unlock();
        }
        result
    }

    /// Find the kits, re-hash only new or changed manifests, run the
    /// `installed.json` migration if it is due, assign and free slots,
    /// and write `library.json` if anything changed. Under the lock.
    pub fn rescan(&mut self) -> Result<ScanReport, LibraryError> {
        let root = self.root.clone().ok_or(LibraryError::NoRoot)?;
        if !root.is_dir() {
            // Nothing installed yet: an empty library, and nothing is
            // created until the first import or download.
            self.doc = IndexDoc::default();
            self.stamp = None;
            self.rebuild_entries();
            return Ok(ScanReport::default());
        }
        self.reload_if_changed();
        if !self.migration_due() && self.scan_is_current(&root) {
            return Ok(ScanReport::default());
        }
        let migration = self.migration.clone();
        self.locked(&root, |doc| {
            let report = rescan_doc(&root, doc, migration.as_ref())?;
            let changed = report.changed;
            Ok((report, changed))
        })
    }

    fn migration_due(&self) -> bool {
        self.migration.is_some() && !self.doc.installed_json_migrated
    }

    /// Store a size measured by [`measure_size`] for the kit in `dir`.
    /// Ignored when the index no longer knows the kit.
    pub fn record_size(&mut self, dir: &Path, bytes: u64) -> Result<(), LibraryError> {
        let root = self.root.clone().ok_or(LibraryError::NoRoot)?;
        if !root.is_dir() {
            return Ok(());
        }
        self.locked(&root, |doc| {
            let mut changed = false;
            for r in doc.kits.iter_mut().filter(|r| r.dir == dir) {
                if r.measured_size != Some(bytes) {
                    r.measured_size = Some(bytes);
                    changed = true;
                }
            }
            Ok(((), changed))
        })
    }

    /// Stat every sample `id`'s manifest names and record how many are
    /// missing in the entry's status ([`EntryStatus::MissingFiles`]).
    /// In memory only. Returns the count.
    pub fn check_missing_files(&mut self, id: &str) -> Result<usize, LibraryError> {
        let manifest = self
            .entry(id)
            .map(|e| e.manifest_path.clone())
            .ok_or_else(|| LibraryError::NotAKit {
                path: PathBuf::from(id),
                reason: "no such kit in the library".into(),
            })?;
        let n = missing_files(&manifest)?.len();
        self.missing.insert(id.to_string(), n);
        self.rebuild_entries();
        Ok(n)
    }

    /// Import a kit folder, or a `.zip` of one, by copying it into the
    /// root through `.staging/` (D2); a kit whose manifest is already in
    /// the library is not copied. Refused when the disk lacks
    /// [`IMPORT_SPACE_FACTOR`] × the kit's size. Cancel leaves nothing
    /// behind. Rescans.
    ///
    /// Blocks for the whole copy: run it on a background job. Only the
    /// final rescan touches the index.
    pub fn import(
        &mut self,
        src: &Path,
        mut job: ImportJob<'_>,
    ) -> Result<ImportOutcome, LibraryError> {
        if src.is_file()
            && src
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
        {
            return self.import_zip(src, job);
        }
        let root = self.root.clone().ok_or(LibraryError::NoRoot)?;
        let not_a_kit = |reason: &str| LibraryError::NotAKit {
            path: src.to_path_buf(),
            reason: reason.to_string(),
        };
        if !src.is_dir() {
            return Err(not_a_kit("not a folder or a .zip"));
        }
        let manifest =
            find_manifest(src).ok_or_else(|| not_a_kit("no drum_samples.json at depth 0 or 1"))?;
        let bytes = std::fs::read(&manifest).map_err(io_err("read", &manifest))?;
        summarize(&bytes).map_err(|e| not_a_kit(&e.0))?;
        let id = crate::nam_library::hash_bytes(&bytes);
        if root.is_dir() {
            self.rescan()?;
            if let Some(e) = self.entry(&id) {
                return Ok(ImportOutcome::AlreadyPresent(e.clone()));
            }
        }
        let files = install::walk_files(src).map_err(io_err("read", src))?;
        let total: u64 = files.iter().map(|(_, n)| n).sum();
        std::fs::create_dir_all(&root).map_err(io_err("mkdir", &root))?;
        job.check_space(&root, space_needed(total))?;
        let name = kit_dir_name(src.file_name().map(|n| n.to_string_lossy().into_owned()));
        let stage = staging_path(&root, &name);
        let result = (|| {
            install::copy_tree(src, &files, &stage, &mut job)?;
            let mut sc = read_sidecar(src).unwrap_or_else(|| Sidecar {
                source: SOURCE_IMPORTED.into(),
                ..Sidecar::default()
            });
            if sc.downloaded_at.is_none() {
                sc.downloaded_at =
                    crate::library_marks::format_timestamp(crate::library_marks::now_unix());
            }
            sc.size_bytes = Some(total);
            write_sidecar(&stage, &sc)?;
            promote(&root, &stage, &name)
        })();
        let dest = finish_staging(&root, &stage, result)?;
        self.added_entry(&dest, &id)
    }

    /// Import a `.zip` of a kit: [`install_zip`](Self::install_zip) under
    /// the zip's stem with an `imported` sidecar.
    pub fn import_zip(
        &mut self,
        zip: &Path,
        job: ImportJob<'_>,
    ) -> Result<ImportOutcome, LibraryError> {
        let name = zip
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let sidecar = Sidecar {
            source: SOURCE_IMPORTED.into(),
            ..Sidecar::default()
        };
        self.install_zip(zip, &name, sidecar, job)
    }

    /// Extract the kit in `zip` into `.staging/`, write `sidecar` (its
    /// `size_bytes` measured, `downloaded_at` defaulted to now), and
    /// rename it into the root as `name` (made unique). A zip whose kit
    /// sits one directory deeper than depth 1 (`Kit/kit/drum_samples.json`)
    /// is hoisted. A kit already in the library is discarded, not
    /// installed twice. The download worker (K3) installs through this.
    pub fn install_zip(
        &mut self,
        zip: &Path,
        name: &str,
        mut sidecar: Sidecar,
        mut job: ImportJob<'_>,
    ) -> Result<ImportOutcome, LibraryError> {
        let root = self.root.clone().ok_or(LibraryError::NoRoot)?;
        let file = File::open(zip).map_err(io_err("open", zip))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| LibraryError::Zip(format!("{}: {e}", zip.display())))?;
        let plan = install::zip_plan(&mut archive)?;
        let total: u64 = plan.iter().map(|(_, _, n)| n).sum();
        std::fs::create_dir_all(&root).map_err(io_err("mkdir", &root))?;
        job.check_space(&root, space_needed(total))?;
        let name = kit_dir_name(Some(name.to_string()));
        let stage = staging_path(&root, &name);
        let mut id = String::new();
        let mut duplicate = None;
        let result = (|| {
            let written = install::extract(&mut archive, &plan, &stage, &mut job)?;
            let manifest = match find_manifest(&stage) {
                Some(m) => m,
                None => {
                    hoist_single_subdir(&stage)?;
                    find_manifest(&stage).ok_or_else(|| LibraryError::NotAKit {
                        path: zip.to_path_buf(),
                        reason: "no drum_samples.json in the archive".into(),
                    })?
                }
            };
            let bytes = std::fs::read(&manifest).map_err(io_err("read", &manifest))?;
            summarize(&bytes).map_err(|e| LibraryError::NotAKit {
                path: zip.to_path_buf(),
                reason: e.0,
            })?;
            id = crate::nam_library::hash_bytes(&bytes);
            self.rescan()?;
            if let Some(e) = self.entry(&id) {
                duplicate = Some(e.clone());
                return Err(LibraryError::Cancelled);
            }
            if sidecar.downloaded_at.is_none() {
                sidecar.downloaded_at =
                    crate::library_marks::format_timestamp(crate::library_marks::now_unix());
            }
            sidecar.size_bytes = Some(written);
            write_sidecar(&stage, &sidecar)?;
            promote(&root, &stage, &name)
        })();
        match finish_staging(&root, &stage, result) {
            Ok(dest) => self.added_entry(&dest, &id),
            Err(_) if duplicate.is_some() => Ok(ImportOutcome::AlreadyPresent(duplicate.unwrap())),
            Err(e) => Err(e),
        }
    }

    fn added_entry(&mut self, dest: &Path, id: &str) -> Result<ImportOutcome, LibraryError> {
        self.rescan()?;
        let entry = self
            .by_dir(dest)
            .or_else(|| self.entry(id))
            .cloned()
            .ok_or_else(|| LibraryError::NotAKit {
                path: dest.to_path_buf(),
                reason: "copied kit did not index".into(),
            })?;
        Ok(ImportOutcome::Added(entry))
    }

    /// Delete the kit whose top directory is `dir` (`remove_dir_all`),
    /// then rescan, which frees its slot. Marks are not touched: they are
    /// kept for the orphan window. Returns the removed entry. Blocks for
    /// as long as the delete takes (a large kit: run it on a job).
    ///
    /// Refused — before anything is removed — unless `dir` is the top
    /// directory of a kit the index knows, directly inside the root after
    /// resolving symlinks and `..`.
    pub fn delete(&mut self, dir: &Path) -> Result<Entry, LibraryError> {
        let root = self.root.clone().ok_or(LibraryError::NoRoot)?;
        let outside = || LibraryError::OutsideLibrary {
            path: dir.to_path_buf(),
        };
        if dir
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(outside());
        }
        let entry = self.by_dir(dir).cloned().ok_or_else(outside)?;
        let canon_root = std::fs::canonicalize(&root).map_err(io_err("resolve", &root))?;
        let meta = std::fs::symlink_metadata(dir).map_err(io_err("stat", dir))?;
        if !meta.is_dir() {
            return Err(outside());
        }
        let canon = std::fs::canonicalize(dir).map_err(io_err("resolve", dir))?;
        if canon.parent() != Some(canon_root.as_path()) || canon.file_name().is_some_and(is_hidden)
        {
            return Err(outside());
        }
        std::fs::remove_dir_all(&canon).map_err(io_err("delete", dir))?;
        self.missing.remove(&entry.id);
        self.rescan()?;
        Ok(entry)
    }

    /// Whether a scan would find exactly what the index records: the same
    /// kits with the same manifest size, mtime and sidecar, and every
    /// distinct id slotted (while slots remain).
    fn scan_is_current(&self, root: &Path) -> bool {
        let kits = scan_kits(root);
        if kits.len() != self.doc.kits.len() {
            return false;
        }
        let by_dir: HashMap<&Path, &KitRecord> =
            self.doc.kits.iter().map(|r| (r.dir.as_path(), r)).collect();
        for (dir, manifest) in &kits {
            let Some(r) = by_dir.get(dir.as_path()) else {
                return false;
            };
            let Ok(meta) = std::fs::metadata(manifest) else {
                return false;
            };
            if r.manifest != *manifest
                || r.size != meta.len()
                || r.mtime_ns != mtime_ns(&meta)
                || r.sidecar != read_sidecar(dir)
            {
                return false;
            }
        }
        let slotted: HashSet<&str> = self.doc.slots.values().map(String::as_str).collect();
        let full = self.doc.slots.len() >= SLOT_COUNT as usize;
        full || self
            .doc
            .kits
            .iter()
            .all(|r| slotted.contains(r.id.as_str()))
    }

    fn rebuild_entries(&mut self) {
        let root = self.root.clone().unwrap_or_default();
        let slot_of: HashMap<&str, u32> = self
            .doc
            .slots
            .iter()
            .map(|(s, id)| (id.as_str(), *s))
            .collect();
        let mut first_dir: HashMap<&str, &Path> = HashMap::new();
        let mut records: Vec<&KitRecord> = self.doc.kits.iter().collect();
        records.sort_by(|a, b| a.dir.to_string_lossy().cmp(&b.dir.to_string_lossy()));
        let mut entries = Vec::with_capacity(records.len());
        for r in records {
            let duplicate_of = match first_dir.get(r.id.as_str()) {
                Some(p) => Some(p.to_path_buf()),
                None => {
                    first_dir.insert(r.id.as_str(), r.dir.as_path());
                    None
                }
            };
            let slot = if duplicate_of.is_none() {
                slot_of.get(r.id.as_str()).copied()
            } else {
                None
            };
            let missing = self.missing.get(&r.id).copied();
            entries.push(make_entry(&root, r, slot, duplicate_of, missing));
        }
        entries.sort_by(|a, b| match (a.slot, b.slot) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.dir.cmp(&b.dir),
        });
        self.by_id.clear();
        self.by_slot.clear();
        self.by_dir.clear();
        for (i, e) in entries.iter().enumerate() {
            if !matches!(e.status, EntryStatus::DuplicateOf(_)) {
                self.by_id.insert(e.id.clone(), i);
            }
            if let Some(s) = e.slot {
                self.by_slot.insert(s, i);
            }
            self.by_dir.insert(e.dir.clone(), i);
        }
        self.entries = entries;
    }
}

fn space_needed(bytes: u64) -> u64 {
    (bytes as f64 * IMPORT_SPACE_FACTOR).ceil() as u64
}

/// A safe directory name for a kit: path separators and leading dots
/// removed; `kit` when nothing is left.
fn kit_dir_name(raw: Option<String>) -> String {
    let raw = raw.unwrap_or_default();
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_start_matches('.').trim().to_string();
    if trimmed.is_empty() {
        "kit".into()
    } else {
        trimmed
    }
}

/// `.staging/<name>.<pid>-<n>`: unique per process and import, so two
/// imports of one name never share a staging directory.
fn staging_path(root: &Path, name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    root.join(STAGING_DIR)
        .join(format!("{name}.{}-{n}", std::process::id()))
}

/// Rename the staged kit into the root as `name`, or `name 2`, `name 3`, …
/// when taken. A rename onto an existing directory fails rather than
/// replacing it, so a racing writer never loses its kit.
fn promote(root: &Path, stage: &Path, name: &str) -> Result<PathBuf, LibraryError> {
    let mut n = 1;
    loop {
        let dest = if n == 1 {
            root.join(name)
        } else {
            root.join(format!("{name} {n}"))
        };
        if !dest.exists() {
            match std::fs::rename(stage, &dest) {
                Ok(()) => return Ok(dest),
                Err(e) if n > 100 => return Err(io_err("rename", &dest)(e)),
                Err(_) => {}
            }
        }
        n += 1;
    }
}

/// On failure (or cancel) remove the staged copy; either way remove
/// `.staging/` when it is left empty.
fn finish_staging(
    root: &Path,
    stage: &Path,
    result: Result<PathBuf, LibraryError>,
) -> Result<PathBuf, LibraryError> {
    if result.is_err() {
        let _ = std::fs::remove_dir_all(stage);
    }
    let _ = std::fs::remove_dir(root.join(STAGING_DIR));
    result
}

/// `stage/<only>/…` → `stage/…`, for a zip that wraps its kit one level
/// deeper than depth 1.
fn hoist_single_subdir(stage: &Path) -> Result<(), LibraryError> {
    let entries: Vec<_> = std::fs::read_dir(stage)
        .map_err(io_err("read", stage))?
        .flatten()
        .collect();
    let [only] = entries.as_slice() else {
        return Ok(());
    };
    if !only.file_type().is_ok_and(|t| t.is_dir()) {
        return Ok(());
    }
    let mut tmp = stage.as_os_str().to_os_string();
    tmp.push(".hoist");
    let tmp = PathBuf::from(tmp);
    std::fs::rename(only.path(), &tmp).map_err(io_err("rename", &tmp))?;
    std::fs::remove_dir(stage).map_err(io_err("rmdir", stage))?;
    std::fs::rename(&tmp, stage).map_err(io_err("rename", stage))?;
    Ok(())
}

/// The scan proper, on the index re-read under the lock.
fn rescan_doc(
    root: &Path,
    doc: &mut IndexDoc,
    migration: Option<&Migration>,
) -> Result<ScanReport, LibraryError> {
    let mut report = ScanReport::default();
    let now = crate::library_marks::now_unix();
    let mut files_changed = false;

    if let Some(m) = migration.filter(|_| !doc.installed_json_migrated) {
        report.migrated = migrate_installed(root, m);
        doc.installed_json_migrated = true;
        files_changed = true;
    }

    let cached: HashMap<PathBuf, KitRecord> =
        doc.kits.drain(..).map(|r| (r.dir.clone(), r)).collect();
    let kits = scan_kits(root);
    let seen: HashSet<&PathBuf> = kits.iter().map(|(d, _)| d).collect();
    if cached.keys().any(|p| !seen.contains(p)) {
        files_changed = true;
    }
    let mut records = Vec::with_capacity(kits.len());
    for (dir, manifest) in &kits {
        let Ok(meta) = std::fs::metadata(manifest) else {
            continue;
        };
        let (size, mtime) = (meta.len(), mtime_ns(&meta));
        let sidecar = read_sidecar(dir);
        match cached.get(dir) {
            Some(r) if r.manifest == *manifest && r.size == size && r.mtime_ns == mtime => {
                let mut r = r.clone();
                if r.sidecar != sidecar {
                    r.sidecar = sidecar;
                    files_changed = true;
                }
                records.push(r);
            }
            prev => {
                let Ok(bytes) = std::fs::read(manifest) else {
                    continue;
                };
                report.hashed += 1;
                files_changed = true;
                let id = crate::nam_library::hash_bytes(&bytes);
                let (summary, error) = match summarize(&bytes) {
                    Ok(s) => (Some(s), None),
                    Err(e) => (None, Some(e.0)),
                };
                records.push(KitRecord {
                    dir: dir.clone(),
                    manifest: manifest.clone(),
                    size,
                    mtime_ns: mtime,
                    first_seen: prev.map(|p| p.first_seen).unwrap_or(now),
                    measured_size: prev.filter(|p| p.id == id).and_then(|p| p.measured_size),
                    id,
                    summary,
                    error,
                    sidecar,
                });
            }
        }
    }

    // Canonical kit per id: the first in scan order.
    let mut canonical: Vec<&str> = Vec::new();
    let mut seen_ids: HashSet<&str> = HashSet::new();
    for r in &records {
        if seen_ids.insert(r.id.as_str()) {
            canonical.push(r.id.as_str());
        }
    }
    let live: HashSet<&str> = canonical.iter().copied().collect();
    let gone: Vec<(u32, String)> = doc
        .slots
        .iter()
        .filter(|(_, id)| !live.contains(id.as_str()))
        .map(|(s, id)| (*s, id.clone()))
        .collect();
    for (slot, id) in gone {
        doc.slots.remove(&slot);
        doc.retired.insert(id.clone(), slot);
        report.removed.push(id);
    }
    let slotted: HashSet<String> = doc.slots.values().cloned().collect();
    for id in canonical {
        if slotted.contains(id) {
            continue;
        }
        let reclaim = doc
            .retired
            .get(id)
            .copied()
            .filter(|s| !doc.slots.contains_key(s));
        if let Some(slot) = reclaim.or_else(|| allocate_slot(doc)) {
            doc.slots.insert(slot, id.to_string());
            doc.next_slot = doc.next_slot.max(slot + 1);
            doc.retired.remove(id);
            report.added.push(id.to_string());
        }
    }
    let held: HashSet<u32> = doc.slots.keys().copied().collect();
    doc.retired.retain(|_, s| !held.contains(s));

    doc.kits = records;
    report.changed = files_changed || !report.added.is_empty() || !report.removed.is_empty();
    Ok(report)
}

/// Next slot under the no-reuse rule: past the high-water mark while there
/// is room, else the lowest free one; `None` when all are taken.
fn allocate_slot(doc: &IndexDoc) -> Option<u32> {
    if doc.next_slot <= MAX_SLOT && !doc.slots.contains_key(&doc.next_slot) {
        return Some(doc.next_slot);
    }
    (0..SLOT_COUNT).find(|s| !doc.slots.contains_key(s))
}

/// `YYYY-MM-DD` (the registry's `installed_at`) → midnight UTC, RFC 3339.
fn date_to_rfc3339(date: &str) -> Option<String> {
    let mut parts = date.trim().splitn(3, '-');
    let y: i32 = parts.next()?.parse().ok()?;
    let m: u8 = parts.next()?.parse().ok()?;
    let d: u8 = parts.next()?.parse().ok()?;
    let date = time::Date::from_calendar_date(y, time::Month::try_from(m).ok()?, d).ok()?;
    let secs = date.midnight().assume_utc().unix_timestamp();
    crate::library_marks::format_timestamp(secs)
}

/// Write a sidecar for every `installed.json` drumkit item whose directory
/// is a kit under `root` without one. Matches by path, then by the item's
/// directory name (the root may have moved). Returns how many it wrote.
fn migrate_installed(root: &Path, m: &Migration) -> usize {
    use crate::registry::{load_registry_from, ContentType};
    if !m.installed_json.is_file() {
        return 0;
    }
    let reg = load_registry_from(&m.installed_json);
    let kits = scan_kits(root);
    let mut written = 0;
    for item in reg.items_of(&ContentType::Drumkit) {
        let item_path = PathBuf::from(&item.path);
        let canon_item = std::fs::canonicalize(&item_path).ok();
        let hit = kits.iter().map(|(d, _)| d).find(|d| {
            **d == item_path
                || (canon_item.is_some() && std::fs::canonicalize(d).ok() == canon_item)
        });
        let hit = hit.or_else(|| {
            let name = item_path.file_name()?;
            kits.iter()
                .map(|(d, _)| d)
                .find(|d| d.file_name() == Some(name))
        });
        let Some(dir) = hit else {
            continue;
        };
        if sidecar_path(dir).exists() {
            continue;
        }
        let plok = m.in_index.as_ref().is_some_and(|f| f(&item.name));
        let sc = Sidecar {
            source: if plok { SOURCE_PLOK } else { SOURCE_LOCAL }.into(),
            index_name: plok.then(|| item.name.clone()),
            downloaded_at: date_to_rfc3339(&item.installed_at),
            ..Sidecar::default()
        };
        match write_sidecar(dir, &sc) {
            Ok(()) => written += 1,
            Err(e) => tracing::warn!("installed.json migration: {}: {e}", dir.display()),
        }
    }
    written
}

/// Read `library.json`, tolerant by record (see `nam_library`'s reader):
/// a bad kit record is dropped and re-hashed; only a document that is not
/// JSON at all reads as empty, and it is quarantined only under the lock.
fn read_index(path: &Path, quarantine: bool) -> IndexDoc {
    let Ok(bytes) = std::fs::read(path) else {
        return IndexDoc::default();
    };
    let value: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => {
            if quarantine {
                tracing::error!(
                    "drum kit library index {} unreadable ({e}); rebuilding",
                    path.display()
                );
                quarantine_corrupt(path);
            }
            return IndexDoc::default();
        }
    };
    fn lenient<T: serde::de::DeserializeOwned>(v: &serde_json::Value, k: &str) -> Option<T> {
        serde_json::from_value(v.get(k)?.clone()).ok()
    }
    let kits = match value.get("kits") {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|r| match serde_json::from_value::<KitRecord>(r.clone()) {
                Ok(rec) => Some(rec),
                Err(e) => {
                    tracing::warn!("drum kit library index: dropping a bad kit record ({e})");
                    None
                }
            })
            .collect(),
        _ => Vec::new(),
    };
    IndexDoc {
        version: lenient(&value, "version").unwrap_or(INDEX_VERSION),
        generation: lenient(&value, "generation").unwrap_or(0),
        next_slot: lenient(&value, "next_slot").unwrap_or(0),
        slots: lenient(&value, "slots").unwrap_or_default(),
        retired: lenient(&value, "retired").unwrap_or_default(),
        installed_json_migrated: lenient(&value, "installed_json_migrated").unwrap_or(false),
        kits,
    }
}

fn make_entry(
    root: &Path,
    r: &KitRecord,
    slot: Option<u32>,
    duplicate_of: Option<PathBuf>,
    missing: Option<usize>,
) -> Entry {
    let sc = r.sidecar.as_ref();
    let summary = r.summary.clone().unwrap_or_default();
    let dir_name = r
        .dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = sc
        .and_then(|s| s.index_name.clone())
        .filter(|n| !n.trim().is_empty())
        .or_else(|| summary.meta_name.clone())
        .unwrap_or_else(|| dir_name.clone());
    let status = match (&duplicate_of, &r.error, missing) {
        (Some(p), _, _) => EntryStatus::DuplicateOf(p.clone()),
        (None, Some(err), _) => EntryStatus::ManifestError(err.clone()),
        (None, None, Some(n)) if n > 0 => EntryStatus::MissingFiles(n),
        _ => EntryStatus::Ok,
    };
    let added_at = sc
        .and_then(|s| s.downloaded_at.as_deref())
        .and_then(crate::library_marks::parse_timestamp)
        .unwrap_or(r.first_seen);
    let rel_path = r
        .manifest
        .strip_prefix(root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| r.manifest.clone());
    Entry {
        id: r.id.clone(),
        slot,
        name,
        dir: r.dir.clone(),
        dir_name,
        manifest_path: r.manifest.clone(),
        rel_path,
        source: Source::from_sidecar(sc),
        sidecar: r.sidecar.clone(),
        added_at,
        size_bytes: sc.and_then(|s| s.size_bytes).or(r.measured_size),
        pieces: summary.pieces,
        mic_setups: summary.mic_setups,
        articulations: summary.articulations,
        layers_max: summary.layers_max,
        rr_max: summary.rr_max,
        sample_count: summary.sample_count,
        status,
    }
}
