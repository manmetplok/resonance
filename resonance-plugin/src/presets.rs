//! The preset system shared by every Resonance plugin.
//!
//! Three pieces, deliberately separable:
//!
//! - [`load`] / [`apply`] — put a preset JSON blob onto a parameter
//!   surface. This is the part that already existed.
//! - [`PresetBank`] — the *browsable* set: the plugin's baked-in factory
//!   presets plus whatever the user has saved on disk, listed together
//!   and tagged with their [`PresetSource`] so a picker can tell them
//!   apart. Owns save / rename / delete.
//! - [`PresetSession`] — which preset is loaded **right now** and whether
//!   it has been edited since. It implements
//!   [`ExtraStateSaver`](crate::plugin::ExtraStateSaver), so returning it
//!   from `ResonancePlugin::extra_state_saver` is all a plugin needs for
//!   the loaded-preset identity to survive closing the window and
//!   reopening the project (finding X2). That one hook covers both of the
//!   bridge's state paths — main-thread and audio-processor — because the
//!   bridge harvests the saver once at construction and uses it in both.
//!
//! # User presets live in the data dir
//!
//! `$XDG_DATA_HOME/resonance/plugin-presets/<clap-plugin-id>/<file>.json`
//! — on Linux `~/.local/share/resonance/plugin-presets/com.resonance.gate/`.
//! That is the same `dirs::data_dir()/resonance` root the app already uses
//! for track presets (`resonance-app/src/presets.rs`), the device-definition
//! registry and `installed.json`, so plugin presets back up and sync with
//! everything else the user has accumulated. Per-plugin subdirectories
//! because names collide across plugins ("Vocal", "Init") and because a
//! preset is only meaningful to the plugin whose param ids it carries.
//! Set [`USER_PRESET_DIR_ENV`] to override the root (tests do; so can a
//! portable install).
//!
//! # A saved preset is a full snapshot
//!
//! [`PresetBank::save`] writes through [`crate::state::params_to_json`],
//! which emits **every** declared parameter. This is structural, not a
//! convention to remember: the loader only writes the ids it finds, so a
//! partial preset silently leaves the previous patch's values in place —
//! the recall bug the delay's hand-written presets shipped with (audit
//! finding P7). Because saved presets share the state writer, they also
//! carry the state [`version`](crate::state::STATE_VERSION) and migrate
//! like project state does.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

use crate::param::Param;
use crate::plugin::ExtraStateSaver;
use crate::state::ParamRename;

/// Environment variable overriding the root directory user presets are
/// read from and written to. Points at the directory that *contains* the
/// per-plugin folders.
pub const USER_PRESET_DIR_ENV: &str = "RESONANCE_PLUGIN_PRESET_DIR";

/// Top-level state key carrying the loaded-preset identity.
pub const PRESET_STATE_KEY: &str = "preset";

/// Parse a preset JSON blob (`{"params": {id: value, ...}}`) and apply matching
/// parameter values via `Param::set_plain`. Returns `true` when the `"params"`
/// object was found, `false` on JSON parse failure or missing top-level key.
pub fn load<'a, F>(json: &str, count: usize, param_at: F) -> bool
where
    F: Fn(usize) -> &'a dyn Param,
{
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
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
    crate::state::load_params_from_json(params, &value)
}

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// Where a preset came from. Pickers show both sets in one list and use
/// this to distinguish them; only [`PresetSource::User`] presets can be
/// renamed or deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    fn parse(s: &str) -> Option<Self> {
        match s {
            "factory" => Some(PresetSource::Factory),
            "user" => Some(PresetSource::User),
            _ => None,
        }
    }
}

/// Identifies one preset: its display name plus which set it belongs to.
/// The pair is the identity, because a user preset may deliberately shadow
/// a factory name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetRef {
    pub name: String,
    pub source: PresetSource,
}

impl PresetRef {
    pub fn factory(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            source: PresetSource::Factory,
        }
    }

    pub fn user(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            source: PresetSource::User,
        }
    }

    fn to_json(&self, modified: bool) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "source": self.source.as_str(),
            "modified": modified,
        })
    }
}

/// One entry in a plugin's baked-in factory bank.
///
/// The plugins each declare a structurally identical `PresetEntry` today;
/// this is the shared spelling they can move onto (`pub use
/// resonance_plugin::presets::FactoryPreset as PresetEntry;`).
pub struct FactoryPreset {
    pub name: &'static str,
    pub json: &'static str,
}

/// The symbol [`export_clap!`](crate::export_clap) exports so a host can
/// read a plugin's factory bank without instantiating it.
///
/// The wire contract itself lives in
/// [`resonance_common::factory_presets`], which both this crate and the
/// engine depend on — the host has no business depending on the plugin
/// SDK, and vice versa.
pub use resonance_common::factory_presets::FACTORY_PRESETS_SYMBOL;

/// Encode a factory bank for the exported symbol. See
/// [`resonance_common::factory_presets::encode`].
pub fn encode_factory_bank(presets: &[FactoryPreset]) -> Option<std::ffi::CString> {
    let pairs: Vec<(&str, &str)> = presets.iter().map(|p| (p.name, p.json)).collect();
    resonance_common::factory_presets::encode(&pairs)
}

/// Decode a factory bank. See
/// [`resonance_common::factory_presets::decode`].
pub fn decode_factory_bank(text: &str) -> Vec<(String, String)> {
    resonance_common::factory_presets::decode(text)
}

// ---------------------------------------------------------------------------
// The bank: factory + user, one list
// ---------------------------------------------------------------------------

/// The browsable preset set for one plugin, and the only thing that
/// writes to the user preset directory.
pub struct PresetBank {
    plugin_id: String,
    factory: &'static [FactoryPreset],
    renames: &'static [ParamRename],
    root: Option<PathBuf>,
}

impl PresetBank {
    /// `plugin_id` is the plugin's CLAP id (`ResonancePlugin::CLAP_ID`);
    /// it names the per-plugin subdirectory.
    pub fn new(plugin_id: impl Into<String>, factory: &'static [FactoryPreset]) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            factory,
            renames: &[],
            root: None,
        }
    }

    /// Declare the plugin's parameter-id renames so presets written
    /// before a rename still recall the renamed parameter.
    pub fn with_renames(mut self, renames: &'static [ParamRename]) -> Self {
        self.renames = renames;
        self
    }

    /// Point this bank at an explicit user-preset root instead of the
    /// platform data directory. Tests use it to stay hermetic; a portable
    /// install can use it to keep presets next to the binary.
    pub fn with_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.root = Some(root.into());
        self
    }

    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    pub fn factory(&self) -> &'static [FactoryPreset] {
        self.factory
    }

    /// Directory this plugin's user presets live in. `None` only when the
    /// platform has no data directory at all and no override is set.
    pub fn user_dir(&self) -> Option<PathBuf> {
        self.root
            .clone()
            .or_else(user_preset_root)
            .map(|root| root.join(&self.plugin_id))
    }

    /// Every preset the user can pick, factory bank first (in the order
    /// the plugin declared it), then user presets sorted by name.
    pub fn list(&self) -> Vec<PresetRef> {
        let mut out: Vec<PresetRef> = self
            .factory
            .iter()
            .map(|e| PresetRef::factory(e.name))
            .collect();
        out.extend(self.list_user());
        out
    }

    /// Just the user half of [`list`](Self::list).
    pub fn list_user(&self) -> Vec<PresetRef> {
        let Some(dir) = self.user_dir() else {
            return Vec::new();
        };
        let Ok(entries) = std::fs::read_dir(&dir) else {
            // Missing directory is the normal state before the first save.
            return Vec::new();
        };
        let mut names: Vec<String> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e != "json").unwrap_or(true) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            // The display name is stored *in* the file, so names that
            // sanitise to the same file stem (or carry characters a file
            // name can't) still round-trip. Fall back to the stem for a
            // file dropped in by hand.
            let name = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| {
                    v.get("name")
                        .and_then(|n| n.as_str())
                        .map(|s| s.to_string())
                })
                .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()));
            if let Some(name) = name {
                names.push(name);
            }
        }
        names.sort_by_key(|n| n.to_lowercase());
        names.dedup();
        names.into_iter().map(PresetRef::user).collect()
    }

    /// The JSON blob behind a preset, or `None` if it no longer exists
    /// (a user preset deleted outside the app, a factory preset from an
    /// older build).
    pub fn json_for(&self, preset: &PresetRef) -> Option<String> {
        match preset.source {
            PresetSource::Factory => self
                .factory
                .iter()
                .find(|e| e.name == preset.name)
                .map(|e| e.json.to_string()),
            PresetSource::User => {
                let path = self.existing_user_path(&preset.name)?;
                std::fs::read_to_string(path).ok()
            }
        }
    }

    /// Apply a preset onto `params`. Returns `false` when the preset is
    /// gone or its JSON is unreadable — the params are left untouched.
    pub fn apply(&self, preset: &PresetRef, params: &[&dyn Param]) -> bool {
        match self.json_for(preset) {
            Some(json) => apply(&json, params, self.renames),
            None => false,
        }
    }

    /// Save the current values of every parameter as a user preset.
    ///
    /// Overwrites an existing user preset of the same name (that is what
    /// a picker's "Save" over a loaded user preset should do); factory
    /// presets are read-only and are never touched.
    pub fn save(&self, name: &str, params: &[&dyn Param]) -> Result<PresetRef, String> {
        let name = validate_name(name)?;
        let dir = self
            .user_dir()
            .ok_or_else(|| "No user data directory available".to_string())?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("Create preset directory: {e}"))?;

        // The full snapshot: `params_to_json` writes every declared id,
        // so a preset can never be a partial recall.
        let mut value = crate::state::params_to_json(params);
        if let Some(obj) = value.as_object_mut() {
            obj.insert("name".to_string(), serde_json::Value::String(name.clone()));
        }
        let text = serde_json::to_string_pretty(&value)
            .map_err(|e| format!("Serialize preset: {e}"))?;
        // Overwrite only the file that already carries this exact display
        // name; anything else gets a fresh one. Deriving the path from the
        // name instead would let "Big+Room" silently destroy "Big Room",
        // since both sanitise to `Big_Room`.
        let path = match self.existing_user_path(&name) {
            Some(path) => path,
            None => self
                .free_user_path(&name)
                .ok_or_else(|| format!("No free file name for preset '{name}'"))?,
        };
        std::fs::write(&path, text).map_err(|e| format!("Write preset: {e}"))?;
        Ok(PresetRef::user(name))
    }

    /// Rename a user preset. Factory presets cannot be renamed.
    pub fn rename(&self, preset: &PresetRef, new_name: &str) -> Result<PresetRef, String> {
        if preset.source != PresetSource::User {
            return Err("Factory presets cannot be renamed".to_string());
        }
        let new_name = validate_name(new_name)?;
        if new_name == preset.name {
            return Ok(preset.clone());
        }
        let from = self
            .existing_user_path(&preset.name)
            .ok_or_else(|| format!("No preset named '{}'", preset.name))?;
        let text = std::fs::read_to_string(&from)
            .map_err(|e| format!("Read preset '{}': {e}", preset.name))?;
        let mut value: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("Parse preset: {e}"))?;
        if let Some(obj) = value.as_object_mut() {
            obj.insert(
                "name".to_string(),
                serde_json::Value::String(new_name.clone()),
            );
        }
        // Refuse on the display name, not on the file name: two names that
        // sanitise alike are still two different presets, and renaming onto
        // one of them must not clobber it.
        if self.existing_user_path(&new_name).is_some() {
            return Err(format!("A preset named '{new_name}' already exists"));
        }
        let to = self
            .free_user_path(&new_name)
            .ok_or_else(|| format!("No free file name for preset '{new_name}'"))?;
        let text =
            serde_json::to_string_pretty(&value).map_err(|e| format!("Serialize preset: {e}"))?;
        std::fs::write(&to, text).map_err(|e| format!("Write preset: {e}"))?;
        if to != from {
            let _ = std::fs::remove_file(&from);
        }
        Ok(PresetRef::user(new_name))
    }

    /// Delete a user preset. Factory presets cannot be deleted.
    pub fn delete(&self, preset: &PresetRef) -> Result<(), String> {
        if preset.source != PresetSource::User {
            return Err("Factory presets cannot be deleted".to_string());
        }
        if let Some(path) = self.existing_user_path(&preset.name) {
            std::fs::remove_file(&path).map_err(|e| format!("Delete preset: {e}"))?;
        }
        Ok(())
    }

    /// The file currently holding the user preset called `name`, found by
    /// the display name stored *inside* each file rather than by
    /// recomputing a path from the name.
    ///
    /// Sanitising is lossy — "Big Room" and "Big+Room" both reduce to
    /// `Big_Room` — so a name cannot be turned back into the one file
    /// that holds it. Only the stored name identifies a preset, which is
    /// also what [`list_user`](Self::list_user) reports.
    fn existing_user_path(&self, name: &str) -> Option<PathBuf> {
        let dir = self.user_dir()?;
        let entries = std::fs::read_dir(&dir).ok()?;
        let mut fallback = None;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e != "json").unwrap_or(true) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            match serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| {
                    v.get("name")
                        .and_then(|n| n.as_str())
                        .map(|s| s.to_string())
                }) {
                Some(stored) if stored == name => return Some(path),
                // A file dropped in by hand carries no "name"; it is
                // listed under its stem, so match that way too — but only
                // after every stored name has had its chance.
                None if path.file_stem().map(|s| s == name).unwrap_or(false) => {
                    fallback = Some(path)
                }
                _ => {}
            }
        }
        fallback
    }

    /// A path no preset occupies yet, for a preset being created.
    ///
    /// Starts from the sanitised name and suffixes `-2`, `-3`, … until it
    /// finds a free one, so saving "Big+Room" next to an existing
    /// "Big Room" adds a second file instead of overwriting the first.
    fn free_user_path(&self, name: &str) -> Option<PathBuf> {
        let dir = self.user_dir()?;
        let stem = sanitize_filename(name);
        let first = dir.join(format!("{stem}.json"));
        if !first.exists() {
            return Some(first);
        }
        // Bounded so a corrupt directory cannot spin here; 999 distinct
        // presets colliding on one stem is far past a real library.
        (2..1000)
            .map(|n| dir.join(format!("{stem}-{n}.json")))
            .find(|p| !p.exists())
    }
}

/// Root directory containing the per-plugin user preset folders.
pub fn user_preset_root() -> Option<PathBuf> {
    if let Some(over) = std::env::var_os(USER_PRESET_DIR_ENV) {
        if !over.is_empty() {
            return Some(PathBuf::from(over));
        }
    }
    dirs::data_dir().map(|d| d.join("resonance/plugin-presets"))
}

fn validate_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("A preset needs a name".to_string());
    }
    if sanitize_filename(trimmed).trim_matches('_').is_empty() {
        // Every character sanitised away — the file name would be all
        // underscores and would collide with every other such name.
        return Err("That name has no usable characters".to_string());
    }
    Ok(trimmed.to_string())
}

/// Same rule the app uses for track presets: keep alphanumerics, `-` and
/// `_`, replace everything else. The display name lives inside the file,
/// so this only has to be a safe, stable file name.
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The session: what is loaded right now, persisted with the project
// ---------------------------------------------------------------------------

/// Tracks which preset is loaded and whether the sound has been edited
/// since, and persists that across a save/load of the project.
///
/// Plugins hand this to the bridge as their extra-state saver:
///
/// ```ignore
/// fn extra_state_saver(&self) -> Option<Arc<dyn ExtraStateSaver>> {
///     Some(self.presets.clone())
/// }
/// ```
///
/// A plugin that already has a saver of its own (file paths, loaded
/// resources) chains it in with [`PresetSession::with_extra`] instead of
/// choosing between the two.
///
/// Thread-safety: the bridge may call `save`/`load` while the plugin is
/// in the audio processor, so state lives behind a mutex and an atomic,
/// never in the plugin struct.
pub struct PresetSession {
    current: Mutex<Option<PresetRef>>,
    modified: AtomicBool,
    inner: Option<Arc<dyn ExtraStateSaver>>,
}

impl Default for PresetSession {
    fn default() -> Self {
        Self {
            current: Mutex::new(None),
            modified: AtomicBool::new(false),
            inner: None,
        }
    }
}

impl PresetSession {
    /// A session with nothing loaded.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// A session that also persists another saver's keys, for plugins
    /// that already had an [`ExtraStateSaver`].
    pub fn with_extra(inner: Arc<dyn ExtraStateSaver>) -> Arc<Self> {
        Arc::new(Self {
            current: Mutex::new(None),
            modified: AtomicBool::new(false),
            inner: Some(inner),
        })
    }

    /// The loaded preset, or `None` when the user has never picked one.
    pub fn current(&self) -> Option<PresetRef> {
        self.current.lock().clone()
    }

    /// Whether a parameter has changed since the preset was loaded.
    /// Editors show this as a dot / asterisk next to the name.
    pub fn is_modified(&self) -> bool {
        self.modified.load(Ordering::Relaxed)
    }

    /// What a picker should show: the preset name, or `placeholder` when
    /// nothing is loaded, with a trailing `*` once edited.
    pub fn label(&self, placeholder: &str) -> String {
        match self.current() {
            Some(p) if self.is_modified() => format!("{} *", p.name),
            Some(p) => p.name,
            None => placeholder.to_string(),
        }
    }

    /// Record that the sound has drifted from the loaded preset. Editors
    /// call this from their param-write path.
    pub fn mark_modified(&self) {
        self.modified.store(true, Ordering::Relaxed);
    }

    /// Set the identity directly (clearing the modified flag). Mostly for
    /// tests and for hosts restoring state by hand; the load/save methods
    /// below do it for you.
    pub fn set_current(&self, preset: Option<PresetRef>) {
        *self.current.lock() = preset;
        self.modified.store(false, Ordering::Relaxed);
    }

    /// Load a preset onto `params` and remember it. Returns `false` and
    /// leaves the identity untouched if the preset could not be read.
    pub fn load_preset(
        &self,
        bank: &PresetBank,
        preset: &PresetRef,
        params: &[&dyn Param],
    ) -> bool {
        if !bank.apply(preset, params) {
            return false;
        }
        self.set_current(Some(preset.clone()));
        true
    }

    /// Save the current sound as a user preset and make it the loaded
    /// preset — so "Save" leaves the picker showing what was just saved,
    /// unmodified.
    pub fn save_as(
        &self,
        bank: &PresetBank,
        name: &str,
        params: &[&dyn Param],
    ) -> Result<PresetRef, String> {
        let saved = bank.save(name, params)?;
        self.set_current(Some(saved.clone()));
        Ok(saved)
    }

    /// Rename a user preset, following the identity if it is the loaded
    /// one.
    pub fn rename(
        &self,
        bank: &PresetBank,
        preset: &PresetRef,
        new_name: &str,
    ) -> Result<PresetRef, String> {
        let renamed = bank.rename(preset, new_name)?;
        let mut current = self.current.lock();
        if current.as_ref() == Some(preset) {
            *current = Some(renamed.clone());
        }
        Ok(renamed)
    }

    /// Delete a user preset. If it was the loaded one the identity is
    /// cleared but the sound stays exactly as it is — deleting the file
    /// must not change what the user is hearing.
    pub fn delete(&self, bank: &PresetBank, preset: &PresetRef) -> Result<(), String> {
        bank.delete(preset)?;
        let mut current = self.current.lock();
        if current.as_ref() == Some(preset) {
            *current = None;
            self.modified.store(false, Ordering::Relaxed);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// The editor-side state machine behind the preset bar
// ---------------------------------------------------------------------------

/// What the user did in a preset bar.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PresetEvent {
    #[default]
    None,
    /// A preset was applied — every parameter may have moved, so the
    /// editor should push the whole surface to the host.
    Loaded(PresetRef),
    Saved(PresetRef),
    Renamed(PresetRef),
    Deleted(PresetRef),
}

/// Which name is being typed, when one is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamingKind {
    SaveAs,
    Rename,
}

/// The transient state of a preset bar: an in-progress name entry and
/// the last error. Deliberately free of any GUI toolkit — the egui
/// rendering in [`crate::preset_ui`] is a thin skin over this, so the
/// save / rename / delete behaviour is testable without a window.
#[derive(Default)]
pub struct PresetEditor {
    naming: Option<Naming>,
    error: Option<String>,
}

#[derive(Clone)]
struct Naming {
    kind: NamingKind,
    /// The preset being renamed; `None` for a save.
    target: Option<PresetRef>,
    buf: String,
}

impl PresetEditor {
    /// The message to show under the bar, if the last action failed.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Whether a name is being typed (editors suppress their own
    /// keyboard shortcuts while it is).
    pub fn naming(&self) -> Option<NamingKind> {
        self.naming.as_ref().map(|n| n.kind)
    }

    /// The name being typed, for the text field to edit in place.
    pub fn name_buffer(&mut self) -> Option<&mut String> {
        self.naming.as_mut().map(|n| &mut n.buf)
    }

    /// Start a "save as", seeded from the loaded preset: a user preset
    /// offers to overwrite itself, a factory preset offers a copy (so
    /// "Save" over "Vocal — Noise Gate" never looks like it will
    /// overwrite the factory bank).
    pub fn begin_save(&mut self, session: &PresetSession) {
        let initial = match session.current() {
            Some(p) if p.source == PresetSource::User => p.name,
            Some(p) => format!("{} (edit)", p.name),
            None => "My Preset".to_string(),
        };
        self.error = None;
        self.naming = Some(Naming {
            kind: NamingKind::SaveAs,
            target: None,
            buf: initial,
        });
    }

    /// Start renaming a user preset.
    pub fn begin_rename(&mut self, preset: &PresetRef) {
        self.error = None;
        self.naming = Some(Naming {
            kind: NamingKind::Rename,
            target: Some(preset.clone()),
            buf: preset.name.clone(),
        });
    }

    /// Abandon the name entry.
    pub fn cancel(&mut self) {
        self.naming = None;
        self.error = None;
    }

    /// Commit the typed name. On failure the field stays open with the
    /// offending name still in it and [`error`](Self::error) set, so the
    /// user can correct it instead of losing what they typed.
    pub fn submit(
        &mut self,
        bank: &PresetBank,
        session: &PresetSession,
        params: &[&dyn Param],
    ) -> PresetEvent {
        let Some(naming) = self.naming.clone() else {
            return PresetEvent::None;
        };
        let result = match naming.kind {
            NamingKind::SaveAs => session
                .save_as(bank, &naming.buf, params)
                .map(PresetEvent::Saved),
            NamingKind::Rename => match &naming.target {
                Some(target) => session
                    .rename(bank, target, &naming.buf)
                    .map(PresetEvent::Renamed),
                None => Err("Nothing to rename".to_string()),
            },
        };
        match result {
            Ok(event) => {
                self.naming = None;
                self.error = None;
                event
            }
            Err(e) => {
                self.error = Some(e);
                PresetEvent::None
            }
        }
    }

    /// Load a preset picked from the list.
    pub fn pick(
        &mut self,
        bank: &PresetBank,
        session: &PresetSession,
        preset: &PresetRef,
        params: &[&dyn Param],
    ) -> PresetEvent {
        if session.load_preset(bank, preset, params) {
            self.error = None;
            PresetEvent::Loaded(preset.clone())
        } else {
            self.error = Some(format!("Preset '{}' could not be loaded", preset.name));
            PresetEvent::None
        }
    }

    /// Move `delta` places through the merged factory+user list and load
    /// what lands, for the bar's ◀ / ▶ buttons.
    ///
    /// Wavetable grew private prev/next steppers around its own combo
    /// because auditioning presets one click at a time is genuinely
    /// useful; ba todo #1280 asks for that to live in the shared widget
    /// rather than as a fork, so every plugin gets it.
    ///
    /// Stepping is clamped, not wrapping: running off the end of a list
    /// and silently reappearing at the other end is disorienting when
    /// you are listening rather than looking. With nothing loaded, a
    /// step forwards starts at the first preset and a step backwards at
    /// the last, so either button gets a user into the list.
    pub fn step(
        &mut self,
        bank: &PresetBank,
        session: &PresetSession,
        delta: i32,
        params: &[&dyn Param],
    ) -> PresetEvent {
        let all = bank.list();
        if all.is_empty() {
            return PresetEvent::None;
        }
        let target = match session.current() {
            Some(current) => match all.iter().position(|p| *p == current) {
                // Clamp at both ends.
                Some(index) => {
                    let next = index as i32 + delta;
                    if next < 0 || next >= all.len() as i32 {
                        return PresetEvent::None;
                    }
                    next as usize
                }
                // Loaded preset is not in the list any more (deleted
                // outside the app): treat the step as an entry point.
                None => 0,
            },
            None if delta >= 0 => 0,
            None => all.len() - 1,
        };
        let preset = all[target].clone();
        self.pick(bank, session, &preset, params)
    }

    /// Delete the loaded user preset.
    pub fn delete(
        &mut self,
        bank: &PresetBank,
        session: &PresetSession,
        preset: &PresetRef,
    ) -> PresetEvent {
        match session.delete(bank, preset) {
            Ok(()) => {
                self.error = None;
                PresetEvent::Deleted(preset.clone())
            }
            Err(e) => {
                self.error = Some(e);
                PresetEvent::None
            }
        }
    }
}

impl ExtraStateSaver for PresetSession {
    fn save(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut map = match &self.inner {
            Some(inner) => inner.save(),
            None => serde_json::Map::new(),
        };
        if let Some(current) = self.current() {
            map.insert(
                PRESET_STATE_KEY.to_string(),
                current.to_json(self.is_modified()),
            );
        }
        map
    }

    fn load(&self, state: &serde_json::Value) {
        if let Some(inner) = &self.inner {
            inner.load(state);
        }
        // A blob saved before preset identity existed (or with no preset
        // loaded) leaves the session empty rather than inventing one.
        let entry = state.get(PRESET_STATE_KEY);
        let Some(name) = entry
            .and_then(|v| v.get("name"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        else {
            *self.current.lock() = None;
            self.modified.store(false, Ordering::Relaxed);
            return;
        };
        let source = entry
            .and_then(|v| v.get("source"))
            .and_then(|v| v.as_str())
            .and_then(PresetSource::parse)
            .unwrap_or(PresetSource::Factory);
        let modified = entry
            .and_then(|v| v.get("modified"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        *self.current.lock() = Some(PresetRef {
            name: name.to_string(),
            source,
        });
        self.modified.store(modified, Ordering::Relaxed);
    }
}
