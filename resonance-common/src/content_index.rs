//! The content-index core shared by the per-user libraries
//! ([`crate::nam_library`], [`crate::drumkit_library`]): a cached
//! `library.json` of content-id'd records plus the stable **slot table** a
//! plugin's selector parameter indexes, its file lock, its tolerant
//! reader, change detection by stat stamp, and free-text lookup (code
//! review ARCH2-04: the two libraries were near-verbatim copies, and had
//! already begun to diverge).
//!
//! Each library keeps its own record type (what it caches about one
//! model / kit), its own entries (what callers see), its own scan (what a
//! "file" is) and its own error type; this module owns everything whose
//! semantics must not differ between them:
//!
//! * **Slots** (`SlotTable`): `slots[n] = id`. A new id takes the slot
//!   after the high-water mark; a freed slot is not reused while any slot
//!   above the mark is free; an id that comes back gets its old slot while
//!   it is still free.
//! * **Canonical entries**: the first record of an id in the library's
//!   sort order holds the slot; later ones are duplicates.
//! * **Lookup** (`Entries::find`): exact folded name, then a ≥6-hex-digit id
//!   prefix, then a unique name prefix, then a unique substring — `None`
//!   when ambiguous.
//! * **Index I/O**: the stamp that makes `reload_if_changed` one `stat`,
//!   the lenient per-record reader that quarantines only under the lock,
//!   and the exclusive `library.lock` every write runs under.
//!
//! Nothing here may run on an audio thread.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::atomic_file::{atomic_write, quarantine_corrupt, AtomicWriteError};

/// The index file under a library root.
pub const LIBRARY_FILE: &str = "library.json";
/// The lock file every index write runs under.
pub const LOCK_FILE: &str = "library.lock";
/// How many slots a selector can address (`0..=MAX_SLOT`).
pub const SLOT_COUNT: u32 = 1000;
pub const MAX_SLOT: u32 = SLOT_COUNT - 1;
/// The `library.json` format version.
pub const INDEX_VERSION: u32 = 1;

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

/// What an import did.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportOutcome<E> {
    /// Copied into the library.
    Added(E),
    /// An item with the same content id is already in the library (not
    /// copied).
    AlreadyPresent(E),
}

impl<E> ImportOutcome<E> {
    pub fn entry(&self) -> &E {
        match self {
            ImportOutcome::Added(e) | ImportOutcome::AlreadyPresent(e) => e,
        }
    }
}

/// An index file's (size, mtime): what `reload_if_changed` compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stamp {
    len: u64,
    mtime: Option<std::time::SystemTime>,
}

pub(crate) fn stat_stamp(path: &Path) -> Option<Stamp> {
    let m = std::fs::metadata(path).ok()?;
    Some(Stamp {
        len: m.len(),
        mtime: m.modified().ok(),
    })
}

/// A file's mtime in nanoseconds since the epoch: the resolution the
/// indexes compare at.
pub fn mtime_ns(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// The slot half of `library.json` (flattened into each library's
/// document, so the file format is unchanged).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SlotTable {
    pub(crate) version: u32,
    /// The index's write counter.
    pub(crate) generation: u64,
    /// The slot after the highest ever handed out.
    pub(crate) next_slot: u32,
    /// slot → id.
    pub(crate) slots: BTreeMap<u32, String>,
    /// id → the slot it held before it disappeared, so an id that comes
    /// back gets it again while it is still free.
    #[serde(default)]
    pub(crate) retired: BTreeMap<String, u32>,
}

impl Default for SlotTable {
    fn default() -> Self {
        Self {
            version: INDEX_VERSION,
            generation: 0,
            next_slot: 0,
            slots: BTreeMap::new(),
            retired: BTreeMap::new(),
        }
    }
}

/// What [`SlotTable::reconcile`] changed.
#[derive(Debug, Default)]
pub(crate) struct SlotChanges {
    /// Ids that gained a slot.
    pub(crate) added: Vec<String>,
    /// Ids whose slot was freed.
    pub(crate) removed: Vec<String>,
}

impl SlotTable {
    /// The table fields of a `library.json` value, each read on its own
    /// so one bad field never costs the others.
    pub(crate) fn from_value(value: &serde_json::Value) -> Self {
        Self {
            version: lenient(value, "version").unwrap_or(INDEX_VERSION),
            generation: lenient(value, "generation").unwrap_or(0),
            next_slot: lenient(value, "next_slot").unwrap_or(0),
            slots: lenient(value, "slots").unwrap_or_default(),
            retired: lenient(value, "retired").unwrap_or_default(),
        }
    }

    /// Next slot under the no-reuse rule: past the high-water mark while
    /// there is room, else the lowest free one; `None` when all are taken.
    pub(crate) fn allocate(&self) -> Option<u32> {
        if self.next_slot <= MAX_SLOT && !self.slots.contains_key(&self.next_slot) {
            return Some(self.next_slot);
        }
        (0..SLOT_COUNT).find(|s| !self.slots.contains_key(s))
    }

    /// Bring the table in line with the `canonical` ids now present (in
    /// the library's order): free the slots of ids that are gone (keeping
    /// them as retired), slot the new ones (an id's retired slot first,
    /// while free), and forget retired slots another id now holds.
    pub(crate) fn reconcile(&mut self, canonical: &[&str]) -> SlotChanges {
        let mut changes = SlotChanges::default();
        let live: HashSet<&str> = canonical.iter().copied().collect();
        let gone: Vec<(u32, String)> = self
            .slots
            .iter()
            .filter(|(_, id)| !live.contains(id.as_str()))
            .map(|(s, id)| (*s, id.clone()))
            .collect();
        for (slot, id) in gone {
            self.slots.remove(&slot);
            self.retired.insert(id.clone(), slot);
            changes.removed.push(id);
        }
        let slotted: HashSet<String> = self.slots.values().cloned().collect();
        for &id in canonical {
            if slotted.contains(id) {
                continue;
            }
            let reclaim = self
                .retired
                .get(id)
                .copied()
                .filter(|s| !self.slots.contains_key(s));
            if let Some(slot) = reclaim.or_else(|| self.allocate()) {
                self.slots.insert(slot, id.to_string());
                self.next_slot = self.next_slot.max(slot + 1);
                self.retired.remove(id);
                changes.added.push(id.to_string());
            }
        }
        let held: HashSet<u32> = self.slots.keys().copied().collect();
        self.retired.retain(|_, s| !held.contains(s));
        changes
    }

    /// Whether every one of `ids` holds a slot, or the table is full (so
    /// a rescan could not slot more).
    pub(crate) fn covers<'a>(&self, mut ids: impl Iterator<Item = &'a str>) -> bool {
        if self.slots.len() >= SLOT_COUNT as usize {
            return true;
        }
        let slotted: HashSet<&str> = self.slots.values().map(String::as_str).collect();
        ids.all(|id| slotted.contains(id))
    }

    /// Mark a write: a new generation at the current format version.
    pub(crate) fn bump(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.version = INDEX_VERSION;
    }

    /// id → slot.
    pub(crate) fn slot_of(&self) -> HashMap<&str, u32> {
        self.slots.iter().map(|(s, id)| (id.as_str(), *s)).collect()
    }
}

/// The distinct ids of `ids`, in first-seen order: the canonical record
/// of each id is its first.
pub(crate) fn canonical_ids<'a>(ids: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut seen = HashSet::new();
    ids.filter(|id| seen.insert(*id)).collect()
}

/// For records already in the library's order (`(id, key path)` each):
/// the slot each holds, and for a duplicate the key path of the record
/// that holds its id instead.
pub(crate) fn slots_and_duplicates<'a>(
    records: impl Iterator<Item = (&'a str, &'a Path)>,
    table: &SlotTable,
) -> Vec<(Option<u32>, Option<PathBuf>)> {
    let slot_of = table.slot_of();
    let mut first: HashMap<&str, &Path> = HashMap::new();
    records
        .map(|(id, key)| match first.get(id) {
            Some(p) => (None, Some(p.to_path_buf())),
            None => {
                first.insert(id, key);
                (slot_of.get(id).copied(), None)
            }
        })
        .collect()
}

/// One field of a JSON object, if present and of the right shape.
pub(crate) fn lenient<T: serde::de::DeserializeOwned>(v: &serde_json::Value, k: &str) -> Option<T> {
    serde_json::from_value(v.get(k)?.clone()).ok()
}

/// Read `library.json` as JSON. A missing file is `None`; a file that is
/// not JSON at all is `None` too, and is quarantined only when
/// `quarantine` is set — which a rescan does under the lock, never a
/// reader, so a reader can never rename away a file a writer has just
/// installed. `what` names the library in the log.
pub(crate) fn read_index_value(
    path: &Path,
    quarantine: bool,
    what: &str,
) -> Option<serde_json::Value> {
    let bytes = std::fs::read(path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(v) => Some(v),
        Err(e) => {
            if quarantine {
                tracing::error!("{what} index {} unreadable ({e}); rebuilding", path.display());
                quarantine_corrupt(path);
            }
            None
        }
    }
}

/// The records under `key`, tolerant by record: one that does not
/// deserialise is dropped (the next scan re-hashes it) and never costs
/// the slot table.
pub(crate) fn lenient_records<R: serde::de::DeserializeOwned>(
    value: &serde_json::Value,
    key: &str,
    what: &str,
) -> Vec<R> {
    match value.get(key) {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|r| match serde_json::from_value::<R>(r.clone()) {
                Ok(rec) => Some(rec),
                Err(e) => {
                    tracing::warn!("{what} index: dropping a bad record ({e})");
                    None
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Write `doc` as `library.json` atomically.
pub(crate) fn write_index<E>(path: &Path, doc: &impl Serialize) -> Result<(), E>
where
    E: From<serde_json::Error> + From<AtomicWriteError>,
{
    let bytes = serde_json::to_vec_pretty(doc)?;
    atomic_write(path, &bytes)?;
    Ok(())
}

/// Run `f` under the exclusive `library.lock` in `root` (best effort where
/// the platform has no file locks). `io_err` maps an open/lock failure to
/// the library's error.
pub(crate) fn with_lock<R, E>(
    root: &Path,
    io_err: impl Fn(&'static str, &Path, std::io::Error) -> E,
    f: impl FnOnce() -> Result<R, E>,
) -> Result<R, E> {
    let lock_path = root.join(LOCK_FILE);
    let lock = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|e| io_err("open", &lock_path, e))?;
    let locked = crate::library_marks::lock_or_best_effort(&lock, &lock_path)
        .map_err(|e| io_err("lock", &lock_path, e))?;
    let result = f();
    if locked {
        let _ = lock.unlock();
    }
    result
}

/// What lookup needs to know about a library's entry.
pub(crate) trait IndexedEntry {
    fn id(&self) -> &str;
    fn slot(&self) -> Option<u32>;
    fn name(&self) -> &str;
    fn is_duplicate(&self) -> bool;
    /// The path the library keys entries by (a model's file, a kit's
    /// directory).
    fn key_path(&self) -> &Path;
}

/// Entries in their published order — slotted ones first in slot order,
/// then the rest (duplicates, the overflow past [`SLOT_COUNT`]) by key
/// path — and the three maps over them.
#[derive(Debug, Clone)]
pub(crate) struct Entries<E> {
    pub(crate) list: Vec<E>,
    by_id: HashMap<String, usize>,
    by_slot: HashMap<u32, usize>,
    by_key: HashMap<PathBuf, usize>,
}

impl<E> Default for Entries<E> {
    fn default() -> Self {
        Self {
            list: Vec::new(),
            by_id: HashMap::new(),
            by_slot: HashMap::new(),
            by_key: HashMap::new(),
        }
    }
}

impl<E: IndexedEntry> Entries<E> {
    pub(crate) fn new(mut list: Vec<E>) -> Self {
        list.sort_by(|a, b| match (a.slot(), b.slot()) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.key_path().cmp(b.key_path()),
        });
        let mut entries = Self {
            list: Vec::new(),
            by_id: HashMap::new(),
            by_slot: HashMap::new(),
            by_key: HashMap::new(),
        };
        for (i, e) in list.iter().enumerate() {
            if !e.is_duplicate() {
                entries.by_id.insert(e.id().to_string(), i);
            }
            if let Some(s) = e.slot() {
                entries.by_slot.insert(s, i);
            }
            entries.by_key.insert(e.key_path().to_path_buf(), i);
        }
        entries.list = list;
        entries
    }

    /// The canonical entry of an id.
    pub(crate) fn by_id(&self, id: &str) -> Option<&E> {
        self.by_id.get(id).map(|&i| &self.list[i])
    }

    pub(crate) fn by_slot(&self, slot: u32) -> Option<&E> {
        self.by_slot.get(&slot).map(|&i| &self.list[i])
    }

    pub(crate) fn by_key(&self, key: &Path) -> Option<&E> {
        self.by_key.get(key).map(|&i| &self.list[i])
    }

    /// Resolve free text to a slotted entry: an exact name (folded: case
    /// and Latin diacritics, as every browser searches), then an id prefix
    /// of at least 6 hex digits, then a unique name prefix, then a unique
    /// name substring. `None` when nothing or several match — two entries
    /// can share a display name, and an ambiguous exact name must make the
    /// caller error rather than silently take the first.
    pub(crate) fn find(&self, text: &str) -> Option<&E> {
        let t = text.trim();
        if t.is_empty() {
            return None;
        }
        use crate::library_marks::vocab::fold;
        let lower = fold(t);
        let slotted = || self.list.iter().filter(|e| e.slot().is_some());
        let mut exact = slotted().filter(|e| fold(e.name()) == lower);
        match (exact.next(), exact.next()) {
            (Some(e), None) => return Some(e),
            (Some(_), Some(_)) => return None,
            _ => {}
        }
        if t.len() >= 6 && t.chars().all(|c| c.is_ascii_hexdigit()) {
            let hex = t.to_ascii_lowercase();
            let mut hits = slotted().filter(|e| e.id().starts_with(&hex));
            if let (Some(e), None) = (hits.next(), hits.next()) {
                return Some(e);
            }
        }
        let unique = |pred: &dyn Fn(&E) -> bool| {
            let mut hits = slotted().filter(|e| pred(e));
            match (hits.next(), hits.next()) {
                (Some(e), None) => Some(e),
                _ => None,
            }
        };
        unique(&|e: &E| fold(e.name()).starts_with(&lower))
            .or_else(|| unique(&|e: &E| fold(e.name()).contains(&lower)))
    }
}

/// The cached index of one library: its root, the stamp of the
/// `library.json` it was read from, and the parsed document.
#[derive(Debug, Clone)]
pub(crate) struct IndexFile<D> {
    pub(crate) root: Option<PathBuf>,
    pub(crate) stamp: Option<Stamp>,
    pub(crate) doc: D,
}

impl<D> IndexFile<D> {
    pub(crate) fn path(&self) -> Option<PathBuf> {
        self.root.as_ref().map(|r| r.join(LIBRARY_FILE))
    }

    /// Re-read `library.json` with `read` if its stamp (size, mtime)
    /// moved. Returns whether it was re-read. One `stat` when nothing
    /// moved.
    ///
    /// The generation is not a reliable "unchanged" signal: an index that
    /// was deleted and rebuilt, or rewritten by another tool, can carry
    /// the generation a reader already has with different contents.
    pub(crate) fn reload_if_changed(&mut self, read: impl FnOnce(&Path) -> D) -> bool {
        let Some(path) = self.path() else {
            return false;
        };
        let now = stat_stamp(&path);
        if now == self.stamp {
            return false;
        }
        self.doc = read(&path);
        self.stamp = now;
        true
    }
}
