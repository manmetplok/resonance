//! [`PresetSession`]: what is loaded right now, persisted with the project.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

use super::bank::{PresetBank, SaveOptions};
use super::{PresetRef, PresetSource, BAR_REFRESH, PRESET_STATE_KEY};
use crate::param::Param;
use crate::plugin::ExtraStateSaver;

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
/// The identity is persisted as `"preset": {id, name, source, modified}`.
/// A project written before preset ids existed carries `{name, source}`
/// only. A session built with [`for_plugin`](Self::for_plugin) (every
/// plugin's) resolves that to an id by name **at state load** (§13), so
/// the next save writes the new shape; a session without a bank keeps it
/// unresolved until [`resolve`](Self::resolve) is called.
///
/// Thread-safety: the bridge may call `save`/`load` while the plugin is
/// in the audio processor, so state lives behind a mutex and an atomic,
/// never in the plugin struct.
pub struct PresetSession {
    current: Mutex<Option<PresetRef>>,
    modified: AtomicBool,
    inner: Option<Arc<dyn ExtraStateSaver>>,
    bank: Option<BankFactory>,
}

/// Builds the bank a session resolves name-only identities against.
/// Deferred so constructing a plugin never touches the disk.
pub type BankFactory = Box<dyn Fn() -> PresetBank + Send + Sync>;

impl Default for PresetSession {
    fn default() -> Self {
        Self {
            current: Mutex::new(None),
            modified: AtomicBool::new(false),
            inner: None,
            bank: None,
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
        Self::resolving(None, Some(inner))
    }

    /// The session for plugin `P`: resolves a name-only identity against
    /// `PresetBank::for_plugin::<P>()` when state loads.
    pub fn for_plugin<P: crate::ResonancePlugin>() -> Arc<Self> {
        Self::resolving(Some(Box::new(PresetBank::for_plugin::<P>)), None)
    }

    /// [`for_plugin`](Self::for_plugin) chaining another saver's keys.
    pub fn for_plugin_with_extra<P: crate::ResonancePlugin>(
        inner: Arc<dyn ExtraStateSaver>,
    ) -> Arc<Self> {
        Self::resolving(Some(Box::new(PresetBank::for_plugin::<P>)), Some(inner))
    }

    /// A session resolving against whatever bank `bank` builds (tests point
    /// it at a private root).
    pub fn resolving(
        bank: Option<BankFactory>,
        inner: Option<Arc<dyn ExtraStateSaver>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            current: Mutex::new(None),
            modified: AtomicBool::new(false),
            inner,
            bank,
        })
    }

    /// The loaded preset, or `None` when the user has never picked one.
    pub fn current(&self) -> Option<PresetRef> {
        self.current.lock().clone()
    }

    /// Whether a parameter has changed since the preset was loaded.
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

    /// Set the identity directly (clearing the modified flag).
    pub fn set_current(&self, preset: Option<PresetRef>) {
        *self.current.lock() = preset;
        self.modified.store(false, Ordering::Relaxed);
    }

    /// Give an unresolved identity (a project from before preset ids)
    /// its id, and refresh a resolved one's display name after a rename
    /// elsewhere. Leaves the modified flag alone. No-op when nothing is
    /// loaded or the preset no longer exists.
    ///
    /// Reads the library's cached index ([`BAR_REFRESH`]), never the disk
    /// directly, because the bar calls it every frame.
    pub fn resolve(&self, bank: &PresetBank) {
        let Some(current) = self.current() else {
            return;
        };
        let found = bank.resolve_cached(&current, BAR_REFRESH);
        self.adopt(&current, found);
    }

    /// Resolve against the directory as it is now (state load).
    fn resolve_now(&self, bank: &PresetBank) {
        let Some(current) = self.current().filter(|c| !c.is_resolved()) else {
            return;
        };
        let found = bank.resolve(&current);
        self.adopt(&current, found);
    }

    fn adopt(&self, current: &PresetRef, found: Option<PresetRef>) {
        let Some(found) = found else {
            return;
        };
        if found.id != current.id || found.name != current.name {
            let mut slot = self.current.lock();
            if slot.as_ref() == Some(current) {
                *slot = Some(found);
            }
        }
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
        let resolved = bank.resolve(preset).unwrap_or_else(|| preset.clone());
        self.set_current(Some(resolved));
        true
    }

    /// Save the current sound as a user preset and make it the loaded
    /// preset — so "Save" leaves the picker showing what was just saved,
    /// unmodified.
    ///
    /// A new preset inherits the loaded preset's descriptive metadata
    /// (category, instrument, genres, character, tags) and records it as
    /// `derived_from` (§6.4 "Save as…"). Saving over an existing user
    /// preset keeps that preset's own metadata and lineage.
    pub fn save_as(
        &self,
        bank: &PresetBank,
        name: &str,
        params: &[&dyn Param],
    ) -> Result<PresetRef, String> {
        let target = PresetRef::unresolved(PresetSource::User, name);
        let overwrites = bank.record(&target).is_some();
        let options = match self.current().and_then(|c| bank.record(&c)) {
            Some(loaded) if !overwrites => SaveOptions {
                meta: Some(loaded.meta.clone()),
                derived_from: Some(loaded.preset.id.clone()),
            },
            _ => SaveOptions::default(),
        };
        let saved = bank.save_with(name, params, options)?;
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
        if current
            .as_ref()
            .is_some_and(|c| c.matches(&renamed) || c.matches(preset))
        {
            *current = Some(renamed.clone());
        }
        Ok(renamed)
    }

    /// Delete a user preset. If it was the loaded one the identity is
    /// cleared but the sound stays exactly as it is — deleting the file
    /// must not change what the user is hearing.
    pub fn delete(&self, bank: &PresetBank, preset: &PresetRef) -> Result<(), String> {
        let resolved = bank.resolve(preset);
        bank.delete(preset)?;
        let mut current = self.current.lock();
        let hit = current.as_ref().is_some_and(|c| {
            c.matches(preset) || resolved.as_ref().is_some_and(|r| c.matches(r))
        });
        if hit {
            *current = None;
            self.modified.store(false, Ordering::Relaxed);
        }
        Ok(())
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
        let id = entry
            .and_then(|v| v.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let modified = entry
            .and_then(|v| v.get("modified"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        *self.current.lock() = Some(PresetRef {
            id: id.to_string(),
            source,
            name: name.to_string(),
        });
        self.modified.store(modified, Ordering::Relaxed);
        if let Some(make_bank) = &self.bank {
            self.resolve_now(&make_bank());
        }
    }
}
