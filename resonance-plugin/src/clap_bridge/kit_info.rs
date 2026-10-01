//! `com.resonance.kit-info` in the bridge: the pads of the kit a drum
//! plugin plays, from its [`crate::plugin::KitInfoSource`]. The ABI is
//! `resonance_common::kit_info`.

use std::ffi::{c_void, CStr};

use clack_plugin::extensions::prelude::*;
use resonance_common::kit_info::{answer, PluginKitInfo, EXTENSION_ID};

use super::ClapBridge;
use crate::plugin::ResonancePlugin;

/// The plugin's half, as clack sees it. Only ever registered (the host
/// reads the vtable), so its raw pointer is never read here.
#[derive(Copy, Clone)]
#[allow(dead_code)]
pub struct PluginKitInfoExt(RawExtension<PluginExtensionSide, PluginKitInfo>);

// SAFETY: EXTENSION_ID names exactly the `PluginKitInfo` layout
// (resonance_common::kit_info).
unsafe impl Extension for PluginKitInfoExt {
    const IDENTIFIERS: &'static [&'static CStr] = &[EXTENSION_ID];
    type ExtensionSide = PluginExtensionSide;

    unsafe fn from_raw(raw: RawExtension<Self::ExtensionSide>) -> Self {
        // SAFETY: the pointer type is upheld by the caller.
        unsafe { Self(raw.cast()) }
    }
}

// SAFETY: the implementation struct is the `PluginKitInfo` layout the id
// names.
unsafe impl<P: ResonancePlugin> ExtensionImplementation<ClapBridge<P>> for PluginKitInfoExt {
    const IMPLEMENTATION: RawExtensionImplementation =
        RawExtensionImplementation::new(&PluginKitInfo {
            get: Some(get::<P>),
        });
}

/// `[main-thread]`: the source's JSON copied into `buf` when it fits; its
/// length either way, 0 for a plugin with nothing to report.
#[allow(clippy::missing_safety_doc)]
unsafe extern "C" fn get<P: ResonancePlugin>(
    plugin: *const c_void,
    buf: *mut u8,
    cap: usize,
) -> usize {
    PluginWrapper::<ClapBridge<P>>::handle(plugin as *const clap_plugin, |plugin| {
        let main = plugin.main_thread().as_ref();
        let Some(json) = main
            .kit_info_source
            .as_ref()
            .and_then(|s| s.kit_info_json())
        else {
            return Ok(0);
        };
        // SAFETY: the host's buffer of `cap` bytes, valid for the call.
        Ok(unsafe { answer(&json, buf, cap) })
    })
    .unwrap_or(0)
}
