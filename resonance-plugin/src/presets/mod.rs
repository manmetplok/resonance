//! The preset system shared by every Resonance plugin
//! (plugin-preset-library.md).
//!
//! - [`load`] / [`apply`] — put a preset onto a parameter surface. Both
//!   take a preset file or a bare state document.
//! - [`PresetLibrary`] — the in-memory index over factory and user
//!   presets for one preset root: stable ids, metadata, search with
//!   facets, and every write (atomic), including the trash.
//! - [`PresetBank`] — one plugin's view of the library: list, load, save,
//!   rename, delete.
//! - [`PresetSession`] — which preset is loaded **right now** and whether
//!   it has been edited since. It implements
//!   [`ExtraStateSaver`](crate::plugin::ExtraStateSaver), so returning it
//!   from `ResonancePlugin::extra_state_saver` is all a plugin needs for
//!   the loaded-preset identity to survive closing the window and
//!   reopening the project.
//! - [`PresetEditor`] — the GUI-agnostic state machine behind the bar.
//!
//! # Identity
//!
//! A preset is identified by `(source, id)`, never by its name
//! ([`PresetRef`]). User presets get a UUIDv4 at save that rename,
//! re-save and metadata edits keep; factory presets declare an explicit
//! slug next to their entry ([`FactoryPreset::id`]). The file name is not
//! identity either: new files are `<sanitised name>-<id8>.json`, and
//! lookup always goes through the index.
//!
//! # User presets live in the data dir
//!
//! `$XDG_DATA_HOME/resonance/plugin-presets/<clap-plugin-id>/<file>.json`
//! — on Linux `~/.local/share/resonance/plugin-presets/com.resonance.gate/`.
//! Set [`USER_PRESET_DIR_ENV`] to override the root (tests do; so can a
//! portable install). Deleted presets go to `<root>/.trash/<clap id>/`
//! for 30 days.
//!
//! # A saved preset is a full snapshot
//!
//! [`PresetBank::save`] writes through [`crate::state::params_to_json`],
//! which emits **every** declared parameter: the loader only writes the
//! ids it finds, so a partial preset would silently leave the previous
//! patch's values in place (audit finding P7). Saved presets carry the
//! state [`version`](crate::state::STATE_VERSION) and migrate like
//! project state does.

use std::path::PathBuf;

use crate::param::Param;
use crate::state::ParamRename;

mod bank;
pub mod browser;
mod editor;
pub mod format;
mod files;
mod library;
pub mod marks;
pub mod migrate;
pub mod query;
pub mod rows;
mod session;

pub use bank::{PresetBank, SaveOptions};
pub use editor::{NamingKind, PresetEditor, PresetEvent};
pub use format::{PresetFile, PresetMeta, PresetPluginInfo, PresetState};
pub use library::{
    Clock, FactoryEntry, PresetLibrary, PresetRecord, SaveRequest, TRASH_DIR, TRASH_RETENTION,
};
pub use marks::{mark_key, MarksSource, NoMarks};
pub use rows::{PresetRow, PresetRows};
pub use query::{Facets, Hit, Query, QueryResult, Sort};
pub use browser::{FormMode, MetaForm, PresetBrowser};
pub use session::{PresetSession, SoundSnapshot};

/// Environment variable overriding the root directory user presets are
/// read from and written to. Points at the directory that *contains* the
/// per-plugin folders.
pub const USER_PRESET_DIR_ENV: &str = "RESONANCE_PLUGIN_PRESET_DIR";

/// Top-level state key carrying the loaded-preset identity.
pub const PRESET_STATE_KEY: &str = "preset";

/// How often a widget drawn every frame (the preset bar) re-checks the
/// user preset directory for changes made by another instance or process
/// (plugin-preset-library.md §4.6). Between checks it reads the library's
/// in-memory index: no disk access per frame.
pub const BAR_REFRESH: std::time::Duration = std::time::Duration::from_secs(2);

/// Parse a preset (a preset file or a bare `{"params": {id: value}}`
/// state document) and apply matching parameter values via
/// `Param::set_plain`. Returns `true` when a `"params"` object was found.
pub fn load<'a, F>(json: &str, count: usize, param_at: F) -> bool
where
    F: Fn(usize) -> &'a dyn Param,
{
    let Some(value) = format::state_document(json) else {
        return false;
    };
    let Some(map) = value.get("params").and_then(|v| v.as_object()) else {
        return false;
    };
    for i in 0..count {
        let p = param_at(i);
        if let Some(v) = map.get(p.id()).and_then(|v| v.as_f64()) {
            p.set_plain(v);
        }
    }
    true
}

/// Same as [`load`], for callers that already hold the parameter slice,
/// with the rename migration applied first so a preset written before a
/// parameter was renamed still recalls it.
pub fn apply(json: &str, params: &[&dyn Param], renames: &[ParamRename]) -> bool {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(json) else {
        return false;
    };
    crate::state::migrate(&mut value, renames);
    let kept: Vec<&dyn Param> = params
        .iter()
        .copied()
        .filter(|p| !p.preset_excluded())
        .collect();
    crate::state::load_params_from_json(&kept, &value)
}

// ---------------------------------------------------------------------------
// Whole-sound presets (plugin-preset-library.md §9.2, slice P2)
// ---------------------------------------------------------------------------

/// The params half of a preset: [`crate::state::params_to_json`] over
/// every parameter that is not [`Param::preset_excluded`].
pub fn preset_params_json(params: &[&dyn Param]) -> serde_json::Value {
    let kept: Vec<&dyn Param> = params
        .iter()
        .copied()
        .filter(|p| !p.preset_excluded())
        .collect();
    crate::state::params_to_json(&kept)
}

/// Lay a preset's state document over the plugin's current one, in place:
/// what "loading a preset" means for the whole sound.
///
/// - every param the preset carries replaces the current value, except
///   the ones `excluded` names (they keep the current value);
/// - each of the plugin's `preset_keys` is taken from the preset, and
///   *removed* when the preset lacks it — the plugin's state loader then
///   decides what absence means, exactly as for a project that lacks the
///   key: the wavetable clears its user tables (a preset without them has
///   none), the amp and the IR keep their current asset (a params-only,
///   pre-P2 preset);
/// - every other key (session and UI state) is left as it is;
/// - the preset's `"preset"` identity key, if any, replaces the current
///   one.
///
/// Run the plugin's rename migration on `preset_doc` first.
pub fn overlay_preset(
    current: &mut serde_json::Value,
    preset_doc: &serde_json::Value,
    preset_keys: &[&str],
    excluded: &dyn Fn(&str) -> bool,
) {
    let Some(state) = current.as_object_mut() else {
        return;
    };
    if let Some(preset_params) = preset_doc.get("params").and_then(|p| p.as_object()) {
        let params = state
            .entry("params")
            .or_insert_with(|| serde_json::Value::Object(Default::default()));
        if let Some(params) = params.as_object_mut() {
            for (id, value) in preset_params {
                if !excluded(id) {
                    params.insert(id.clone(), value.clone());
                }
            }
        }
    }
    for key in preset_keys {
        match preset_doc.get(*key) {
            Some(v) => {
                state.insert(key.to_string(), v.clone());
            }
            None => {
                state.remove(*key);
            }
        }
    }
    if let Some(identity) = preset_doc.get(PRESET_STATE_KEY) {
        state.insert(PRESET_STATE_KEY.to_string(), identity.clone());
    }
}

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// Where a preset came from. Only [`PresetSource::User`] presets can be
/// renamed or deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresetSource {
    /// Baked into the plugin binary via `include_str!`.
    Factory,
    /// A file in the user preset directory.
    User,
}

impl PresetSource {
    /// Stable spelling used in saved plugin state.
    pub fn as_str(self) -> &'static str {
        match self {
            PresetSource::Factory => "factory",
            PresetSource::User => "user",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "factory" => Some(PresetSource::Factory),
            "user" => Some(PresetSource::User),
            _ => None,
        }
    }
}

/// Identifies one preset. Equality is `(source, id)`; `name` is only a
/// display hint (and what error messages quote).
///
/// A reference with an empty `id` is **unresolved**: it came from a
/// project saved before preset ids existed. Two unresolved refs are equal
/// when their names are; an unresolved ref never equals a resolved one
/// (so `==` stays an equivalence relation). Use [`PresetRef::matches`] to
/// compare a possibly-unresolved ref against a listed one, and
/// [`PresetBank::resolve`] to give it its id.
#[derive(Debug, Clone, Eq)]
pub struct PresetRef {
    pub id: String,
    pub source: PresetSource,
    pub name: String,
}

impl PartialEq for PresetRef {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && self.id == other.id
            && (!self.id.is_empty() || self.name == other.name)
    }
}

impl PresetRef {
    pub fn factory(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            source: PresetSource::Factory,
            name: name.into(),
        }
    }

    pub fn user(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            source: PresetSource::User,
            name: name.into(),
        }
    }

    /// A reference by name only, to be resolved against a bank.
    pub fn unresolved(source: PresetSource, name: impl Into<String>) -> Self {
        Self {
            id: String::new(),
            source,
            name: name.into(),
        }
    }

    pub fn is_resolved(&self) -> bool {
        !self.id.is_empty()
    }

    /// Whether `self` and `other` name the same preset, allowing either to
    /// be unresolved: by id when both have one, otherwise by source and
    /// case-insensitive name (the same rule the index resolves by).
    pub fn matches(&self, other: &PresetRef) -> bool {
        if self.source != other.source {
            return false;
        }
        if self.is_resolved() && other.is_resolved() {
            return self.id == other.id;
        }
        self.name.trim().to_lowercase() == other.name.trim().to_lowercase()
    }

    fn to_json(&self, modified: bool) -> serde_json::Value {
        let mut v = serde_json::json!({
            "name": self.name,
            "source": self.source.as_str(),
            "modified": modified,
        });
        if self.is_resolved() {
            v["id"] = serde_json::Value::String(self.id.clone());
        }
        v
    }
}

/// One entry in a plugin's baked-in factory bank.
///
/// `json` is a format-1 preset file ([`format`]) whose `id` and
/// `meta.name` equal the fields here (a fleet test holds them together).
/// `name` stays a Rust literal because `resonance-mcp`'s lockstep test
/// reads factory names out of `presets.rs` source (D7).
pub struct FactoryPreset {
    /// Stable slug, unique per plugin, never reused for another sound.
    pub id: &'static str,
    pub name: &'static str,
    pub json: &'static str,
}

impl FactoryPreset {
    /// The parsed preset file, or why it does not parse.
    pub fn file(&self) -> Result<PresetFile, String> {
        PresetFile::parse(self.json)
    }

    /// The state document this preset carries.
    pub fn state_doc(&self) -> serde_json::Value {
        format::state_document(self.json).unwrap_or(serde_json::Value::Null)
    }

    /// The state document as JSON text: what the loaders and a plugin's
    /// `load_state` take (they also accept the whole file, [`Self::json`]).
    pub fn state_json(&self) -> String {
        self.state_doc().to_string()
    }

    /// Its descriptive metadata (empty when the file does not parse).
    pub fn meta(&self) -> PresetMeta {
        self.file().map(|f| f.meta).unwrap_or_default()
    }
}

/// The symbol [`export_clap!`](crate::export_clap) exports so a host can
/// read a plugin's factory bank without instantiating it.
pub use resonance_common::factory_presets::FACTORY_PRESETS_SYMBOL;

/// Encode a factory bank for the exported symbol:
/// `[{"id", "name", "json": <state document>, "meta"}]`.
///
/// `json` stays the bare state document, so a host decoding with
/// [`resonance_common::factory_presets::decode`] (which reads only `name`
/// and `json`) keeps working; `id` and `meta` ride alongside for hosts
/// that read them ([`decode_factory_entries`]).
pub fn encode_factory_bank(presets: &[FactoryPreset]) -> Option<std::ffi::CString> {
    let entries: Vec<serde_json::Value> = presets
        .iter()
        .map(|p| {
            let meta = p.meta();
            serde_json::json!({
                "id": p.id,
                "name": p.name,
                "json": p.state_doc(),
                "meta": serde_json::to_value(&meta).unwrap_or(serde_json::Value::Null),
            })
        })
        .collect();
    let text = serde_json::to_string(&serde_json::Value::Array(entries)).ok()?;
    std::ffi::CString::new(text).ok()
}

/// Decode a factory bank as `(name, state json)` pairs. See
/// [`resonance_common::factory_presets::decode`].
pub fn decode_factory_bank(text: &str) -> Vec<(String, String)> {
    resonance_common::factory_presets::decode(text)
}

/// Decode a factory bank with ids and metadata, ready for
/// [`PresetLibrary::register_factory_entries`]. An entry without an id
/// (an older build) gets one slugged from its name.
pub fn decode_factory_entries(text: &str) -> Vec<FactoryEntry> {
    let Ok(serde_json::Value::Array(entries)) = serde_json::from_str::<serde_json::Value>(text)
    else {
        return Vec::new();
    };
    entries
        .into_iter()
        .filter_map(|entry| {
            let name = entry.get("name")?.as_str()?.to_string();
            let doc = entry.get("json")?.clone();
            let id = entry
                .get("id")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .or_else(|| resonance_common::library_marks::normalize_tag(&name))?;
            let meta: PresetMeta = entry
                .get("meta")
                .and_then(|m| serde_json::from_value(m.clone()).ok())
                .unwrap_or_else(|| PresetMeta::named(name.clone()));
            let file = PresetFile::new(id.clone(), PresetPluginInfo::default(), meta, doc);
            Some(FactoryEntry {
                id,
                name,
                json: file.to_text().ok()?,
            })
        })
        .collect()
}

static DEFAULT_ROOTS_OVERRIDE: std::sync::OnceLock<(PathBuf, Option<PathBuf>)> =
    std::sync::OnceLock::new();

/// Point the process-wide default preset root (and, when given, the
/// directory of the marks store the default library opens) somewhere else
/// for the rest of the process — a test seam with no environment variable,
/// so no `setenv` race with a test harness's threads. The first call wins.
pub fn override_default_roots(root: PathBuf, marks_dir: Option<PathBuf>) {
    let _ = DEFAULT_ROOTS_OVERRIDE.set((root, marks_dir));
}

/// The marks directory [`override_default_roots`] set, if any.
pub(crate) fn default_marks_dir_override() -> Option<PathBuf> {
    DEFAULT_ROOTS_OVERRIDE.get().and_then(|(_, m)| m.clone())
}

/// Root directory containing the per-plugin user preset folders.
pub fn user_preset_root() -> Option<PathBuf> {
    if let Some((root, _)) = DEFAULT_ROOTS_OVERRIDE.get() {
        return Some(root.clone());
    }
    if let Some(over) = std::env::var_os(USER_PRESET_DIR_ENV) {
        if !over.is_empty() {
            return Some(PathBuf::from(over));
        }
    }
    dirs::data_dir().map(|d| d.join("resonance/plugin-presets"))
}
