//! [`PresetLibrary`]: the in-memory index over factory and user presets
//! (plugin-preset-library.md §4.6), and the only thing that writes to the
//! user preset directory.
//!
//! - **Factory** records are registered by the caller: a plugin from its
//!   `FACTORY_PRESETS`, the host (round 2) from the scan.
//! - **User** records come from `<root>/<clap id>/*.json`. The first time a
//!   plugin's directory is indexed in a process, legacy files are
//!   converted ([`super::migrate`]) and trash older than
//!   [`TRASH_RETENTION`] is purged. After that the directory is re-read
//!   only when its fingerprint (directory mtime, entry count, newest entry
//!   mtime) changed, and the fingerprint itself is checked at most once
//!   per `max_age` the caller passes, so a bar drawn every frame costs
//!   nothing between checks.
//! - **Writes** (save, rename, trash) go through an atomic replace and
//!   update the index in place, so the writing process sees them at once
//!   and every other one on its next fingerprint check (§11).
//! - **Marks** are read through [`MarksSource`] at query time and never
//!   cached, so they cannot go stale in the index.
//!
//! One library per preset root per process ([`PresetLibrary::shared`] /
//! [`PresetLibrary::shared_for_root`]): every editor instance of a plugin
//! `.so`, and every `PresetBank` the host builds per call, share it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::{Mutex, RwLock};

use super::format::{self, PresetFile, PresetMeta, PresetPluginInfo};
use super::marks::{mark_key, MarksSource, NoMarks};
use super::query::{self, Query, QueryResult};
use super::rows::PresetRows;
use resonance_common::library_marks::{Marks, SharedMarks};
use super::{files, migrate, user_preset_root, FactoryPreset, PresetRef, PresetSource};
use super::PRESET_STATE_KEY;

/// How long a deleted user preset stays recoverable in `.trash/` (D10).
pub const TRASH_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Name of the trash directory under the preset root.
pub const TRASH_DIR: &str = ".trash";

/// The clock the library stamps and purges with. Injected in tests.
pub type Clock = Arc<dyn Fn() -> SystemTime + Send + Sync>;

/// One indexed preset.
#[derive(Debug, Clone, PartialEq)]
pub struct PresetRecord {
    pub preset: PresetRef,
    pub meta: PresetMeta,
    /// `plugin.version` of the file; `None` for factory presets and for
    /// files that did not record one.
    pub plugin_version: Option<String>,
    /// The file behind a user preset. `None` for factory presets.
    pub path: Option<PathBuf>,
    /// Position in bank order: factory presets in declared order, then
    /// user presets by name.
    pub bank_order: usize,
}

/// A factory preset in owned form, for callers whose bank is not a
/// compile-time constant (the host's copy arrives from the scan).
#[derive(Debug, Clone, PartialEq)]
pub struct FactoryEntry {
    pub id: String,
    pub name: String,
    /// The preset file (format 1) or a bare state document.
    pub json: String,
}

impl FactoryEntry {
    /// An entry from the parts the host reads out of a plugin's factory
    /// symbol: the bare state document and the `meta` object, both as JSON
    /// text. Builds the format-1 file so the index reads its metadata.
    pub fn from_parts(id: &str, name: &str, state_json: &str, meta_json: Option<&str>) -> Self {
        let doc: serde_json::Value = serde_json::from_str(state_json).unwrap_or_default();
        let mut meta: PresetMeta = meta_json
            .and_then(|m| serde_json::from_str(m).ok())
            .unwrap_or_default();
        meta.name = name.to_string();
        let file = PresetFile::new(id, PresetPluginInfo::default(), meta, doc);
        Self {
            id: id.to_string(),
            name: name.to_string(),
            json: file.to_text().unwrap_or_else(|_| state_json.to_string()),
        }
    }
}

/// What [`PresetLibrary::save`] writes.
#[derive(Debug, Clone, Default)]
pub struct SaveRequest {
    pub name: String,
    /// The plugin's state document. Its `"preset"` session key is
    /// stripped: a preset must not claim to be a modified version of
    /// itself.
    pub doc: serde_json::Value,
    /// Descriptive metadata. For a new preset this seeds the meta block;
    /// when the save overwrites an existing preset of the same name,
    /// `Some` replaces its descriptive fields and `None` keeps them.
    pub meta: Option<PresetMeta>,
    /// Lineage for a *new* preset ("Save as…" from a loaded one). Ignored
    /// on overwrite, which keeps the file's own lineage.
    pub derived_from: Option<String>,
    /// Plugin name and version to record (`id` is filled in).
    pub plugin: PresetPluginInfo,
    /// The id a *new* preset gets (a UUID minted up front by a caller that
    /// must report it before the file lands); a fresh one when `None` or
    /// not UUID-shaped.
    pub id: Option<String>,
    /// Update this user preset (by id) in place instead of the one `name`
    /// addresses. It keeps its id and takes `name`; refused if it does not
    /// exist or another user preset already has `name`.
    pub target: Option<String>,
    /// A third-party plugin's opaque state instead of `doc` (§8 tier T0):
    /// stored as `state.encoding = "clap-state"`; `doc` is then ignored.
    pub blob: Option<Vec<u8>>,
    /// For a blob: whether it is the plugin's preset form (`Some(true)`),
    /// its full state (`Some(false)`), or unknown (`None`).
    pub blob_preset_form: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Fingerprint {
    Missing,
    Present {
        dir_mtime: Option<SystemTime>,
        entries: usize,
        newest: Option<SystemTime>,
    },
}

#[derive(Default)]
struct FactorySet {
    /// `(ptr, len)` of the static slice last registered, so re-registering
    /// the same bank (every `PresetBank::new`) costs a comparison.
    key: Option<(usize, usize)>,
    records: Vec<PresetRecord>,
    docs: Vec<String>,
}

struct UserIndex {
    dir: PathBuf,
    opened: bool,
    records: Vec<PresetRecord>,
    fingerprint: Option<Fingerprint>,
    checked_at: Option<Instant>,
}

#[derive(Default)]
struct PluginIndex {
    factory: FactorySet,
    user: Option<UserIndex>,
    merged: Option<Arc<Vec<PresetRecord>>>,
}

/// The preset index for one preset root. See the module docs.
pub struct PresetLibrary {
    root: Option<PathBuf>,
    /// `None` until installed or first needed: the process-wide default
    /// library opens the user's marks store lazily ([`Self::marks`]), so
    /// nothing reads it unless a query or a mark asks.
    marks: RwLock<Option<Arc<dyn MarksSource>>>,
    default_marks: bool,
    /// When [`Self::refresh_marks`] last asked the store to re-read.
    marks_checked: Mutex<Option<Instant>>,
    clock: Clock,
    plugins: Mutex<HashMap<String, PluginIndex>>,
    /// Background housekeeping started by a first open (the trash sweep).
    housekeeping: Mutex<Vec<std::thread::JoinHandle<usize>>>,
}

impl Default for PresetLibrary {
    fn default() -> Self {
        Self::new()
    }
}

impl PresetLibrary {
    /// A library over the default root ([`user_preset_root`], resolved on
    /// every access so [`super::USER_PRESET_DIR_ENV`] is honoured).
    pub fn new() -> Self {
        Self {
            root: None,
            marks: RwLock::new(None),
            default_marks: false,
            marks_checked: Mutex::new(None),
            clock: Arc::new(SystemTime::now),
            plugins: Mutex::new(HashMap::new()),
            housekeeping: Mutex::new(Vec::new()),
        }
    }

    /// Point this library at an explicit root: the directory that
    /// *contains* the per-plugin folders.
    pub fn with_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.root = Some(root.into());
        self
    }

    /// Replace the clock (tests: trash purging, timestamps).
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// The process-wide library over the default root.
    pub fn shared() -> Arc<Self> {
        static SHARED: OnceLock<Arc<PresetLibrary>> = OnceLock::new();
        SHARED
            .get_or_init(|| {
                Arc::new(Self {
                    default_marks: true,
                    ..Self::new()
                })
            })
            .clone()
    }

    /// The process-wide library over `root`, created on first use.
    pub fn shared_for_root(root: &Path) -> Arc<Self> {
        static BY_ROOT: OnceLock<Mutex<HashMap<PathBuf, Arc<PresetLibrary>>>> = OnceLock::new();
        BY_ROOT
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .entry(root.to_path_buf())
            .or_insert_with(|| Arc::new(Self::new().with_root(root)))
            .clone()
    }

    /// Install the marks store this library reads and writes marks
    /// through (the app installs its own `SharedMarks`).
    pub fn set_marks(&self, marks: Arc<dyn MarksSource>) {
        *self.marks.write() = Some(marks);
    }

    /// The marks store. The process-wide default library opens the user's
    /// store ([`SharedMarks::open_default_or_detached`], honouring
    /// `RESONANCE_LIBRARY_DIR`) the first time it is asked; a library over
    /// an explicit root reads [`NoMarks`] until one is installed.
    pub fn marks(&self) -> Arc<dyn MarksSource> {
        if let Some(m) = self.marks.read().clone() {
            return m;
        }
        let mut slot = self.marks.write();
        slot.get_or_insert_with(|| {
            if self.default_marks {
                match super::default_marks_dir_override() {
                    Some(dir) => Arc::new(
                        SharedMarks::open(dir).unwrap_or_else(|_| SharedMarks::detached()),
                    ),
                    None => Arc::new(SharedMarks::open_default_or_detached()),
                }
            } else {
                Arc::new(NoMarks)
            }
        })
        .clone()
    }

    /// The root directory, or `None` when the platform has no data
    /// directory and no override is set.
    pub fn root(&self) -> Option<PathBuf> {
        self.root.clone().or_else(user_preset_root)
    }

    /// The directory holding `plugin_id`'s user presets.
    pub fn plugin_dir(&self, plugin_id: &str) -> Option<PathBuf> {
        self.root().map(|r| r.join(plugin_id))
    }

    /// Where trashed presets of `plugin_id` go.
    pub fn trash_dir(&self, plugin_id: &str) -> Option<PathBuf> {
        self.root().map(|r| r.join(TRASH_DIR).join(plugin_id))
    }

    fn now(&self) -> SystemTime {
        (self.clock)()
    }

    // -----------------------------------------------------------------
    // Factory registration
    // -----------------------------------------------------------------

    /// Register a plugin's compiled-in factory bank. Cheap to repeat with
    /// the same slice.
    pub fn register_factory(&self, plugin_id: &str, bank: &'static [FactoryPreset]) {
        let key = (bank.as_ptr() as usize, bank.len());
        let mut plugins = self.plugins.lock();
        let index = plugins.entry(plugin_id.to_string()).or_default();
        if index.factory.key == Some(key) {
            return;
        }
        let entries = bank.iter().map(|p| FactoryEntry {
            id: p.id.to_string(),
            name: p.name.to_string(),
            json: p.json.to_string(),
        });
        index.factory = build_factory(plugin_id, entries);
        index.factory.key = Some(key);
        index.merged = None;
    }

    /// How many factory presets are registered for `plugin_id`.
    pub fn factory_len(&self, plugin_id: &str) -> usize {
        self.plugins
            .lock()
            .get(plugin_id)
            .map(|i| i.factory.records.len())
            .unwrap_or(0)
    }

    /// Register a factory bank that is not a compile-time constant.
    /// Replaces whatever was registered for `plugin_id`.
    pub fn register_factory_entries(
        &self,
        plugin_id: &str,
        entries: impl IntoIterator<Item = FactoryEntry>,
    ) {
        let mut plugins = self.plugins.lock();
        let index = plugins.entry(plugin_id.to_string()).or_default();
        index.factory = build_factory(plugin_id, entries);
        index.merged = None;
    }

    // -----------------------------------------------------------------
    // Reading
    // -----------------------------------------------------------------

    /// Every preset of `plugin_id` in bank order, re-reading the user
    /// directory first if its fingerprint changed and was last checked
    /// at least `max_age` ago. `Duration::ZERO` checks now (explicit
    /// calls: a control-API list, a click); a bar drawn every frame
    /// passes its poll interval.
    pub fn records(&self, plugin_id: &str, max_age: Duration) -> Arc<Vec<PresetRecord>> {
        let mut plugins = self.plugins.lock();
        let index = plugins.entry(plugin_id.to_string()).or_default();
        self.ensure_fresh(plugin_id, index, max_age);
        if let Some(merged) = &index.merged {
            return merged.clone();
        }
        let mut all = index.factory.records.clone();
        if let Some(user) = &index.user {
            all.extend(user.records.iter().cloned());
        }
        for (i, r) in all.iter_mut().enumerate() {
            r.bank_order = i;
        }
        let merged = Arc::new(all);
        index.merged = Some(merged.clone());
        merged
    }

    /// [`record`](Self::record) without touching the disk: looks only at
    /// what is already in memory, the registered factory bank plus the
    /// user index **if** something (an editor's bar, a browser, an
    /// explicit list) has already opened this plugin's directory in this
    /// process. Never opens a directory, never converts, never writes.
    /// State load resolves through this, so loading a project, and every
    /// plugin state test, stays off the user's preset directory.
    pub fn peek_record(&self, plugin_id: &str, preset: &PresetRef) -> Option<PresetRecord> {
        let plugins = self.plugins.lock();
        let index = plugins.get(plugin_id)?;
        let dir = self.plugin_dir(plugin_id);
        let users: &[PresetRecord] = match &index.user {
            Some(u) if u.opened && Some(&u.dir) == dir.as_ref() => &u.records,
            _ => &[],
        };
        let from_factory = find_record(&index.factory.records, preset);
        from_factory.or_else(|| find_record(users, preset)).cloned()
    }

    /// Force a re-read of `plugin_id`'s user directory.
    pub fn refresh(&self, plugin_id: &str) {
        let mut plugins = self.plugins.lock();
        let index = plugins.entry(plugin_id.to_string()).or_default();
        if let Some(user) = &mut index.user {
            user.fingerprint = None;
            user.checked_at = None;
        }
        self.ensure_fresh(plugin_id, index, Duration::ZERO);
    }

    /// The record behind `preset`. An unresolved reference (no id: a
    /// project saved before ids existed) resolves by name, exact match
    /// first, then case-insensitively.
    pub fn record(&self, plugin_id: &str, preset: &PresetRef) -> Option<PresetRecord> {
        let all = self.records(plugin_id, Duration::ZERO);
        find_record(&all, preset).cloned()
    }

    /// The state document behind `preset` as JSON text — what the
    /// loaders take. `None` when the preset is gone or unreadable.
    pub fn state_json(&self, plugin_id: &str, preset: &PresetRef) -> Option<String> {
        let record = self.record(plugin_id, preset)?;
        match record.preset.source {
            PresetSource::Factory => {
                let plugins = self.plugins.lock();
                let index = plugins.get(plugin_id)?;
                let at = index
                    .factory
                    .records
                    .iter()
                    .position(|r| r.preset.id == record.preset.id)?;
                index.factory.docs.get(at).cloned()
            }
            PresetSource::User => {
                let path = record.path.as_ref()?;
                let file = std::fs::read_to_string(path)
                    .ok()
                    .and_then(|t| PresetFile::parse(&t).ok());
                if file.is_none() {
                    // Gone or torn under us: make the next read rescan.
                    self.invalidate(plugin_id);
                }
                file?.state_json()
            }
        }
    }

    /// The opaque state behind a blob-encoded (third-party) user preset.
    /// `None` for a document preset, or when it is gone or unreadable.
    pub fn state_blob(&self, plugin_id: &str, preset: &PresetRef) -> Option<Vec<u8>> {
        let record = self.record(plugin_id, preset)?;
        let path = record.path.as_ref()?;
        let file = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| PresetFile::parse(&t).ok())?;
        file.state.blob_bytes()
    }

    /// The rows of `plugin_ids` (every plugin this library knows when
    /// empty), with marks fresh from the store: what a browser and
    /// `presets.search` read.
    pub fn rows(&self, plugin_ids: &[String]) -> PresetRows {
        let plugin_ids: Vec<String> = if plugin_ids.is_empty() {
            let mut ids: Vec<String> = self.plugins.lock().keys().cloned().collect();
            ids.sort();
            ids
        } else {
            plugin_ids.to_vec()
        };
        // Throttled: a query per keystroke or star must not stat the marks
        // file and re-list every preset directory each time. Our own
        // writes update the index directly; another process's show within
        // the poll interval.
        self.refresh_marks(crate::library_marks::BROWSER_POLL_INTERVAL);
        let marks = self.marks();
        let sets: Vec<(String, Arc<Vec<PresetRecord>>)> = plugin_ids
            .into_iter()
            .map(|id| {
                let records = self.records(&id, crate::library_marks::BROWSER_POLL_INTERVAL);
                (id, records)
            })
            .collect();
        PresetRows::build(
            sets.iter().map(|(id, r)| (id.as_str(), r.as_slice())),
            marks.as_ref(),
        )
    }

    /// Search. `q.plugins` empty searches every plugin this library has
    /// indexed or had a factory bank registered for.
    pub fn query(&self, q: &Query) -> QueryResult {
        query::run(&self.rows(&q.plugins), q)
    }

    // -----------------------------------------------------------------
    // Marks (favourite, personal tags, recents)
    // -----------------------------------------------------------------

    /// Pick up another process's marks write, checking at most once per
    /// `max_age` (a widget drawn every frame passes its poll interval).
    pub fn refresh_marks(&self, max_age: Duration) {
        {
            let mut at = self.marks_checked.lock();
            if at.is_some_and(|t| t.elapsed() < max_age) {
                return;
            }
            *at = Some(Instant::now());
        }
        self.marks().refresh();
    }

    /// The marks of one preset.
    pub fn preset_marks(&self, plugin_id: &str, preset_id: &str) -> Marks {
        self.marks().marks(&mark_key(plugin_id, preset_id))
    }

    /// Star or unstar a preset (factory presets too: marks never touch the
    /// preset). Per-user state, not content.
    pub fn set_favorite(
        &self,
        plugin_id: &str,
        preset_id: &str,
        favorite: bool,
    ) -> Result<Marks, String> {
        self.marks()
            .update(&mark_key(plugin_id, preset_id), &|m| m.favorite = favorite)
    }

    /// Replace a preset's personal tags (normalised; empty clears).
    pub fn set_personal_tags(
        &self,
        plugin_id: &str,
        preset_id: &str,
        tags: &[String],
    ) -> Result<Marks, String> {
        let tags = resonance_common::library_marks::normalize_tags(tags);
        self.marks()
            .update(&mark_key(plugin_id, preset_id), &|m| m.tags = tags.clone())
    }

    /// Record a user pick (a browser/bar pick or a control-API load, never
    /// a project restore): `last_used` = now, `use_count` + 1.
    pub fn record_use(&self, plugin_id: &str, preset_id: &str) -> Result<Marks, String> {
        let now = self
            .now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        self.marks().update(&mark_key(plugin_id, preset_id), &|m| {
            m.last_used = Some(now);
            m.use_count = m.use_count.saturating_add(1);
        })
    }

    // -----------------------------------------------------------------
    // Writing
    // -----------------------------------------------------------------

    /// Save a user preset. Names are unique per plugin among user presets,
    /// case-insensitively (D11): saving under a name that exists
    /// overwrites that preset in place — same id, same lineage,
    /// `modified` bumped — and anything else creates a new preset with a
    /// fresh UUID.
    pub fn save(&self, plugin_id: &str, request: SaveRequest) -> Result<PresetRecord, String> {
        let name = validate_name(&request.name)?;
        let dir = self
            .plugin_dir(plugin_id)
            .ok_or_else(|| "No user data directory available".to_string())?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("Create preset directory: {e}"))?;

        let mut doc = request.doc;
        if let Some(obj) = doc.as_object_mut() {
            obj.remove(PRESET_STATE_KEY);
        }
        let now = format::rfc3339(self.now());

        let mut plugins = self.plugins.lock();
        let index = plugins.entry(plugin_id.to_string()).or_default();
        self.ensure_fresh(plugin_id, index, Duration::ZERO);
        let users: &[PresetRecord] =
            index.user.as_ref().map(|u| u.records.as_slice()).unwrap_or(&[]);
        let existing = match &request.target {
            Some(target) => {
                let found = users
                    .iter()
                    .find(|r| r.preset.id == *target)
                    .cloned()
                    .ok_or_else(|| format!("No user preset with id {target:?}"))?;
                if users
                    .iter()
                    .any(|r| r.preset.id != found.preset.id && same_name(&r.meta.name, &name))
                {
                    return Err(format!("A preset named '{name}' already exists"));
                }
                Some(found)
            }
            None => users.iter().find(|r| same_name(&r.meta.name, &name)).cloned(),
        };

        let mut plugin = request.plugin;
        plugin.id = plugin_id.to_string();
        let (id, meta, old_path) = match existing {
            Some(old) => {
                // Re-read the file so fields another writer added survive.
                let mut meta = old
                    .path
                    .as_ref()
                    .and_then(|p| std::fs::read_to_string(p).ok())
                    .and_then(|t| PresetFile::parse(&t).ok())
                    .map(|f| f.meta)
                    .unwrap_or(old.meta.clone());
                if let Some(new) = &request.meta {
                    meta.inherit_descriptive(new);
                }
                meta.name = name.clone();
                meta.modified = Some(now.clone());
                (old.preset.id.clone(), meta, old.path)
            }
            None => {
                let mut meta = PresetMeta::named(name.clone());
                if let Some(new) = &request.meta {
                    meta.inherit_descriptive(new);
                }
                meta.created = Some(now.clone());
                meta.modified = Some(now.clone());
                meta.derived_from = request.derived_from.clone();
                let id = request
                    .id
                    .clone()
                    .filter(|id| format::is_uuid(id) && !users.iter().any(|r| r.preset.id == *id))
                    .unwrap_or_else(format::new_uuid);
                (id, meta, None)
            }
        };
        let mut file = PresetFile::new(id, plugin, meta.normalized(), doc);
        if let Some(blob) = &request.blob {
            file.state = match request.blob_preset_form {
                Some(form) => format::PresetState::clap_blob_form(blob, form),
                None => format::PresetState::clap_blob(blob),
            };
        }
        let path = dir.join(files::preset_file_name(&name, &file.id));
        if let Some(old) = &old_path {
            files::move_before_rewrite(old, &path)?;
        }
        files::atomic_write(&path, file.to_text()?.as_bytes())?;
        let record = user_record(&file, path);
        upsert_user(index, record.clone());
        Ok(record)
    }

    /// Rename a user preset. The id does not change, so favourites, tags
    /// and every loaded identity follow it. Refused when another user
    /// preset already has the name (D11).
    pub fn rename(
        &self,
        plugin_id: &str,
        preset: &PresetRef,
        new_name: &str,
    ) -> Result<PresetRecord, String> {
        if preset.source != PresetSource::User {
            return Err("Factory presets cannot be renamed".to_string());
        }
        let new_name = validate_name(new_name)?;
        let mut plugins = self.plugins.lock();
        let index = plugins.entry(plugin_id.to_string()).or_default();
        self.ensure_fresh(plugin_id, index, Duration::ZERO);
        let users = index.user.as_ref().map(|u| u.records.as_slice()).unwrap_or(&[]);
        let record = find_record(users, preset)
            .cloned()
            .ok_or_else(|| format!("No preset named '{}'", preset.name))?;
        if record.meta.name == new_name {
            return Ok(record);
        }
        if users
            .iter()
            .any(|r| r.preset.id != record.preset.id && same_name(&r.meta.name, &new_name))
        {
            return Err(format!("A preset named '{new_name}' already exists"));
        }
        let from = record
            .path
            .clone()
            .ok_or_else(|| "User preset has no file".to_string())?;
        let text = std::fs::read_to_string(&from)
            .map_err(|e| format!("Read preset '{}': {e}", preset.name))?;
        let mut file = PresetFile::parse(&text)?;
        file.meta.name = new_name.clone();
        file.meta.modified = Some(format::rfc3339(self.now()));
        let to = from.with_file_name(files::preset_file_name(&new_name, &file.id));
        files::move_before_rewrite(&from, &to)?;
        files::atomic_write(&to, file.to_text()?.as_bytes())?;
        let record = user_record(&file, to);
        upsert_user(index, record.clone());
        Ok(record)
    }

    /// Edit a **user** preset's content metadata in place (name, id and
    /// lineage excepted; rename with [`Self::rename`]). Factory presets are
    /// refused: their tags are personal marks (`set_personal_tags`).
    pub fn update_meta(
        &self,
        plugin_id: &str,
        preset: &PresetRef,
        edit: impl FnOnce(&mut PresetMeta),
    ) -> Result<PresetRecord, String> {
        if preset.source != PresetSource::User {
            return Err(
                "Factory presets are read-only; star or tag them with presets.set_marks".into(),
            );
        }
        let mut plugins = self.plugins.lock();
        let index = plugins.entry(plugin_id.to_string()).or_default();
        self.ensure_fresh(plugin_id, index, Duration::ZERO);
        let users = index.user.as_ref().map(|u| u.records.as_slice()).unwrap_or(&[]);
        let record = find_record(users, preset)
            .cloned()
            .ok_or_else(|| format!("No preset named '{}'", preset.name))?;
        let path = record
            .path
            .clone()
            .ok_or_else(|| "User preset has no file".to_string())?;
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("Read preset '{}': {e}", preset.name))?;
        let mut file = PresetFile::parse(&text)?;
        let (name, created, derived) = (
            file.meta.name.clone(),
            file.meta.created.clone(),
            file.meta.derived_from.clone(),
        );
        edit(&mut file.meta);
        file.meta.name = name;
        file.meta.created = created;
        file.meta.derived_from = derived;
        file.meta.modified = Some(format::rfc3339(self.now()));
        file.meta = std::mem::take(&mut file.meta).normalized();
        files::atomic_write(&path, file.to_text()?.as_bytes())?;
        let record = user_record(&file, path);
        upsert_user(index, record.clone());
        Ok(record)
    }

    /// A user-preset name based on `base` that no user preset of
    /// `plugin_id` has yet: `base`, else `base 2`, `base 3`, …
    pub fn unique_name(&self, plugin_id: &str, base: &str) -> String {
        let records = self.records(plugin_id, Duration::ZERO);
        let taken = |n: &str| {
            records
                .iter()
                .any(|r| r.preset.source == PresetSource::User && same_name(&r.meta.name, n))
        };
        let base = base.trim();
        if !taken(base) {
            return base.to_string();
        }
        (2..1000)
            .map(|i| format!("{base} {i}"))
            .find(|n| !taken(n))
            .unwrap_or_else(|| format!("{base} {}", format::new_uuid()))
    }

    /// Copy any preset (factory or user) to a new user preset
    /// "`<name> copy`": its sound and its metadata, a fresh id, and
    /// `derived_from` pointing at the original (§6.4 Duplicate).
    pub fn duplicate(
        &self,
        plugin_id: &str,
        preset: &PresetRef,
        plugin: PresetPluginInfo,
    ) -> Result<PresetRecord, String> {
        let record = self
            .record(plugin_id, preset)
            .ok_or_else(|| format!("No preset named '{}'", preset.name))?;
        let blob = self.state_blob(plugin_id, preset);
        let doc: serde_json::Value = match &blob {
            Some(_) => serde_json::Value::Null,
            None => self
                .state_json(plugin_id, preset)
                .and_then(|j| serde_json::from_str(&j).ok())
                .ok_or_else(|| format!("Preset '{}' is unreadable", preset.name))?,
        };
        let name = self.unique_name(plugin_id, &format!("{} copy", record.meta.name));
        self.save(
            plugin_id,
            SaveRequest {
                name,
                doc,
                meta: Some(record.meta.clone()),
                derived_from: Some(record.preset.id.clone()),
                plugin,
                id: None,
                target: None,
                blob,
                blob_preset_form: None,
            },
        )
    }

    /// Import a preset file (format 1, or a bare state document) for
    /// `plugin_id` (§6.4 Import). A file for another plugin is refused,
    /// naming it; one whose document names none of `param_ids` is refused
    /// as not this plugin's. The file keeps its id unless another preset
    /// already has it (then it gets a fresh one, and `Ok((record, true))`
    /// says so); a name another user preset has gets a number.
    pub fn import(
        &self,
        plugin_id: &str,
        path: &Path,
        param_ids: &[&str],
    ) -> Result<(PresetRecord, bool), String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        let value: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("not a preset file: {e}"))?;
        let stem = stem_of(path);
        let file = if format::is_envelope(&value) {
            PresetFile::from_value(value)?
        } else {
            let id = format::new_uuid();
            migrate::convert_legacy_document(value, &stem, plugin_id, self.now(), id)?
        };
        if !file.plugin.id.is_empty() && file.plugin.id != plugin_id {
            return Err(format!("That preset is for {}, not this plugin", file.plugin.id));
        }
        // A third-party blob is opaque: its plugin id is the only check.
        let blob = file.state.blob_bytes();
        let doc = match &blob {
            Some(_) if !file.plugin.id.is_empty() => serde_json::Value::Null,
            Some(_) => return Err("That preset names no plugin".to_string()),
            None => file.state.doc.clone().ok_or("the preset carries no state document")?,
        };
        let names_ours = blob.is_some()
            || doc.get("params").and_then(|p| p.as_object()).is_some_and(|p| {
                param_ids.is_empty() || p.keys().any(|k| param_ids.contains(&k.as_str()))
            });
        if !names_ours {
            return Err("That preset names none of this plugin's parameters".to_string());
        }
        let records = self.records(plugin_id, Duration::ZERO);
        let clash = file.id.is_empty() || records.iter().any(|r| r.preset.id == file.id);
        let wanted = if file.meta.name.is_empty() {
            &stem
        } else {
            &file.meta.name
        };
        let name = self.unique_name(plugin_id, wanted);
        let saved = self.save(
            plugin_id,
            SaveRequest {
                name,
                doc,
                meta: Some(file.meta.clone()),
                derived_from: file.meta.derived_from.clone(),
                plugin: file.plugin.clone(),
                id: (!clash).then(|| file.id.clone()),
                target: None,
                blob,
                blob_preset_form: None,
            },
        )?;
        Ok((saved, clash))
    }

    /// Write `preset` as one preset file at `path` (§6.4 Export): the
    /// format-1 file, factory presets included (with `plugin` filled in).
    pub fn export(
        &self,
        plugin_id: &str,
        preset: &PresetRef,
        plugin: PresetPluginInfo,
        path: &Path,
    ) -> Result<(), String> {
        let record = self
            .record(plugin_id, preset)
            .ok_or_else(|| format!("No preset named '{}'", preset.name))?;
        let blob = self.state_blob(plugin_id, preset);
        let doc: serde_json::Value = match &blob {
            Some(_) => serde_json::Value::Null,
            None => self
                .state_json(plugin_id, preset)
                .and_then(|j| serde_json::from_str(&j).ok())
                .ok_or_else(|| format!("Preset '{}' is unreadable", preset.name))?,
        };
        let mut info = plugin;
        info.id = plugin_id.to_string();
        let mut file = PresetFile::new(record.preset.id.clone(), info, record.meta.clone(), doc);
        if let Some(blob) = &blob {
            file.state = format::PresetState::clap_blob(blob);
        }
        files::atomic_write(path, file.to_text()?.as_bytes())
    }

    /// Move a user preset to the trash (D10: recoverable for
    /// [`TRASH_RETENTION`]). Returns where it went.
    pub fn trash(&self, plugin_id: &str, preset: &PresetRef) -> Result<PathBuf, String> {
        if preset.source != PresetSource::User {
            return Err("Factory presets cannot be deleted".to_string());
        }
        let trash = self
            .trash_dir(plugin_id)
            .ok_or_else(|| "No user data directory available".to_string())?;
        let mut plugins = self.plugins.lock();
        let index = plugins.entry(plugin_id.to_string()).or_default();
        self.ensure_fresh(plugin_id, index, Duration::ZERO);
        let users = index.user.as_ref().map(|u| u.records.as_slice()).unwrap_or(&[]);
        let record = find_record(users, preset)
            .cloned()
            .ok_or_else(|| format!("No preset named '{}'", preset.name))?;
        let from = record
            .path
            .clone()
            .ok_or_else(|| "User preset has no file".to_string())?;
        std::fs::create_dir_all(&trash).map_err(|e| format!("Create trash: {e}"))?;
        let stamp = self
            .now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let file_name = from
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("{}.json", record.preset.id));
        let to = trash.join(format!("{stamp}-{file_name}"));
        std::fs::rename(&from, &to).map_err(|e| format!("Delete preset: {e}"))?;
        if let Some(user) = &mut index.user {
            user.records.retain(|r| r.preset.id != record.preset.id);
        }
        index.merged = None;
        Ok(to)
    }

    /// Delete trashed presets of `plugin_id` older than
    /// [`TRASH_RETENTION`]. Runs when a plugin's directory is first
    /// indexed; returns how many files went.
    pub fn purge_trash(&self, plugin_id: &str) -> usize {
        let Some(trash) = self.trash_dir(plugin_id) else {
            return 0;
        };
        purge_trash_dir(&trash, self.now_secs())
    }

    fn now_secs(&self) -> u64 {
        self.now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// Wait for the background housekeeping (the trash sweep a first open
    /// starts) to finish. Tests; a UI never needs to.
    pub fn wait_housekeeping(&self) {
        let handles: Vec<_> = self.housekeeping.lock().drain(..).collect();
        for h in handles {
            let _ = h.join();
        }
    }

    // -----------------------------------------------------------------
    // Freshness
    // -----------------------------------------------------------------

    fn invalidate(&self, plugin_id: &str) {
        if let Some(user) = self
            .plugins
            .lock()
            .get_mut(plugin_id)
            .and_then(|i| i.user.as_mut())
        {
            user.fingerprint = None;
            user.checked_at = None;
        }
    }

    fn ensure_fresh(&self, plugin_id: &str, index: &mut PluginIndex, max_age: Duration) {
        let Some(dir) = self.plugin_dir(plugin_id) else {
            if index.user.take().is_some() {
                index.merged = None;
            }
            return;
        };
        let reset = index.user.as_ref().map(|u| u.dir != dir).unwrap_or(true);
        if reset {
            index.user = Some(UserIndex {
                dir: dir.clone(),
                opened: false,
                records: Vec::new(),
                fingerprint: None,
                checked_at: None,
            });
            index.merged = None;
        }
        let factory_ids: Vec<String> = index
            .factory
            .records
            .iter()
            .map(|r| r.preset.id.clone())
            .collect();
        let user = index.user.as_mut().expect("just ensured");
        if !user.opened {
            user.opened = true;
            let report = migrate::convert_legacy_dir(&dir, plugin_id, self.now());
            if !report.is_empty() {
                tracing::info!("presets for {plugin_id}: {report:?}");
            }
            // The trash sweep is housekeeping nobody waits for: off the
            // calling (UI / update) thread.
            if let Some(trash) = self.trash_dir(plugin_id) {
                let now = self.now_secs();
                let handle = std::thread::Builder::new()
                    .name("preset-trash-purge".into())
                    .spawn(move || purge_trash_dir(&trash, now))
                    .ok();
                self.housekeeping.lock().extend(handle);
            }
        } else if let Some(at) = user.checked_at {
            if at.elapsed() < max_age {
                return;
            }
        }
        user.checked_at = Some(Instant::now());
        let fp = fingerprint(&dir);
        if user.fingerprint.as_ref() == Some(&fp) {
            return;
        }
        user.records = scan_dir(&dir, plugin_id, self.now(), &factory_ids);
        // Re-read the fingerprint: scanning may have rewritten files
        // (ids minted, strays converted), and those writes are ours.
        user.fingerprint = Some(fingerprint(&dir));
        index.merged = None;
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Delete trashed presets older than [`TRASH_RETENTION`] at `now` (Unix
/// seconds). Returns how many went.
fn purge_trash_dir(trash: &Path, now: u64) -> usize {
    let Ok(entries) = std::fs::read_dir(trash) else {
        return 0;
    };
    let mut purged = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stamp) = name.split('-').next().and_then(|s| s.parse::<u64>().ok()) else {
            continue;
        };
        if now.saturating_sub(stamp) > TRASH_RETENTION.as_secs()
            && std::fs::remove_file(entry.path()).is_ok()
        {
            purged += 1;
        }
    }
    purged
}

fn build_factory(plugin_id: &str, entries: impl IntoIterator<Item = FactoryEntry>) -> FactorySet {
    let mut set = FactorySet::default();
    for entry in entries {
        let value: serde_json::Value = match serde_json::from_str(&entry.json) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("{plugin_id}: factory preset '{}' is not JSON: {e}", entry.name);
                continue;
            }
        };
        let (meta, doc) = if format::is_envelope(&value) {
            match PresetFile::from_value(value) {
                Ok(file) => (file.meta.clone(), file.state_json().unwrap_or_default()),
                Err(e) => {
                    tracing::error!("{plugin_id}: factory preset '{}': {e}", entry.name);
                    continue;
                }
            }
        } else {
            (PresetMeta::named(entry.name.clone()), entry.json.clone())
        };
        let mut meta = meta;
        // The Rust literal is the name of record (D7); a fleet test keeps
        // the file's `meta.name` equal to it.
        meta.name = entry.name.clone();
        set.records.push(PresetRecord {
            preset: PresetRef::factory(entry.id, entry.name),
            meta,
            plugin_version: None,
            path: None,
            bank_order: set.records.len(),
        });
        set.docs.push(doc);
    }
    set
}

pub(crate) fn find_record<'a>(
    records: &'a [PresetRecord],
    preset: &PresetRef,
) -> Option<&'a PresetRecord> {
    let of_source = || records.iter().filter(|r| r.preset.source == preset.source);
    if !preset.id.is_empty() {
        return of_source().find(|r| r.preset.id == preset.id);
    }
    of_source()
        .find(|r| r.meta.name == preset.name)
        .or_else(|| of_source().find(|r| same_name(&r.meta.name, &preset.name)))
}

fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

pub(crate) fn validate_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("A preset needs a name".to_string());
    }
    if files::sanitize_filename(trimmed).trim_matches('_').is_empty() {
        return Err("That name has no usable characters".to_string());
    }
    Ok(trimmed.to_string())
}

fn user_record(file: &PresetFile, path: PathBuf) -> PresetRecord {
    PresetRecord {
        preset: PresetRef::user(file.id.clone(), file.meta.name.clone()),
        meta: file.meta.clone(),
        plugin_version: file.plugin.version.clone(),
        path: Some(path),
        bank_order: 0,
    }
}

fn sort_users(records: &mut [PresetRecord]) {
    records.sort_by(|a, b| {
        a.meta
            .name
            .to_lowercase()
            .cmp(&b.meta.name.to_lowercase())
            .then_with(|| a.meta.name.cmp(&b.meta.name))
            .then_with(|| a.preset.id.cmp(&b.preset.id))
    });
}

fn upsert_user(index: &mut PluginIndex, record: PresetRecord) {
    if let Some(user) = &mut index.user {
        user.records.retain(|r| r.preset.id != record.preset.id);
        user.records.push(record);
        sort_users(&mut user.records);
        // Our own write changed the directory; adopt the new fingerprint
        // so it is not mistaken for someone else's.
        user.fingerprint = Some(fingerprint(&user.dir));
    }
    index.merged = None;
}

fn fingerprint(dir: &Path) -> Fingerprint {
    let Ok(meta) = std::fs::metadata(dir) else {
        return Fingerprint::Missing;
    };
    let mut entries = 0;
    let mut newest: Option<SystemTime> = None;
    if let Ok(read) = std::fs::read_dir(dir) {
        for entry in read.flatten() {
            entries += 1;
            if let Ok(m) = entry.metadata().and_then(|m| m.modified()) {
                newest = Some(newest.map_or(m, |n| n.max(m)));
            }
        }
    }
    Fingerprint::Present {
        dir_mtime: meta.modified().ok(),
        entries,
        newest,
    }
}

/// Read every preset file in `dir`.
///
/// - Unparsable files are quarantined; a file of a *newer* format version
///   is skipped and left alone (a newer build can read it).
/// - A stray legacy file is converted.
/// - A file with no name is listed under its file stem.
/// - Ids are made unique and unmistakable (§4.2). A file gets a fresh
///   UUID, written into it, when it has no id, when its id is not
///   UUID-shaped (a factory file copied in by hand carries the factory
///   slug, and a user preset must never share a marks key with a factory
///   one), when the id is a registered factory id, or when a newer file
///   already has the id but a different name or sound (a hand copy).
///   Two files with one id, the same name and the same sound (a rename
///   caught half-way, §11.4) list once, the newer.
fn scan_dir(
    dir: &Path,
    plugin_id: &str,
    now: SystemTime,
    factory_ids: &[String],
) -> Vec<PresetRecord> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(PresetFile, PathBuf, Option<SystemTime>)> = Vec::new();
    for entry in entries.flatten() {
        let mut path = entry.path();
        if !files::is_preset_path(&path) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let value: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => {
                files::quarantine(&path);
                continue;
            }
        };
        let mut file = if format::is_envelope(&value) {
            if format::is_newer_format(&value) {
                tracing::info!("{}: written by a newer build, skipped", path.display());
                continue;
            }
            match PresetFile::from_value(value) {
                Ok(file) => file,
                Err(_) => {
                    files::quarantine(&path);
                    continue;
                }
            }
        } else {
            match migrate::convert_legacy_file(&path, plugin_id, now) {
                Ok((new_path, file)) => {
                    path = new_path;
                    file
                }
                Err(e) => {
                    tracing::warn!("{}: {e}", path.display());
                    continue;
                }
            }
        };
        if file.meta.name.trim().is_empty() {
            file.meta.name = stem_of(&path);
        }
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        found.push((file, path, mtime));
    }

    // Newest first, so on an id clash the older file is the one re-minted.
    found.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.cmp(&b.1)));
    let mut records: Vec<PresetRecord> = Vec::with_capacity(found.len());
    let mut kept: Vec<PresetFile> = Vec::with_capacity(found.len());
    for (mut file, mut path, _) in found {
        let id = file.id.trim();
        let mut remint = !format::is_uuid(id) || factory_ids.iter().any(|f| f == id);
        if !remint {
            if let Some(twin) = kept.iter().find(|k| k.id == file.id) {
                if twin.meta.name == file.meta.name && twin.state == file.state {
                    continue;
                }
                remint = true;
            }
        }
        if remint {
            file.id = format::new_uuid();
            let to = dir.join(files::preset_file_name(&file.meta.name, &file.id));
            let written = files::move_before_rewrite(&path, &to)
                .and_then(|()| file.to_text())
                .and_then(|text| files::atomic_write(&to, text.as_bytes()));
            if let Err(e) = written {
                tracing::warn!("{}: could not write a new id: {e}", path.display());
                continue;
            }
            path = to;
        }
        records.push(user_record(&file, path));
        kept.push(file);
    }
    sort_users(&mut records);
    records
}

pub(crate) fn stem_of(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}
