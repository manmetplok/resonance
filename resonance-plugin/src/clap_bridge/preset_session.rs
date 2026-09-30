//! `com.resonance.preset-session` in the bridge (plugin-preset-library.md
//! §7, slice P5): the loaded-preset identity and modified flag reported to
//! the host, and the host's list of automated params the modified
//! comparison leaves out. The ABI is `resonance_common::preset_session`.
//!
//! The session (`PresetSession`) notices a change on whatever thread it
//! happens (an editor pick, a state load, the comparison in the editor's
//! frame) and runs the notifier the bridge installed: a flag on the
//! `HostHandle` plus `request_callback`. The report itself goes out from
//! `on_main_thread`, the one thread CLAP allows it on, deduplicated
//! against the last one sent. A change of identity is also announced with
//! the standard `clap_host_preset_load.loaded()` for other hosts.

use std::ffi::{c_char, c_void, CStr, CString};

use clack_extensions::preset_discovery::preset_data::Location;
use clack_extensions::preset_discovery::HostPresetLoad;
use clack_plugin::extensions::prelude::*;
use resonance_common::preset_session::{
    HostPresetSession, IdentityReport, PluginPresetSession, EXTENSION_ID,
};

use super::shared::ClapMainThread;
use super::ClapBridge;
use crate::plugin::ResonancePlugin;

/// The host's half, as clack sees it.
#[derive(Copy, Clone)]
pub struct HostPresetSessionExt(RawExtension<HostExtensionSide, HostPresetSession>);

// SAFETY: EXTENSION_ID names exactly the `HostPresetSession` layout
// (resonance_common::preset_session), on both ends.
unsafe impl Extension for HostPresetSessionExt {
    const IDENTIFIERS: &'static [&'static CStr] = &[EXTENSION_ID];
    type ExtensionSide = HostExtensionSide;

    unsafe fn from_raw(raw: RawExtension<Self::ExtensionSide>) -> Self {
        // SAFETY: the pointer type is upheld by the caller.
        unsafe { Self(raw.cast()) }
    }
}

/// The plugin's half, as clack sees it. Only ever registered (the host
/// reads the vtable), so its raw pointer is never read here.
#[derive(Copy, Clone)]
#[allow(dead_code)]
pub struct PluginPresetSessionExt(RawExtension<PluginExtensionSide, PluginPresetSession>);

// SAFETY: as for `HostPresetSessionExt`.
unsafe impl Extension for PluginPresetSessionExt {
    const IDENTIFIERS: &'static [&'static CStr] = &[EXTENSION_ID];
    type ExtensionSide = PluginExtensionSide;

    unsafe fn from_raw(raw: RawExtension<Self::ExtensionSide>) -> Self {
        // SAFETY: the pointer type is upheld by the caller.
        unsafe { Self(raw.cast()) }
    }
}

// SAFETY: the implementation struct is the `PluginPresetSession` layout
// the id names.
unsafe impl<P: ResonancePlugin> ExtensionImplementation<ClapBridge<P>> for PluginPresetSessionExt {
    const IMPLEMENTATION: RawExtensionImplementation =
        RawExtensionImplementation::new(&PluginPresetSession {
            set_ignored_params: Some(set_ignored_params::<P>),
        });
}

#[allow(clippy::missing_safety_doc)]
unsafe extern "C" fn set_ignored_params<P: ResonancePlugin>(
    plugin: *const c_void,
    json: *const c_char,
) {
    let _ = PluginWrapper::<ClapBridge<P>>::handle(plugin as *const clap_plugin, |plugin| {
        if json.is_null() {
            return Ok(());
        }
        // SAFETY: a NUL-terminated string from the host, valid for the call.
        let text = unsafe { CStr::from_ptr(json) }.to_string_lossy();
        let ids = resonance_common::preset_session::parse_ignored_params(&text);
        let main = plugin.main_thread().as_mut();
        for (slot, flag) in main.shared.param_preset_ignored.iter().enumerate() {
            let ignored = main
                .shared
                .param_metas
                .get(slot)
                .is_some_and(|m| ids.contains(&m.clap_id));
            flag.store(ignored, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(saver) = &main.extra_state_saver {
            saver.set_ignored_params(ids);
        }
        Ok(())
    });
}

impl<'a, P: ResonancePlugin> ClapMainThread<'a, P> {
    /// Re-run the preset-modified comparison against the params as they
    /// are now: the plugin's own when it is here, the shared atomics when
    /// it is in the audio processor. `[main-thread]`.
    pub(super) fn compare_preset_sound(&mut self) {
        let Some(saver) = self.extra_state_saver.clone() else {
            return;
        };
        if let Some(plugin) = &self.plugin {
            let refs: Vec<&dyn crate::param::Param> =
                (0..plugin.param_count()).map(|i| plugin.param(i)).collect();
            saver.compare_preset_modified(&refs);
        } else {
            let temp = super::state::TempParamOwned::all_from(&self.shared);
            let refs: Vec<&dyn crate::param::Param> =
                temp.iter().map(|p| p as &dyn crate::param::Param).collect();
            saver.compare_preset_modified(&refs);
        }
    }

    /// Send the identity report if it changed since the last one, and a
    /// `loaded()` when the identity itself changed. `[main-thread]`.
    pub(super) fn report_preset_identity(&mut self) {
        let Some(json) = self.extra_state_saver.as_ref().and_then(|s| s.preset_report()) else {
            return;
        };
        if self.last_preset_report.as_deref() == Some(json.as_str()) {
            return;
        }
        let previous = self
            .last_preset_report
            .replace(json.clone())
            .and_then(|p| IdentityReport::parse(&p));
        let current = IdentityReport::parse(&json);
        let shared = self.host.shared();
        if let Some(ext) = shared.get_extension::<HostPresetSessionExt>() {
            let raw = shared.use_extension(&ext.0);
            if let (Some(report), Ok(text)) = (raw.report, CString::new(json)) {
                // SAFETY: the host pointer is live (main thread, instance
                // alive); the string outlives the call.
                unsafe {
                    report(
                        shared.as_raw() as *const _ as *const c_void,
                        text.as_ptr(),
                    )
                };
            }
        }
        // Other hosts learn the identity from CLAP's own `loaded()`: a
        // factory preset by id inside the plugin. (A user preset's file is
        // what the plugin's own browser reads; hosts that index files see
        // it through the file.)
        let identity = |r: &Option<IdentityReport>| r.as_ref().map(|r| (r.source.clone(), r.id.clone()));
        if identity(&current) != identity(&previous) {
            if let Some(r) = current.filter(|r| r.source == "factory" && !r.id.is_empty()) {
                if let Some(ext) = self.host.shared().get_extension::<HostPresetLoad>() {
                    if let Ok(key) = CString::new(r.id) {
                        ext.loaded(&mut self.host, Location::Plugin, Some(&key));
                    }
                }
            }
        }
    }
}
