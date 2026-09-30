//! The per-user NAM model library (nam-model-library.md §4–§8): an index
//! of every installed `.nam` with its real metadata, content ids, and the
//! stable **slot table** the amp's `file_select` parameter indexes.
//!
//! ```text
//! <root>/                               (env override: RESONANCE_AMP_MODEL_DIR)
//!   tone3000/<name>_<model_id>.nam      downloads
//!   tone3000/<…>.nam.meta.json          provenance sidecar (Tone3000 metadata)
//!   imported/<file>.nam                 copies made by Import
//!   library.json                        index + slot table (a cache)
//!   library.lock                        File::lock for the rescan that writes it
//! ```
//!
//! The files and their sidecars are the truth; `library.json` is a cache
//! of hashes and headers plus the slot table. Deleting it costs a rescan
//! and re-assigns slots in migration order.
//!
//! **Identity** is the sha256 of the file bytes (D1): it survives renames
//! and moves and makes duplicates visible. It is computed once per
//! (path, size, mtime).
//!
//! **Slots** (D2, §5.1): `slots[n] = id`. A new model takes the slot after
//! the high-water mark; a freed slot is not reused while any slot above the
//! mark is free, so in practice slots are append-only until 1000 models.
//! A model that comes back (re-download, re-import) gets its old slot back
//! if it is still free. The first build assigns slots in today's
//! `scan_directory(tone3000/)` order, so existing `file_select` values keep
//! their meaning.
//!
//! **Concurrency** (§8): the rescan that allocates slots and writes
//! `library.json` runs under an exclusive `File::lock` on `library.lock`,
//! re-reading the index under the lock first, so two processes never put
//! two models in one slot. Nothing here may run on an audio thread.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::atomic_file::{atomic_write, quarantine_corrupt, AtomicWriteError};

mod header;
mod sidecar;

pub use header::{read_header, read_header_from, HeaderError, NamHeader, DEFAULT_SAMPLE_RATE};
pub use sidecar::{
    is_sidecar, read_sidecar, sidecar_path, write_sidecar, Sidecar, SIDECAR_SUFFIX,
    SOURCE_TONE3000,
};

/// The marks kind of a NAM model (`"amp-model:<sha256>"`).
pub const KIND: &str = crate::library_marks::kind::AMP_MODEL;

/// Overrides the library root. Tests use it (or the explicit-root API) to
/// stay out of the user's data dir.
pub const AMP_MODEL_DIR_ENV: &str = "RESONANCE_AMP_MODEL_DIR";

/// The root under the platform data dir.
pub const MODEL_SUBDIR: &str = "resonance/amp-models";

/// Where Tone3000 downloads land, under the root.
pub const TONE3000_DIR: &str = "tone3000";

/// Where Import copies files, under the root.
pub const IMPORTED_DIR: &str = "imported";

pub const LIBRARY_FILE: &str = "library.json";
pub const LOCK_FILE: &str = "library.lock";

/// How many slots `file_select` can address (`0..=MAX_SLOT`).
pub const SLOT_COUNT: u32 = 1000;
pub const MAX_SLOT: u32 = SLOT_COUNT - 1;

const INDEX_VERSION: u32 = 1;

/// The library root: [`AMP_MODEL_DIR_ENV`] if set and non-empty, else
/// `<data dir>/resonance/amp-models`.
pub fn default_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(AMP_MODEL_DIR_ENV).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    dirs::data_dir().map(|d| d.join(MODEL_SUBDIR))
}

/// The mark key of a model id.
pub fn mark_key(id: &str) -> String {
    crate::library_marks::mark_key(KIND, id)
}

/// sha256 of a file's bytes, lowercase hex. Streams in 64 KiB chunks.
pub fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// sha256 of a byte slice, lowercase hex.
pub fn hash_bytes(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Where a model came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// A Tone3000 download: `model_id` is the re-download key.
    Tone3000 { tone_id: i64, model_id: i64 },
    /// A copy made by Import (`imported/`).
    Imported,
    /// Anything else under the root (dropped in by hand).
    External,
}

impl Source {
    pub fn label(&self) -> &'static str {
        match self {
            Source::Tone3000 { .. } => "tone3000",
            Source::Imported => "imported",
            Source::External => "external",
        }
    }
}

/// Health of one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryStatus {
    Ok,
    /// The header did not parse; the reason.
    Unreadable(String),
    /// Same bytes as the entry at this path, which holds the slot.
    DuplicateOf(PathBuf),
}

/// One installed model, as the browser and the control API see it.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// sha256 of the bytes.
    pub id: String,
    /// The `file_select` value, if this entry holds one.
    pub slot: Option<u32>,
    pub path: PathBuf,
    pub file_name: String,
    /// Sidecar `tone_title` (· size) › `metadata.name` › file stem.
    pub name: String,
    pub author: Option<String>,
    pub gear: Option<String>,
    pub gear_type: Option<String>,
    pub tone_type: Option<String>,
    /// e.g. `WaveNet A2` ([`NamHeader::architecture_label`]).
    pub architecture: String,
    pub sample_rate: f64,
    pub size_bytes: u64,
    /// File mtime, Unix seconds.
    pub mtime: i64,
    /// Sidecar `downloaded_at`, else when the index first saw the file.
    pub added_at: i64,
    pub source: Source,
    pub esr: Option<f64>,
    pub loudness_db: Option<f64>,
    pub status: EntryStatus,
}

impl Entry {
    pub fn mark_key(&self) -> String {
        mark_key(&self.id)
    }

    pub fn is_ok(&self) -> bool {
        self.status == EntryStatus::Ok
    }
}

/// One file as the index remembers it (the hash/header cache).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileRecord {
    path: PathBuf,
    size: u64,
    mtime_ns: u64,
    id: String,
    first_seen: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    header: Option<HeaderRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sidecar: Option<Sidecar>,
}

/// The header fields the index keeps.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct HeaderRecord {
    architecture: String,
    #[serde(default)]
    version: Option<String>,
    sample_rate: f64,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    modeled_by: Option<String>,
    #[serde(default)]
    gear_type: Option<String>,
    #[serde(default)]
    gear_make: Option<String>,
    #[serde(default)]
    gear_model: Option<String>,
    #[serde(default)]
    tone_type: Option<String>,
    #[serde(default)]
    loudness: Option<f64>,
    #[serde(default)]
    validation_esr: Option<f64>,
}

impl HeaderRecord {
    fn from_header(h: &NamHeader) -> Self {
        Self {
            architecture: h.architecture.clone(),
            version: h.version.clone(),
            sample_rate: h.sample_rate,
            name: h.name.clone(),
            modeled_by: h.modeled_by.clone(),
            gear_type: h.gear_type.clone(),
            gear_make: h.gear_make.clone(),
            gear_model: h.gear_model.clone(),
            tone_type: h.tone_type.clone(),
            loudness: h.loudness,
            validation_esr: h.validation_esr,
        }
    }

    fn to_header(&self) -> NamHeader {
        NamHeader {
            version: self.version.clone(),
            architecture: self.architecture.clone(),
            sample_rate: self.sample_rate,
            sample_rate_declared: true,
            name: self.name.clone(),
            modeled_by: self.modeled_by.clone(),
            gear_type: self.gear_type.clone(),
            gear_make: self.gear_make.clone(),
            gear_model: self.gear_model.clone(),
            tone_type: self.tone_type.clone(),
            loudness: self.loudness,
            validation_esr: self.validation_esr,
            ..NamHeader::default()
        }
    }
}

/// `library.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct IndexDoc {
    version: u32,
    generation: u64,
    /// The slot after the highest ever handed out.
    next_slot: u32,
    /// slot → id.
    slots: BTreeMap<u32, String>,
    /// id → the slot it held before it disappeared, so a model that comes
    /// back gets it again while it is still free.
    #[serde(default)]
    retired: BTreeMap<String, u32>,
    files: Vec<FileRecord>,
}

impl Default for IndexDoc {
    fn default() -> Self {
        Self {
            version: INDEX_VERSION,
            generation: 0,
            next_slot: 0,
            slots: BTreeMap::new(),
            retired: BTreeMap::new(),
            files: Vec::new(),
        }
    }
}

/// Failure reading, scanning or changing the library.
#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("no model library directory (no data dir)")]
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
    #[error("{} is not a NAM model: {reason}", path.display())]
    NotAModel { path: PathBuf, reason: String },
    #[error("{} is not inside the model library", path.display())]
    OutsideLibrary { path: PathBuf },
}

fn io_err<'a>(op: &'static str, path: &'a Path) -> impl FnOnce(std::io::Error) -> LibraryError + 'a {
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
    /// Ids whose slot was freed (the file is gone).
    pub removed: Vec<String>,
    /// Files whose bytes were hashed this time (new or changed).
    pub hashed: usize,
    /// Whether `library.json` was rewritten.
    pub changed: bool,
}

/// What an import did.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportOutcome {
    /// Copied into `imported/`.
    Added(Entry),
    /// A file with the same bytes is already in the library (not copied).
    AlreadyPresent(Entry),
}

impl ImportOutcome {
    pub fn entry(&self) -> &Entry {
        match self {
            ImportOutcome::Added(e) | ImportOutcome::AlreadyPresent(e) => e,
        }
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

/// Sort rank of a file's directory: downloads first, then imports, then
/// anything else — the order duplicates are resolved in and new files are
/// slotted in.
fn dir_rank(root: &Path, path: &Path) -> u8 {
    match path.parent() {
        Some(p) if p == root.join(TONE3000_DIR) => 0,
        Some(p) if p == root.join(IMPORTED_DIR) => 1,
        _ => 2,
    }
}

/// The `.nam` files under `root`: the root itself and its immediate
/// subdirectories (`tone3000/`, `imported/`, anything the user made).
fn scan_files(root: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![root.to_path_buf()];
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                dirs.push(e.path());
            }
        }
    }
    let mut out = Vec::new();
    for dir in dirs {
        for p in crate::scan_directory(&dir, "nam") {
            let p = PathBuf::from(p);
            if !is_sidecar(&p) {
                out.push(p);
            }
        }
    }
    // Downloads first in `scan_directory` (string) order, which is what the
    // migration's "today's sorted order" means.
    out.sort_by(|a, b| {
        dir_rank(root, a)
            .cmp(&dir_rank(root, b))
            .then_with(|| a.to_string_lossy().cmp(&b.to_string_lossy()))
    });
    out
}

/// The model library: the index plus its derived entries.
#[derive(Debug, Clone)]
pub struct Library {
    root: Option<PathBuf>,
    doc: IndexDoc,
    stamp: Option<Stamp>,
    entries: Vec<Entry>,
    by_id: HashMap<String, usize>,
    by_slot: HashMap<u32, usize>,
    by_path: HashMap<PathBuf, usize>,
}

impl Library {
    /// A library with no root: empty, and every write fails with
    /// [`LibraryError::NoRoot`]. For a platform with no data dir.
    pub fn empty() -> Self {
        Self {
            root: None,
            doc: IndexDoc::default(),
            stamp: None,
            entries: Vec::new(),
            by_id: HashMap::new(),
            by_slot: HashMap::new(),
            by_path: HashMap::new(),
        }
    }

    /// Load the cached index under `root` without scanning. A missing
    /// index is an empty library; a corrupt one is quarantined
    /// (`library.json.corrupt`) and reads as empty (the next rescan
    /// rebuilds it).
    pub fn open(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let path = root.join(LIBRARY_FILE);
        let stamp = stat_stamp(&path);
        let doc = read_index(&path);
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

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub fn tone3000_dir(&self) -> Option<PathBuf> {
        self.root.as_ref().map(|r| r.join(TONE3000_DIR))
    }

    pub fn imported_dir(&self) -> Option<PathBuf> {
        self.root.as_ref().map(|r| r.join(IMPORTED_DIR))
    }

    /// The paths a [`crate::library_marks::FreshnessPoll`] should watch
    /// for this library: the root, its two subdirectories and the index.
    pub fn watch_paths(&self) -> Vec<PathBuf> {
        match &self.root {
            Some(r) => vec![
                r.clone(),
                r.join(TONE3000_DIR),
                r.join(IMPORTED_DIR),
                r.join(LIBRARY_FILE),
            ],
            None => Vec::new(),
        }
    }

    /// The index's write counter.
    pub fn generation(&self) -> u64 {
        self.doc.generation
    }

    /// Every entry, slotted ones first in slot order, then the rest
    /// (duplicates, the overflow past 1000) by path.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Total bytes on disk.
    pub fn total_bytes(&self) -> u64 {
        self.entries.iter().map(|e| e.size_bytes).sum()
    }

    /// The canonical entry of a content id.
    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.by_id.get(id).map(|&i| &self.entries[i])
    }

    /// The entry that holds `slot`.
    pub fn by_slot(&self, slot: u32) -> Option<&Entry> {
        self.by_slot.get(&slot).map(|&i| &self.entries[i])
    }

    /// The entry of the file at `path` (exact path match).
    pub fn by_path(&self, path: &Path) -> Option<&Entry> {
        self.by_path.get(path).map(|&i| &self.entries[i])
    }

    /// The slot of `id`, if it holds one.
    pub fn slot_of(&self, id: &str) -> Option<u32> {
        self.entry(id).and_then(|e| e.slot)
    }

    /// The entry of a Tone3000 model id, if it is installed.
    pub fn tone3000_model(&self, model_id: i64) -> Option<&Entry> {
        self.entries.iter().find(|e| {
            matches!(e.source, Source::Tone3000 { model_id: m, .. } if m == model_id)
                && !matches!(e.status, EntryStatus::DuplicateOf(_))
        })
    }

    /// Resolve free text to a slotted entry, for `string_to_value` and
    /// label addressing: an exact name (case-insensitive), then an id
    /// prefix of at least 6 hex digits, then a unique name prefix, then a
    /// unique name substring. `None` when nothing or several match.
    pub fn find(&self, text: &str) -> Option<&Entry> {
        let t = text.trim();
        if t.is_empty() {
            return None;
        }
        let lower = t.to_lowercase();
        let slotted = || self.entries.iter().filter(|e| e.slot.is_some());
        if let Some(e) = slotted().find(|e| e.name.to_lowercase() == lower) {
            return Some(e);
        }
        if t.len() >= 6 && t.chars().all(|c| c.is_ascii_hexdigit()) {
            let mut hits = slotted().filter(|e| e.id.starts_with(&lower));
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
        unique(&|e: &Entry| e.name.to_lowercase().starts_with(&lower))
            .or_else(|| unique(&|e: &Entry| e.name.to_lowercase().contains(&lower)))
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
        let doc = read_index(&path);
        let changed = doc.generation != self.doc.generation;
        self.doc = doc;
        self.stamp = now;
        if changed {
            self.rebuild_entries();
        }
        changed
    }

    /// Scan the files, re-hash only new or changed ones, assign and free
    /// slots, and write `library.json` if anything changed. Runs under the
    /// `library.lock` file lock, re-reading the index under it.
    pub fn rescan(&mut self) -> Result<ScanReport, LibraryError> {
        let root = self.root.clone().ok_or(LibraryError::NoRoot)?;
        if !root.is_dir() {
            // Nothing installed yet: an empty library, and nothing is
            // created until the first download or import.
            self.doc = IndexDoc::default();
            self.stamp = None;
            self.rebuild_entries();
            return Ok(ScanReport::default());
        }
        let lock_path = root.join(LOCK_FILE);
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(io_err("open", &lock_path))?;
        lock.lock().map_err(io_err("lock", &lock_path))?;
        let result = self.rescan_locked(&root);
        let _ = lock.unlock();
        result
    }

    fn rescan_locked(&mut self, root: &Path) -> Result<ScanReport, LibraryError> {
        let index_path = root.join(LIBRARY_FILE);
        let mut doc = read_index(&index_path);
        let mut report = ScanReport::default();
        let now = crate::library_marks::now_unix();

        let cached: HashMap<PathBuf, FileRecord> =
            doc.files.drain(..).map(|r| (r.path.clone(), r)).collect();
        let mut files_changed = false;
        let mut records: Vec<FileRecord> = Vec::new();
        let paths = scan_files(root);
        let seen: HashSet<&PathBuf> = paths.iter().collect();
        if cached.keys().any(|p| !seen.contains(p)) {
            files_changed = true;
        }
        for path in &paths {
            let Ok(meta) = std::fs::metadata(path) else {
                continue;
            };
            let (size, mtime) = (meta.len(), mtime_ns(&meta));
            let sidecar = read_sidecar(path);
            match cached.get(path) {
                Some(r) if r.size == size && r.mtime_ns == mtime => {
                    let mut r = r.clone();
                    if r.sidecar != sidecar {
                        r.sidecar = sidecar;
                        files_changed = true;
                    }
                    records.push(r);
                }
                prev => {
                    let Ok(id) = hash_file(path) else {
                        continue;
                    };
                    report.hashed += 1;
                    files_changed = true;
                    let (header, error) = match read_header(path) {
                        Ok(h) => (Some(HeaderRecord::from_header(&h)), None),
                        Err(e) => (None, Some(e.to_string())),
                    };
                    records.push(FileRecord {
                        path: path.clone(),
                        size,
                        mtime_ns: mtime,
                        id,
                        first_seen: prev.map(|p| p.first_seen).unwrap_or(now),
                        header,
                        error,
                        sidecar,
                    });
                }
            }
        }

        // Canonical file per id: the first in scan order.
        let mut canonical: Vec<&str> = Vec::new();
        let mut seen_ids: HashSet<&str> = HashSet::new();
        for r in &records {
            if seen_ids.insert(r.id.as_str()) {
                canonical.push(r.id.as_str());
            }
        }
        let live: HashSet<&str> = canonical.iter().copied().collect();

        // Free the slots of ids that are gone.
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
        // Slot the new ones, in scan order.
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
            let slot = reclaim.or_else(|| allocate_slot(&doc));
            if let Some(slot) = slot {
                doc.slots.insert(slot, id.to_string());
                doc.next_slot = doc.next_slot.max(slot + 1);
                doc.retired.remove(id);
                report.added.push(id.to_string());
            }
        }
        // Forget retired slots somebody else now holds.
        let held: HashSet<u32> = doc.slots.keys().copied().collect();
        doc.retired.retain(|_, s| !held.contains(s));

        doc.files = records;
        report.changed = files_changed || !report.added.is_empty() || !report.removed.is_empty();
        if report.changed {
            doc.generation = doc.generation.wrapping_add(1);
            doc.version = INDEX_VERSION;
            let bytes = serde_json::to_vec_pretty(&doc)?;
            atomic_write(&index_path, &bytes)?;
        }
        self.doc = doc;
        self.stamp = stat_stamp(&index_path);
        self.rebuild_entries();
        Ok(report)
    }

    /// Copy `src` into `imported/` after checking its header, unless a
    /// file with the same bytes is already in the library. Rescans.
    pub fn import(&mut self, src: &Path) -> Result<ImportOutcome, LibraryError> {
        let dir = self.imported_dir().ok_or(LibraryError::NoRoot)?;
        read_header(src).map_err(|e| LibraryError::NotAModel {
            path: src.to_path_buf(),
            reason: e.to_string(),
        })?;
        let id = hash_file(src).map_err(io_err("read", src))?;
        self.rescan()?;
        if let Some(e) = self.entry(&id) {
            return Ok(ImportOutcome::AlreadyPresent(e.clone()));
        }
        std::fs::create_dir_all(&dir).map_err(io_err("mkdir", &dir))?;
        let dest = unique_dest(&dir, src);
        let tmp = dest.with_extension("nam.part");
        std::fs::copy(src, &tmp).map_err(io_err("copy", src))?;
        std::fs::rename(&tmp, &dest).map_err(io_err("rename", &dest))?;
        self.rescan()?;
        let entry = self
            .by_path(&dest)
            .or_else(|| self.entry(&id))
            .cloned()
            .ok_or_else(|| LibraryError::NotAModel {
                path: dest.clone(),
                reason: "copied file did not index".into(),
            })?;
        Ok(ImportOutcome::Added(entry))
    }

    /// Delete the model file at `path` and its sidecar, then rescan (which
    /// frees the slot if no other file has the same bytes). Only files
    /// inside the root. Marks are not touched: they are kept for the
    /// orphan window. Returns the removed entry.
    pub fn delete(&mut self, path: &Path) -> Result<Entry, LibraryError> {
        let root = self.root.clone().ok_or(LibraryError::NoRoot)?;
        if !path.starts_with(&root) {
            return Err(LibraryError::OutsideLibrary {
                path: path.to_path_buf(),
            });
        }
        let entry = self.by_path(path).cloned();
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err("delete", path)(e)),
        }
        let _ = std::fs::remove_file(sidecar_path(path));
        self.rescan()?;
        entry.ok_or_else(|| LibraryError::Io {
            op: "delete",
            path: path.to_path_buf(),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        })
    }

    fn rebuild_entries(&mut self) {
        let root = self.root.clone().unwrap_or_default();
        let slot_of: HashMap<&str, u32> = self
            .doc
            .slots
            .iter()
            .map(|(s, id)| (id.as_str(), *s))
            .collect();
        let mut first_path: HashMap<&str, &Path> = HashMap::new();
        let mut entries = Vec::with_capacity(self.doc.files.len());
        let mut records: Vec<&FileRecord> = self.doc.files.iter().collect();
        records.sort_by(|a, b| {
            dir_rank(&root, &a.path)
                .cmp(&dir_rank(&root, &b.path))
                .then_with(|| a.path.to_string_lossy().cmp(&b.path.to_string_lossy()))
        });
        for r in records {
            let duplicate_of = match first_path.get(r.id.as_str()) {
                Some(p) => Some(p.to_path_buf()),
                None => {
                    first_path.insert(r.id.as_str(), r.path.as_path());
                    None
                }
            };
            let slot = if duplicate_of.is_none() {
                slot_of.get(r.id.as_str()).copied()
            } else {
                None
            };
            entries.push(make_entry(&root, r, slot, duplicate_of));
        }
        entries.sort_by(|a, b| match (a.slot, b.slot) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.path.cmp(&b.path),
        });
        self.by_id.clear();
        self.by_slot.clear();
        self.by_path.clear();
        for (i, e) in entries.iter().enumerate() {
            if !matches!(e.status, EntryStatus::DuplicateOf(_)) {
                self.by_id.insert(e.id.clone(), i);
            }
            if let Some(s) = e.slot {
                self.by_slot.insert(s, i);
            }
            self.by_path.insert(e.path.clone(), i);
        }
        self.entries = entries;
    }
}

/// Next slot under the no-reuse rule: past the high-water mark while there
/// is room, else the lowest free one; `None` when all 1000 are taken.
fn allocate_slot(doc: &IndexDoc) -> Option<u32> {
    if doc.next_slot <= MAX_SLOT && !doc.slots.contains_key(&doc.next_slot) {
        return Some(doc.next_slot);
    }
    (0..SLOT_COUNT).find(|s| !doc.slots.contains_key(s))
}

fn unique_dest(dir: &Path, src: &Path) -> PathBuf {
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "model".into());
    let mut dest = dir.join(format!("{stem}.nam"));
    let mut n = 2;
    while dest.exists() {
        dest = dir.join(format!("{stem}-{n}.nam"));
        n += 1;
    }
    dest
}

fn read_index(path: &Path) -> IndexDoc {
    let Ok(bytes) = std::fs::read(path) else {
        return IndexDoc::default();
    };
    match serde_json::from_slice::<IndexDoc>(&bytes) {
        Ok(doc) => doc,
        Err(e) => {
            tracing::error!("model library index {} unreadable ({e}); rebuilding", path.display());
            quarantine_corrupt(path);
            IndexDoc::default()
        }
    }
}

fn make_entry(root: &Path, r: &FileRecord, slot: Option<u32>, duplicate_of: Option<PathBuf>) -> Entry {
    let header = r.header.as_ref().map(HeaderRecord::to_header);
    let sc = r.sidecar.as_ref();
    let file_name = r
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = r
        .path
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let sidecar_name = sc.and_then(|s| s.tone_title.clone()).map(|t| match sc.and_then(|s| s.size.as_deref()) {
        Some(size) if !size.is_empty() => format!("{t} · {size}"),
        _ => t,
    });
    let name = sidecar_name
        .or_else(|| header.as_ref().and_then(|h| h.name.clone()))
        .unwrap_or(stem);
    let source = match sc {
        Some(s) if s.source == SOURCE_TONE3000 => match (s.tone_id, s.model_id) {
            (Some(tone_id), Some(model_id)) => Source::Tone3000 { tone_id, model_id },
            _ => Source::External,
        },
        _ if r.path.parent() == Some(root.join(IMPORTED_DIR).as_path()) => Source::Imported,
        _ => Source::External,
    };
    let status = match (&duplicate_of, &r.error) {
        (Some(p), _) => EntryStatus::DuplicateOf(p.clone()),
        (None, Some(err)) => EntryStatus::Unreadable(err.clone()),
        (None, None) => EntryStatus::Ok,
    };
    let added_at = sc
        .and_then(|s| s.downloaded_at.as_deref())
        .and_then(crate::library_marks::parse_timestamp)
        .unwrap_or(r.first_seen);
    Entry {
        id: r.id.clone(),
        slot,
        path: r.path.clone(),
        file_name,
        name,
        author: sc
            .and_then(|s| s.author.clone())
            .or_else(|| header.as_ref().and_then(|h| h.modeled_by.clone())),
        gear: header
            .as_ref()
            .and_then(NamHeader::gear)
            .or_else(|| sc.and_then(|s| s.gear.clone())),
        gear_type: header.as_ref().and_then(|h| h.gear_type.clone()),
        tone_type: header.as_ref().and_then(|h| h.tone_type.clone()),
        architecture: header
            .as_ref()
            .map(NamHeader::architecture_label)
            .unwrap_or_else(|| "?".into()),
        sample_rate: header
            .as_ref()
            .map(|h| h.sample_rate)
            .unwrap_or(DEFAULT_SAMPLE_RATE),
        size_bytes: r.size,
        mtime: (r.mtime_ns / 1_000_000_000) as i64,
        added_at,
        source,
        esr: header.as_ref().and_then(|h| h.validation_esr),
        loudness_db: header.as_ref().and_then(|h| h.loudness),
        status,
    }
}
