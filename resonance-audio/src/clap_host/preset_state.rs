//! Presets through the plugin (plugin-preset-library.md §6.7, §8):
//! `clap.state-context` (the preset form of the state: `save_ex` /
//! `load_ex` with `CLAP_STATE_CONTEXT_FOR_PRESET`) and `clap.preset-load`
//! (`from_location`). All `[main-thread]`: the engine thread calls them,
//! never the audio thread.
//!
//! A plugin without `state-context` falls back to plain `clap.state`, which
//! is the whole state: that is a third-party plugin's preset (tier T0,
//! slice P7). A plugin with it saves only what a preset holds and lays a
//! loaded preset over its current state without a reactivation cycle
//! (a state-context load is an ordinary main-thread call that must work
//! while active; Resonance's own bridge does).

use std::ffi::{c_void, CStr};

use clap_sys::ext::preset_load::{
    clap_plugin_preset_load, CLAP_EXT_PRESET_LOAD, CLAP_EXT_PRESET_LOAD_COMPAT,
};
use clap_sys::ext::state_context::{
    clap_plugin_state_context, CLAP_EXT_STATE_CONTEXT, CLAP_STATE_CONTEXT_FOR_PRESET,
};

use super::instance::ClapInstance;
use super::state::{capture_ostream, feed_istream};
use crate::types::PluginPresetLocation as PresetLocation;

impl ClapInstance {
    fn extension(&self, id: &CStr) -> Option<*const c_void> {
        // SAFETY: `plugin` is live for `self`; `get_extension` is
        // thread-safe per CLAP and returns a pointer valid for the
        // instance's lifetime (or null).
        unsafe {
            let get = (*self.plugin).get_extension?;
            let ext = get(self.plugin, id.as_ptr());
            (!ext.is_null()).then_some(ext)
        }
    }

    fn state_context(&self) -> Option<*const clap_plugin_state_context> {
        self.extension(CLAP_EXT_STATE_CONTEXT)
            .map(|p| p as *const clap_plugin_state_context)
    }

    /// Whether the plugin has a preset form of its state (state-context).
    pub fn has_preset_state(&self) -> bool {
        self.state_context().is_some()
    }

    /// Whether the plugin can load a preset by location (preset-load).
    pub fn has_preset_load(&self) -> bool {
        self.extension(CLAP_EXT_PRESET_LOAD)
            .or_else(|| self.extension(CLAP_EXT_PRESET_LOAD_COMPAT))
            .is_some()
    }

    /// The state to store as a preset: `save_ex(FOR_PRESET)` when the
    /// plugin has state-context, else the plain full state. The flag says
    /// which one it is.
    pub fn save_preset_state(&self) -> Option<(Vec<u8>, bool)> {
        if let Some(ctx) = self.state_context() {
            // SAFETY: `ctx` came from `get_extension` for this instance.
            if let Some(save) = unsafe { (*ctx).save } {
                let plugin = self.plugin;
                let out = capture_ostream(|stream| unsafe {
                    save(plugin, stream, CLAP_STATE_CONTEXT_FOR_PRESET)
                });
                if let Some(data) = out {
                    return Some((data, true));
                }
            }
        }
        self.save_state().map(|d| (d, false))
    }

    /// Recall a preset's state: `load_ex(FOR_PRESET)` when the plugin has
    /// state-context (no reactivation cycle), else the plain state load
    /// with its activation cycle ([`Self::reload_with_state`]).
    pub fn load_preset_state(&mut self, data: &[u8]) -> bool {
        if let Some(ctx) = self.state_context() {
            // SAFETY: as in `save_preset_state`.
            if let Some(load) = unsafe { (*ctx).load } {
                let plugin = self.plugin;
                return feed_istream(data, |stream| unsafe {
                    load(plugin, stream, CLAP_STATE_CONTEXT_FOR_PRESET)
                });
            }
        }
        self.reload_with_state(data)
    }

    /// Ask the plugin to load the preset at `location` (`load_key` names
    /// one inside it). False when the plugin lacks preset-load or refuses.
    pub fn load_preset_from_location(
        &mut self,
        location: &PresetLocation,
        load_key: Option<&str>,
    ) -> bool {
        let Some(ext) = self
            .extension(CLAP_EXT_PRESET_LOAD)
            .or_else(|| self.extension(CLAP_EXT_PRESET_LOAD_COMPAT))
            .map(|p| p as *const clap_plugin_preset_load)
        else {
            return false;
        };
        // SAFETY: `ext` came from `get_extension` for this instance.
        let Some(from_location) = (unsafe { (*ext).from_location }) else {
            return false;
        };
        let key = load_key.and_then(|k| std::ffi::CString::new(k).ok());
        let (kind, path) = match location {
            PresetLocation::Plugin => (
                clap_sys::factory::preset_discovery::CLAP_PRESET_DISCOVERY_LOCATION_PLUGIN,
                None,
            ),
            PresetLocation::File(p) => (
                clap_sys::factory::preset_discovery::CLAP_PRESET_DISCOVERY_LOCATION_FILE,
                std::ffi::CString::new(p.to_string_lossy().into_owned()).ok(),
            ),
        };
        if matches!(location, PresetLocation::File(_)) && path.is_none() {
            return false;
        }
        // SAFETY: the strings outlive the call; null is how CLAP spells
        // "no path" / "no key".
        unsafe {
            from_location(
                self.plugin,
                kind,
                path.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()),
                key.as_ref().map_or(std::ptr::null(), |k| k.as_ptr()),
            )
        }
    }
}
