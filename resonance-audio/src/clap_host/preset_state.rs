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

use std::ffi::{c_char, c_void, CStr};

use clap_sys::ext::preset_load::{
    clap_host_preset_load, clap_plugin_preset_load, CLAP_EXT_PRESET_LOAD,
    CLAP_EXT_PRESET_LOAD_COMPAT,
};
use clap_sys::host::clap_host;
use resonance_common::preset_session::{HostPresetSession, PluginPresetSession, EXTENSION_ID};
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

// ---------------------------------------------------------------------------
// The plugin's reports back (slice P5)
// ---------------------------------------------------------------------------

/// What a plugin told the host about its loaded preset since the last
/// drain, in the order it said it.
#[derive(Debug, Clone, PartialEq)]
pub enum PresetHostReport {
    /// `com.resonance.preset-session` `report`: the identity and modified
    /// flag; `None` for "nothing loaded".
    Identity(Option<resonance_common::preset_session::IdentityReport>),
    /// `clap_host_preset_load.loaded()`.
    Loaded {
        location: PresetLocation,
        load_key: Option<String>,
    },
    /// `clap_host_preset_load.on_error()`.
    Error { message: String },
}

/// The host's `clap_host_preset_load` vtable.
pub(super) fn host_preset_load_ext() -> clap_host_preset_load {
    clap_host_preset_load {
        on_error: Some(host_preset_on_error),
        loaded: Some(host_preset_loaded),
    }
}

/// The host's `com.resonance.preset-session` vtable.
pub(super) fn host_preset_session_ext() -> HostPresetSession {
    HostPresetSession {
        report: Some(host_preset_report),
    }
}

unsafe fn c_str(p: *const c_char) -> Option<String> {
    if p.is_null() {
        None
    } else {
        // SAFETY: a NUL-terminated string from the plugin, valid for the call.
        Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
    }
}

unsafe fn location_from(kind: u32, location: *const c_char) -> Option<PresetLocation> {
    use clap_sys::factory::preset_discovery::{
        CLAP_PRESET_DISCOVERY_LOCATION_FILE, CLAP_PRESET_DISCOVERY_LOCATION_PLUGIN,
    };
    match kind {
        CLAP_PRESET_DISCOVERY_LOCATION_PLUGIN => Some(PresetLocation::Plugin),
        CLAP_PRESET_DISCOVERY_LOCATION_FILE => {
            // SAFETY: as for `c_str`.
            unsafe { c_str(location) }.map(|p| PresetLocation::File(p.into()))
        }
        _ => None,
    }
}

/// `[main-thread]` per CLAP; only queued here, drained by the engine's
/// host-request poll.
unsafe extern "C" fn host_preset_loaded(
    host: *const clap_host,
    kind: u32,
    location: *const c_char,
    load_key: *const c_char,
) {
    // SAFETY: the host pointer is ours; strings valid for the call.
    unsafe {
        let Some(data) = super::host_data_from(host) else {
            return;
        };
        let Some(location) = location_from(kind, location) else {
            return;
        };
        data.preset_reports.lock().push(PresetHostReport::Loaded {
            location,
            load_key: c_str(load_key),
        });
    }
}

unsafe extern "C" fn host_preset_on_error(
    host: *const clap_host,
    _kind: u32,
    _location: *const c_char,
    _load_key: *const c_char,
    os_error: i32,
    msg: *const c_char,
) {
    // SAFETY: as for `host_preset_loaded`.
    unsafe {
        let Some(data) = super::host_data_from(host) else {
            return;
        };
        let message = c_str(msg).unwrap_or_else(|| format!("preset load failed ({os_error})"));
        data.preset_reports.lock().push(PresetHostReport::Error { message });
    }
}

unsafe extern "C" fn host_preset_report(host: *const c_void, json: *const c_char) {
    // SAFETY: as for `host_preset_loaded`.
    unsafe {
        let Some(data) = super::host_data_from(host as *const clap_host) else {
            return;
        };
        let Some(text) = c_str(json) else {
            return;
        };
        let identity = resonance_common::preset_session::IdentityReport::parse(&text);
        data.preset_reports.lock().push(PresetHostReport::Identity(identity));
    }
}

impl ClapInstance {
    /// Everything the plugin reported about its preset since the last
    /// call. Engine thread.
    pub fn take_preset_reports(&mut self) -> Vec<PresetHostReport> {
        std::mem::take(&mut *self.host_data.preset_reports.lock())
    }

    /// Take the plugin's queued `on_error` messages (joined), leaving its
    /// other reports queued: a failed `from_location` reports once.
    pub fn take_preset_error(&mut self) -> Option<String> {
        let mut reports = self.host_data.preset_reports.lock();
        let mut messages = Vec::new();
        reports.retain(|r| match r {
            PresetHostReport::Error { message } => {
                messages.push(message.clone());
                false
            }
            _ => true,
        });
        (!messages.is_empty()).then(|| messages.join("; "))
    }

    /// Ask for the params to be re-read at the next host-request poll.
    pub fn request_params_refresh(&mut self) {
        self.host_data
            .params_refresh
            .store(true, std::sync::atomic::Ordering::Release);
    }

    /// Whether the params should be re-read (a values rescan, or a load's
    /// second look), clearing the flag.
    pub fn take_params_refresh(&mut self) -> bool {
        self.host_data
            .params_refresh
            .swap(false, std::sync::atomic::Ordering::AcqRel)
    }

    /// Whether this is one of Resonance's own plugins: it serves
    /// `com.resonance.preset-session` (every plugin built on the SDK's
    /// bridge does). Provenance decides how its state is stored in a
    /// preset, never the state's content.
    pub fn is_first_party(&self) -> bool {
        self.extension(EXTENSION_ID).is_some()
    }

    /// Tell a Resonance plugin which params the host automates, so its
    /// modified comparison leaves them out (D8). False for a plugin
    /// without `com.resonance.preset-session`. `[main-thread]`.
    pub fn set_preset_ignored_params(&mut self, clap_ids: &[u32]) -> bool {
        let Some(ext) = self
            .extension(EXTENSION_ID)
            .map(|p| p as *const PluginPresetSession)
        else {
            return false;
        };
        // SAFETY: `ext` came from `get_extension` for this id, whose layout
        // is `PluginPresetSession` on both ends.
        let Some(set) = (unsafe { (*ext).set_ignored_params }) else {
            return false;
        };
        let Ok(json) = std::ffi::CString::new(
            resonance_common::preset_session::ignored_params_json(clap_ids),
        )
        else {
            return false;
        };
        // SAFETY: the plugin is live; the string outlives the call.
        unsafe { set(self.plugin as *const c_void, json.as_ptr()) };
        true
    }
}
