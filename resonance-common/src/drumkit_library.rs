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
//! progress callback and a disk check ([`ImportJob`]). Every staged file
//! is fsynced before the rename. A staging directory left by a process
//! that died (or older than [`STAGING_MAX_AGE`]) is swept by the next
//! rescan. Importing from a `.zip` needs the `drumkit-zip` feature.
//!
//! **`installed.json`** (D3): on the first scan of a library given an
//! `installed.json` path ([`Library::with_installed_json`]), each drumkit
//! item whose directory is a kit with no sidecar gets one, carrying its
//! `installed_at`. Its source is `plok` when the supplied lookup finds the
//! name in the plok.org index (and the sidecar then carries the index
//! entry's `file`, the re-download key), else `local`. It runs once per
//! index (a flag in `library.json`) and only reads `installed.json`: an
//! unreadable one is skipped, never quarantined or rewritten.
//!
//! **Concurrency**: every write of `library.json` runs under an exclusive
//! `File::lock` on `library.lock`, re-reading the index under the lock
//! first. Nothing here may run on an audio thread.
//!
//! The slot table, index I/O, lock and lookup are
//! [`crate::content_index`], shared with the NAM library; this module is
//! what a kit is (code review ARCH2-04).

use std::collections::{BTreeMap, HashMap, HashSet};
#[cfg(feature = "drumkit-zip")]
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::atomic_file::AtomicWriteError;
use crate::content_index::{
    self, canonical_ids, hash_bytes, mtime_ns, slots_and_duplicates, stat_stamp, Entries,
    IndexFile, IndexedEntry, SlotTable,
};
pub use crate::content_index::{LIBRARY_FILE, LOCK_FILE, MAX_SLOT, SLOT_COUNT};

mod install;
mod manifest;
mod sidecar;

pub use install::{free_space, measure_size, ImportJob, ImportProgress};
pub use manifest::{
    sample_paths, summarize, Articulation, KitMeta, ManifestError, ManifestSummary, MicKind,
    MicSetupInfo, PadHint, Piece, PortHint, MANIFEST_FILE, META_KEY,
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

/// A staging directory older than this is swept even when the process
/// that made it still runs (its pid may have been reused).
pub const STAGING_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Import needs this much free space per byte copied.
pub const IMPORT_SPACE_FACTOR: f64 = 1.1;

/// The library root: [`DRUMKIT_DIR_ENV`] if set and non-empty, else
/// `<data dir>/resonance/drumkits`.
pub fn default_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(DRUMKIT_DIR_ENV).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    dirs::data_dir().map(|d| d.join(KIT_SUBDIR))
}

/// Where the retired installed-content registry lived
/// (`<data dir>/resonance/installed.json`), the migration source.
pub fn default_installed_json() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("resonance/installed.json"))
}

/// One drum-kit item of the retired `installed.json`
/// (`{"items":[{"name","type","path","installed_at"}]}`). Only the
/// migration reads the file; nothing writes it any more (D3).
#[derive(Debug, Clone, Deserialize)]
struct InstalledJsonItem {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    path: String,
    installed_at: String,
}

/// The `installed.json` document, items kept raw so one item of a type or
/// shape this build does not know (the retired `amp-model`) is skipped
/// rather than failing the whole file.
#[derive(Debug, Deserialize)]
struct InstalledJson {
    #[serde(default)]
    items: Vec<serde_json::Value>,
}

/// The drum-kit items of an `installed.json` document. `Err` when it is
/// not JSON of that shape.
fn installed_json_drumkits(bytes: &[u8]) -> Result<Vec<InstalledJsonItem>, serde_json::Error> {
    let doc: InstalledJson = serde_json::from_slice(bytes)?;
    Ok(doc
        .items
        .into_iter()
        .filter_map(|v| serde_json::from_value::<InstalledJsonItem>(v).ok())
        .filter(|item| item.kind == "drumkit")
        .collect())
}

/// Whether the `installed.json` at `path` lists any drum kit. Only reads:
/// a missing or corrupt file is `false` and is never quarantined.
pub fn installed_json_lists_drumkits(path: &Path) -> bool {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| installed_json_drumkits(&bytes).ok())
        .is_some_and(|items| !items.is_empty())
}

/// The mark key of a kit id.
pub fn mark_key(id: &str) -> String {
    crate::library_marks::mark_key(KIND, id)
}

/// sha256 of a manifest's bytes, lowercase hex: the kit id.
pub fn hash_manifest(path: &Path) -> std::io::Result<String> {
    content_index::hash_file(path)
}

/// The manifest of the kit in `kit_dir`: `kit_dir/drum_samples.json`, else
/// the first (by name) non-hidden subdirectory holding one.
pub fn find_manifest(kit_dir: &Path) -> Option<PathBuf> {
    let direct = kit_dir.join(MANIFEST_FILE);
    if direct.is_file() {
        return Some(direct);
    }
    depth1_manifests(kit_dir).into_iter().next()
}

/// The manifests one level down in `dir`: `dir/<sub>/drum_samples.json`
/// for each non-hidden subdirectory, by name.
fn depth1_manifests(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut subs: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && !is_hidden(&e.file_name()))
        .map(|e| e.path())
        .collect();
    subs.sort();
    subs.into_iter()
        .map(|s| s.join(MANIFEST_FILE))
        .filter(|m| m.is_file())
        .collect()
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

impl IndexedEntry for Entry {
    fn id(&self) -> &str {
        &self.id
    }
    fn slot(&self) -> Option<u32> {
        self.slot
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn is_duplicate(&self) -> bool {
        matches!(self.status, EntryStatus::DuplicateOf(_))
    }
    fn key_path(&self) -> &Path {
        &self.dir
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

/// `library.json`: the shared slot table, the migration flag, then the
/// kit records.
#[derive(Debug, Clone, Default, Serialize)]
struct IndexDoc {
    #[serde(flatten)]
    table: SlotTable,
    /// Whether the one-time `installed.json` migration has run.
    installed_json_migrated: bool,
    kits: Vec<KitRecord>,
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
    /// A folder (or zip) with no manifest of its own and several kits one
    /// level down: each is imported on its own.
    #[error("{} holds {count} kits — import each one", path.display())]
    MultipleKits { path: PathBuf, count: usize },
    /// A symlinked folder inside an imported kit that points outside it or
    /// loops.
    #[error("{} {reason}", path.display())]
    UnsafeLink { path: PathBuf, reason: &'static str },
    /// A `.zip` was given to a build without the `drumkit-zip` feature.
    #[error("{}: importing a .zip is not supported by this build", path.display())]
    ZipUnsupported { path: PathBuf },
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

/// What an import did. `AlreadyPresent`: a kit with the same manifest is
/// already in the library; nothing was left behind.
pub type ImportOutcome = content_index::ImportOutcome<Entry>;

/// What the plok.org index says about a kit name, for the `installed.json`
/// migration: a hit makes the kit `source = plok` and carries the entry's
/// fields into its sidecar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexMatch {
    /// The index's name for the kit; the `installed.json` name when `None`.
    pub index_name: Option<String>,
    /// The entry's `file` (the zip's name): the re-download key.
    pub index_file: Option<String>,
    /// sha256 of the zip, when the index gives one.
    pub sha256: Option<String>,
    pub description: Option<String>,
    pub index_tags: Vec<String>,
}

/// Looks a kit name up in the plok.org index, for the `installed.json`
/// migration; `None` when the name is not in it.
pub type InIndexFn = dyn Fn(&str) -> Option<IndexMatch> + Send + Sync;

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
    index: IndexFile<IndexDoc>,
    entries: Entries<Entry>,
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
            index: IndexFile {
                root: None,
                stamp: None,
                doc: IndexDoc::default(),
            },
            entries: Entries::default(),
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
        let mut lib = Self {
            index: IndexFile {
                stamp: stat_stamp(&path),
                doc: read_index(&path, false),
                root: Some(root),
            },
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
    /// next scan, once per index (see the module docs). `in_index` looks a
    /// kit name up in the plok.org index (a hit → `source = plok`, with
    /// the entry's `file`, sha256, description and tags); without one
    /// every migrated kit is `local`.
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
        self.index.root.as_deref()
    }

    pub fn staging_dir(&self) -> Option<PathBuf> {
        self.index.root.as_ref().map(|r| r.join(STAGING_DIR))
    }

    /// The paths a [`crate::library_marks::FreshnessPoll`] should watch:
    /// the root (a kit added or removed) and the index.
    pub fn watch_paths(&self) -> Vec<PathBuf> {
        match &self.index.root {
            Some(r) => vec![r.clone(), r.join(LIBRARY_FILE)],
            None => Vec::new(),
        }
    }

    /// The index's write counter.
    pub fn generation(&self) -> u64 {
        self.index.doc.table.generation
    }

    /// Every entry, slotted ones first in slot order, then the rest
    /// (duplicates, the overflow past 1000) by directory.
    pub fn entries(&self) -> &[Entry] {
        &self.entries.list
    }

    pub fn len(&self) -> usize {
        self.entries.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.list.is_empty()
    }

    /// Total known bytes on disk (kits not yet measured count 0).
    pub fn total_bytes(&self) -> u64 {
        self.entries.list.iter().filter_map(|e| e.size_bytes).sum()
    }

    /// The canonical entry of a kit id.
    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.by_id(id)
    }

    /// [`entry`](Self::entry) (the spec's name).
    pub fn by_id(&self, id: &str) -> Option<&Entry> {
        self.entry(id)
    }

    /// The entry that holds `slot`.
    pub fn by_slot(&self, slot: u32) -> Option<&Entry> {
        self.entries.by_slot(slot)
    }

    /// The entry whose top directory is `dir` (exact path match).
    pub fn by_dir(&self, dir: &Path) -> Option<&Entry> {
        self.entries.by_key(dir)
    }

    /// The entry whose manifest is `rel` relative to the root.
    pub fn by_rel_path(&self, rel: &Path) -> Option<&Entry> {
        self.entries.list.iter().find(|e| e.rel_path == rel)
    }

    /// The slot of `id`, if it holds one.
    pub fn slot_of(&self, id: &str) -> Option<u32> {
        self.entry(id).and_then(|e| e.slot)
    }

    /// The directories of kits with no known size, for a background
    /// [`measure_size`] → [`record_size`](Self::record_size) pass.
    pub fn unsized_dirs(&self) -> Vec<PathBuf> {
        self.entries
            .list
            .iter()
            .filter(|e| e.size_bytes.is_none())
            .map(|e| e.dir.clone())
            .collect()
    }

    /// Resolve free text to a slotted entry ([`content_index`]'s lookup):
    /// an exact name (folded: case and diacritics), then an id prefix of
    /// at least 6 hex digits, then a unique name prefix, then a unique
    /// name substring. `None` when nothing or several match.
    pub fn find(&self, text: &str) -> Option<&Entry> {
        self.entries.find(text)
    }

    /// Re-read `library.json` if its stamp (size, mtime) moved, and rebuild
    /// the entries from it. Returns whether it was re-read. One `stat` when
    /// nothing moved.
    pub fn reload_if_changed(&mut self) -> bool {
        let reread = self.index.reload_if_changed(|p| read_index(p, false));
        if reread {
            self.rebuild_entries();
        }
        reread
    }

    /// Run `f` on the index re-read under the `library.lock` file lock;
    /// when it returns `true` the index is written with a new generation.
    fn locked<R>(
        &mut self,
        root: &Path,
        f: impl FnOnce(&mut IndexDoc) -> Result<(R, bool), LibraryError>,
    ) -> Result<R, LibraryError> {
        with_lock(root, || {
            let index_path = root.join(LIBRARY_FILE);
            let mut doc = read_index(&index_path, true);
            let (out, changed) = f(&mut doc)?;
            if changed {
                doc.table.bump();
                content_index::write_index::<LibraryError>(&index_path, &doc)?;
            }
            self.index.doc = doc;
            self.index.stamp = stat_stamp(&index_path);
            self.rebuild_entries();
            Ok(out)
        })
    }

    /// Find the kits, re-hash only new or changed manifests, run the
    /// `installed.json` migration if it is due, assign and free slots,
    /// and write `library.json` if anything changed. Under the lock.
    pub fn rescan(&mut self) -> Result<ScanReport, LibraryError> {
        let root = self.index.root.clone().ok_or(LibraryError::NoRoot)?;
        if !root.is_dir() {
            // Nothing installed yet: an empty library, and nothing is
            // created until the first import or download.
            self.index.doc = IndexDoc::default();
            self.index.stamp = None;
            self.rebuild_entries();
            return Ok(ScanReport::default());
        }
        if root.join(STAGING_DIR).exists() {
            with_lock(&root, || {
                sweep_staging(&root);
                Ok(())
            })?;
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
        self.migration.is_some() && !self.index.doc.installed_json_migrated
    }

    /// Store a size measured by [`measure_size`] for the kit in `dir`.
    /// Ignored when the index no longer knows the kit.
    pub fn record_size(&mut self, dir: &Path, bytes: u64) -> Result<(), LibraryError> {
        let root = self.index.root.clone().ok_or(LibraryError::NoRoot)?;
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
    /// [`IMPORT_SPACE_FACTOR`] × the kit's size, and when the folder has
    /// no manifest of its own but several kits one level down
    /// ([`LibraryError::MultipleKits`]). Symlinked files are copied as
    /// their targets; a symlinked folder that escapes the kit or loops is
    /// refused ([`LibraryError::UnsafeLink`]). Cancel leaves nothing
    /// behind. Rescans.
    ///
    /// Blocks for the whole copy: run it on a background job. Only the
    /// rescans touch the index.
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
        let root = self.index.root.clone().ok_or(LibraryError::NoRoot)?;
        let not_a_kit = |reason: &str| LibraryError::NotAKit {
            path: src.to_path_buf(),
            reason: reason.to_string(),
        };
        if !src.is_dir() {
            return Err(not_a_kit("not a folder or a .zip"));
        }
        if !src.join(MANIFEST_FILE).is_file() {
            let count = depth1_manifests(src).len();
            if count > 1 {
                return Err(LibraryError::MultipleKits {
                    path: src.to_path_buf(),
                    count,
                });
            }
        }
        let manifest =
            find_manifest(src).ok_or_else(|| not_a_kit("no drum_samples.json at depth 0 or 1"))?;
        let bytes = std::fs::read(&manifest).map_err(io_err("read", &manifest))?;
        summarize(&bytes).map_err(|e| not_a_kit(&e.0))?;
        let id = hash_bytes(&bytes);
        self.rescan()?;
        if let Some(e) = self.entry(&id) {
            return Ok(ImportOutcome::AlreadyPresent(e.clone()));
        }
        let files = install::walk_files(src)?;
        let total: u64 = files.iter().map(|(_, n)| n).sum();
        std::fs::create_dir_all(&root).map_err(io_err("mkdir", &root))?;
        job.check_space(&root, space_needed(total))?;
        let name = kit_dir_name(src.file_name().map(|n| n.to_string_lossy().into_owned()));
        let stage = fresh_stage(&root, &name)?;
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

    /// Install the kit in `zip` as `name` (made unique): its manifest is
    /// read straight from the archive first, and a kit already in the
    /// library returns [`ImportOutcome::AlreadyPresent`] before any disk
    /// check or extraction. Otherwise the zip is extracted into
    /// `.staging/`, `sidecar` written (its `size_bytes` measured,
    /// `downloaded_at` defaulted to now), and the result renamed into the
    /// root.
    ///
    /// The manifest may sit at depth 0 or 1, or at depth 1 inside a single
    /// wrapper directory (`Kit/kit/drum_samples.json`), which is stripped.
    /// macOS litter (`__MACOSX/`, `.DS_Store`, any dotfile) is not
    /// extracted, and an entry that inflates past its declared size fails
    /// the install. The download worker (K3) installs through this.
    ///
    /// Without the `drumkit-zip` feature it fails with
    /// [`LibraryError::ZipUnsupported`].
    pub fn install_zip(
        &mut self,
        zip: &Path,
        name: &str,
        sidecar: Sidecar,
        job: ImportJob<'_>,
    ) -> Result<ImportOutcome, LibraryError> {
        #[cfg(feature = "drumkit-zip")]
        {
            self.install_zip_impl(zip, name, sidecar, job)
        }
        #[cfg(not(feature = "drumkit-zip"))]
        {
            let _ = (name, sidecar, job);
            Err(LibraryError::ZipUnsupported {
                path: zip.to_path_buf(),
            })
        }
    }

    #[cfg(feature = "drumkit-zip")]
    fn install_zip_impl(
        &mut self,
        zip: &Path,
        name: &str,
        sidecar: Sidecar,
        mut job: ImportJob<'_>,
    ) -> Result<ImportOutcome, LibraryError> {
        let root = self.index.root.clone().ok_or(LibraryError::NoRoot)?;
        let mut opened = OpenedZip::open(zip)?;
        self.rescan()?;
        if let Some(e) = self.entry(&opened.id) {
            return Ok(ImportOutcome::AlreadyPresent(e.clone()));
        }
        std::fs::create_dir_all(&root).map_err(io_err("mkdir", &root))?;
        job.check_space(&root, space_needed(opened.plan.total()))?;
        let name = kit_dir_name(Some(name.to_string()));
        let stage = fresh_stage(&root, &name)?;
        let result = opened
            .stage(&stage, sidecar, &mut job)
            .and_then(|()| promote(&root, &stage, &name));
        let dest = finish_staging(&root, &stage, result)?;
        let id = opened.id;
        self.added_entry(&dest, &id)
    }

    /// Re-download (K3): replace the kit whose top directory is
    /// `existing_dir` with the kit in `zip`, in place. The zip is extracted
    /// and `sidecar` written in `.staging/` first; then the old directory
    /// is renamed aside (into `.staging/`, hidden), the new one renamed to
    /// its name, and only then the old one deleted. A failure at any step
    /// puts the old kit back. The directory keeps its name, so when the
    /// manifest is unchanged (a repair of a kit with missing files) the id,
    /// slot and marks all stay; a changed manifest is a new id and slot.
    /// Refused like [`delete`](Self::delete) unless `existing_dir` is a
    /// kit the index knows, directly inside the root.
    ///
    /// Without the `drumkit-zip` feature it fails with
    /// [`LibraryError::ZipUnsupported`].
    pub fn install_zip_replacing(
        &mut self,
        zip: &Path,
        existing_dir: &Path,
        sidecar: Sidecar,
        job: ImportJob<'_>,
    ) -> Result<Entry, LibraryError> {
        #[cfg(feature = "drumkit-zip")]
        {
            self.install_zip_replacing_impl(zip, existing_dir, sidecar, job)
        }
        #[cfg(not(feature = "drumkit-zip"))]
        {
            let _ = (existing_dir, sidecar, job);
            Err(LibraryError::ZipUnsupported {
                path: zip.to_path_buf(),
            })
        }
    }

    #[cfg(feature = "drumkit-zip")]
    fn install_zip_replacing_impl(
        &mut self,
        zip: &Path,
        existing_dir: &Path,
        sidecar: Sidecar,
        mut job: ImportJob<'_>,
    ) -> Result<Entry, LibraryError> {
        let root = self.index.root.clone().ok_or(LibraryError::NoRoot)?;
        let (old, canon) = self.kit_dir_in_root(&root, existing_dir)?;
        let mut opened = OpenedZip::open(zip)?;
        job.check_space(&root, space_needed(opened.plan.total()))?;
        let stage = fresh_stage(&root, &old.dir_name)?;
        let result = opened.stage(&stage, sidecar, &mut job).and_then(|()| {
            let aside = staging_path(&root, &format!("{ASIDE_PREFIX}{}", old.dir_name));
            std::fs::rename(&canon, &aside).map_err(io_err("rename", &canon))?;
            if let Err(e) = std::fs::rename(&stage, &canon) {
                if let Err(back) = std::fs::rename(&aside, &canon) {
                    tracing::error!(
                        "re-download: could not restore {} from {}: {back}",
                        canon.display(),
                        aside.display()
                    );
                }
                return Err(io_err("rename", &stage)(e));
            }
            if let Err(e) = std::fs::remove_dir_all(&aside) {
                // The next rescan's sweep removes it.
                tracing::warn!("re-download: remove {}: {e}", aside.display());
            }
            Ok(canon.clone())
        });
        finish_staging(&root, &stage, result)?;
        self.missing.remove(&old.id);
        self.rescan()?;
        self.by_dir(existing_dir)
            .cloned()
            .ok_or_else(|| LibraryError::NotAKit {
                path: existing_dir.to_path_buf(),
                reason: "re-downloaded kit did not index".into(),
            })
    }

    /// The entry of a promoted kit, after a rescan. When it does not index
    /// the promoted directory is removed again: an import either lands in
    /// the library or leaves nothing.
    fn added_entry(&mut self, dest: &Path, id: &str) -> Result<ImportOutcome, LibraryError> {
        let found = self.rescan().and_then(|_| {
            self.by_dir(dest)
                .or_else(|| self.entry(id))
                .cloned()
                .ok_or_else(|| LibraryError::NotAKit {
                    path: dest.to_path_buf(),
                    reason: "copied kit did not index".into(),
                })
        });
        match found {
            Ok(entry) => Ok(ImportOutcome::Added(entry)),
            Err(e) => {
                let _ = std::fs::remove_dir_all(dest);
                let _ = self.rescan();
                Err(e)
            }
        }
    }

    /// The entry of the kit whose top directory is `dir`, and `dir`
    /// resolved — refused unless `dir` is the top directory of a kit the
    /// index knows, a real directory (not a symlink) directly inside the
    /// root after resolving symlinks and `..`.
    fn kit_dir_in_root(&self, root: &Path, dir: &Path) -> Result<(Entry, PathBuf), LibraryError> {
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
        let canon_root = std::fs::canonicalize(root).map_err(io_err("resolve", root))?;
        let meta = std::fs::symlink_metadata(dir).map_err(io_err("stat", dir))?;
        if !meta.is_dir() {
            return Err(outside());
        }
        let canon = std::fs::canonicalize(dir).map_err(io_err("resolve", dir))?;
        if canon.parent() != Some(canon_root.as_path()) || canon.file_name().is_some_and(is_hidden)
        {
            return Err(outside());
        }
        Ok((entry, canon))
    }

    /// Delete the kit whose top directory is `dir` (`remove_dir_all`),
    /// then rescan, which frees its slot. Marks are not touched: they are
    /// kept for the orphan window. Returns the removed entry. Blocks for
    /// as long as the delete takes (a large kit: run it on a job).
    ///
    /// Refused — before anything is removed — unless `dir` is the top
    /// directory of a kit the index knows, a real directory directly
    /// inside the root after resolving symlinks and `..`.
    pub fn delete(&mut self, dir: &Path) -> Result<Entry, LibraryError> {
        let root = self.index.root.clone().ok_or(LibraryError::NoRoot)?;
        let (entry, canon) = self.kit_dir_in_root(&root, dir)?;
        std::fs::remove_dir_all(&canon).map_err(io_err("delete", dir))?;
        self.missing.remove(&entry.id);
        self.rescan()?;
        Ok(entry)
    }

    /// Whether a scan would find exactly what the index records: the same
    /// kits with the same manifest size, mtime and sidecar, and every
    /// distinct id slotted (while slots remain).
    fn scan_is_current(&self, root: &Path) -> bool {
        let doc = &self.index.doc;
        let kits = scan_kits(root);
        if kits.len() != doc.kits.len() {
            return false;
        }
        let by_dir: HashMap<&Path, &KitRecord> =
            doc.kits.iter().map(|r| (r.dir.as_path(), r)).collect();
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
        doc.table.covers(doc.kits.iter().map(|r| r.id.as_str()))
    }

    fn rebuild_entries(&mut self) {
        let root = self.index.root.clone().unwrap_or_default();
        let doc = &self.index.doc;
        let mut records: Vec<&KitRecord> = doc.kits.iter().collect();
        records.sort_by(|a, b| a.dir.to_string_lossy().cmp(&b.dir.to_string_lossy()));
        let placed =
            slots_and_duplicates(records.iter().map(|r| (r.id.as_str(), r.dir.as_path())), &doc.table);
        let entries = records
            .iter()
            .zip(placed)
            .map(|(r, (slot, duplicate_of))| {
                make_entry(&root, r, slot, duplicate_of, self.missing.get(&r.id).copied())
            })
            .collect();
        self.entries = Entries::new(entries);
    }
}

fn space_needed(bytes: u64) -> u64 {
    (bytes as f64 * IMPORT_SPACE_FACTOR).ceil() as u64
}

/// A safe directory name for a kit: path separators and control
/// characters replaced, then surrounding whitespace and leading dots
/// stripped until nothing changes (`". ."` → `""`); `kit` when nothing is
/// left. Never hidden, so the scan always sees it, and neither is any
/// `"{name} {n}"` built from it.
fn kit_dir_name(raw: Option<String>) -> String {
    let raw = raw.unwrap_or_default();
    let mut name: String = raw
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    loop {
        let next = name.trim().trim_start_matches('.');
        if next.len() == name.len() {
            break;
        }
        name = next.to_string();
    }
    if name.is_empty() || name.starts_with('.') {
        "kit".into()
    } else {
        name
    }
}

/// The prefix of a kit directory renamed aside by a re-download
/// (`.staging/.old-<name>.<pid>-<n>`). Staged imports never start with a
/// dot ([`kit_dir_name`]), so the two cannot be confused.
const ASIDE_PREFIX: &str = ".old-";

/// `.staging/<name>.<pid>-<n>`: unique per process and import, so two
/// imports of one name never share a staging directory.
fn staging_path(root: &Path, name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    root.join(STAGING_DIR)
        .join(format!("{name}.{}-{n}", std::process::id()))
}

/// A [`staging_path`] that is empty: whatever a previous process with
/// this pid left there (pid reuse) is removed first, so it can never be
/// promoted along with this import.
fn fresh_stage(root: &Path, name: &str) -> Result<PathBuf, LibraryError> {
    let stage = staging_path(root, name);
    if std::fs::symlink_metadata(&stage).is_ok() {
        std::fs::remove_dir_all(&stage)
            .or_else(|_| std::fs::remove_file(&stage))
            .map_err(io_err("clear", &stage))?;
    }
    Ok(stage)
}

/// Rename `from` to `to`, failing if `to` exists. On Linux this is
/// `renameat2(RENAME_NOREPLACE)`, so the check and the rename are one
/// step; elsewhere (and on filesystems without it) a check, then
/// `rename` — which replaces an *empty* directory at `to` and fails on a
/// non-empty one.
fn rename_noreplace(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt;
        let f = std::ffi::CString::new(from.as_os_str().as_bytes())?;
        let t = std::ffi::CString::new(to.as_os_str().as_bytes())?;
        // SAFETY: both are NUL-terminated paths; AT_FDCWD resolves them
        // against the working directory, as `rename` does.
        let rc = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                f.as_ptr(),
                libc::AT_FDCWD,
                t.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if rc == 0 {
            return Ok(());
        }
        let e = std::io::Error::last_os_error();
        if !matches!(e.raw_os_error(), Some(libc::EINVAL) | Some(libc::ENOSYS)) {
            return Err(e);
        }
    }
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(std::io::ErrorKind::AlreadyExists.into());
    }
    std::fs::rename(from, to)
}

/// Rename the staged kit into the root as `name`, or `name 2`, `name 3`, …
/// when taken. The rename never replaces an existing entry
/// ([`rename_noreplace`]), so a racing writer never loses its kit.
fn promote(root: &Path, stage: &Path, name: &str) -> Result<PathBuf, LibraryError> {
    for n in 1..=100 {
        let dest = if n == 1 {
            root.join(name)
        } else {
            root.join(format!("{name} {n}"))
        };
        match rename_noreplace(stage, &dest) {
            Ok(()) => return Ok(dest),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists || dest.exists() => {}
            Err(e) => return Err(io_err("rename", &dest)(e)),
        }
    }
    Err(io_err("rename", &root.join(name))(
        std::io::ErrorKind::AlreadyExists.into(),
    ))
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

/// Run `f` under the exclusive `library.lock` file lock.
fn with_lock<R>(
    root: &Path,
    f: impl FnOnce() -> Result<R, LibraryError>,
) -> Result<R, LibraryError> {
    content_index::with_lock(root, |op, path, e| io_err(op, path)(e), f)
}

/// Whether process `pid` is running (unix: `kill(pid, 0)`; `EPERM` means
/// it exists under another user).
#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    // SAFETY: signal 0 only checks that the process exists.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Unknown on this platform: assume alive, so only the age rule sweeps.
#[cfg(not(unix))]
fn pid_alive(_pid: u32) -> bool {
    true
}

/// Remove what dead imports left in `.staging/`: every
/// `<name>.<pid>-<n>` whose process is gone or that is older than
/// [`STAGING_MAX_AGE`]. A re-download's aside copy
/// (`.old-<name>.<pid>-<n>`) whose replacement never landed — no
/// `<root>/<name>` — is renamed back instead of removed. Entries not in
/// that shape are left alone. Run under the lock.
fn sweep_staging(root: &Path) {
    let staging = root.join(STAGING_DIR);
    let Ok(rd) = std::fs::read_dir(&staging) else {
        return;
    };
    let own = std::process::id();
    for e in rd.flatten() {
        let file_name = e.file_name();
        let name = file_name.to_string_lossy();
        let Some((base, tag)) = name.rsplit_once('.') else {
            continue;
        };
        let Some(pid) = tag
            .split_once('-')
            .and_then(|(pid, n)| n.parse::<u64>().ok().and(pid.parse::<u32>().ok()))
        else {
            continue;
        };
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > STAGING_MAX_AGE);
        let alive = pid == own || pid_alive(pid);
        if alive && !old {
            continue;
        }
        let path = e.path();
        if let Some(kit) = base.strip_prefix(ASIDE_PREFIX) {
            let home = root.join(kit);
            if !kit.is_empty() && std::fs::symlink_metadata(&home).is_err() {
                match std::fs::rename(&path, &home) {
                    Ok(()) => {
                        tracing::warn!(
                            "restored {} from an interrupted re-download",
                            home.display()
                        );
                        continue;
                    }
                    Err(err) => tracing::warn!("restore {}: {err}", home.display()),
                }
            }
        }
        let removed = std::fs::remove_dir_all(&path).or_else(|_| std::fs::remove_file(&path));
        match removed {
            Ok(()) => tracing::info!("swept stale staging {}", path.display()),
            Err(err) => tracing::warn!("sweep {}: {err}", path.display()),
        }
    }
    let _ = std::fs::remove_dir(&staging);
}

/// A kit zip opened for install: the archive, its plan, and the id of the
/// manifest inside it (read without extracting anything).
#[cfg(feature = "drumkit-zip")]
struct OpenedZip {
    path: PathBuf,
    archive: zip::ZipArchive<File>,
    plan: install::ZipPlan,
    id: String,
}

#[cfg(feature = "drumkit-zip")]
impl OpenedZip {
    fn open(zip: &Path) -> Result<Self, LibraryError> {
        let file = File::open(zip).map_err(io_err("open", zip))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| LibraryError::Zip(format!("{}: {e}", zip.display())))?;
        let plan = install::zip_plan(&mut archive, zip)?;
        let bytes = install::read_entry(&mut archive, plan.manifest, plan.manifest_size)?;
        summarize(&bytes).map_err(|e| LibraryError::NotAKit {
            path: zip.to_path_buf(),
            reason: e.0,
        })?;
        Ok(Self {
            path: zip.to_path_buf(),
            id: hash_bytes(&bytes),
            archive,
            plan,
        })
    }

    /// Extract into `stage` and write `sidecar` there (its size measured,
    /// `downloaded_at` defaulted to now).
    fn stage(
        &mut self,
        stage: &Path,
        mut sidecar: Sidecar,
        job: &mut ImportJob<'_>,
    ) -> Result<(), LibraryError> {
        let written = install::extract(&mut self.archive, &self.plan, stage, job)?;
        if find_manifest(stage).is_none() {
            return Err(LibraryError::NotAKit {
                path: self.path.clone(),
                reason: "no drum_samples.json after extraction".into(),
            });
        }
        if sidecar.downloaded_at.is_none() {
            sidecar.downloaded_at =
                crate::library_marks::format_timestamp(crate::library_marks::now_unix());
        }
        sidecar.size_bytes = Some(written);
        write_sidecar(stage, &sidecar)?;
        Ok(())
    }
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
                let id = hash_bytes(&bytes);
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
    let canonical = canonical_ids(records.iter().map(|r| r.id.as_str()));
    let changes = doc.table.reconcile(&canonical);
    report.added = changes.added;
    report.removed = changes.removed;

    doc.kits = records;
    report.changed = files_changed || !report.added.is_empty() || !report.removed.is_empty();
    Ok(report)
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
    // The migration only ever reads: a corrupt file is left as it is.
    let Ok(bytes) = std::fs::read(&m.installed_json) else {
        return 0;
    };
    let items = match installed_json_drumkits(&bytes) {
        Ok(items) => items,
        Err(e) => {
            tracing::warn!(
                "installed.json migration: {} unreadable ({e}); skipped",
                m.installed_json.display()
            );
            return 0;
        }
    };
    let kits = scan_kits(root);
    let mut written = 0;
    for item in &items {
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
        let hit = m.in_index.as_ref().and_then(|f| f(&item.name));
        let downloaded_at = date_to_rfc3339(&item.installed_at);
        let sc = match hit {
            Some(h) => Sidecar {
                source: SOURCE_PLOK.into(),
                index_name: Some(
                    h.index_name
                        .filter(|n| !n.trim().is_empty())
                        .unwrap_or_else(|| item.name.clone()),
                ),
                index_file: h.index_file,
                sha256: h.sha256,
                description: h.description,
                index_tags: h.index_tags,
                downloaded_at,
                size_bytes: None,
            },
            None => Sidecar {
                source: SOURCE_LOCAL.into(),
                downloaded_at,
                ..Sidecar::default()
            },
        };
        match write_sidecar(dir, &sc) {
            Ok(()) => written += 1,
            Err(e) => tracing::warn!("installed.json migration: {}: {e}", dir.display()),
        }
    }
    written
}

/// Read `library.json`, tolerant by record (`content_index`'s reader): a
/// bad kit record is dropped and re-hashed; only a document that is not
/// JSON at all reads as empty, and it is quarantined only under the lock.
fn read_index(path: &Path, quarantine: bool) -> IndexDoc {
    const WHAT: &str = "drum kit library";
    let Some(value) = content_index::read_index_value(path, quarantine, WHAT) else {
        return IndexDoc::default();
    };
    IndexDoc {
        table: SlotTable::from_value(&value),
        installed_json_migrated: content_index::lenient(&value, "installed_json_migrated")
            .unwrap_or(false),
        kits: content_index::lenient_records(&value, "kits", WHAT),
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
