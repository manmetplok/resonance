//! `com.resonance.param-flags` in the bridge: tells the host which params
//! the state leaves out ([`crate::param::Param::state_excluded`]), so it
//! does not persist or re-send them next to the state blob. The ABI is
//! `resonance_common::param_flags`.

use std::ffi::{c_void, CStr};

use clack_plugin::extensions::prelude::*;
use resonance_common::param_flags::{PluginParamFlags, EXTENSION_ID};

use super::ClapBridge;
use crate::plugin::ResonancePlugin;

/// The plugin's half, as clack sees it. Only ever registered (the host
/// reads the vtable), so its raw pointer is never read here.
#[derive(Copy, Clone)]
#[allow(dead_code)]
pub struct PluginParamFlagsExt(RawExtension<PluginExtensionSide, PluginParamFlags>);

// SAFETY: EXTENSION_ID names exactly the `PluginParamFlags` layout
// (resonance_common::param_flags).
unsafe impl Extension for PluginParamFlagsExt {
    const IDENTIFIERS: &'static [&'static CStr] = &[EXTENSION_ID];
    type ExtensionSide = PluginExtensionSide;

    unsafe fn from_raw(raw: RawExtension<Self::ExtensionSide>) -> Self {
        // SAFETY: the pointer type is upheld by the caller.
        unsafe { Self(raw.cast()) }
    }
}

// SAFETY: the implementation struct is the `PluginParamFlags` layout the
// id names.
unsafe impl<P: ResonancePlugin> ExtensionImplementation<ClapBridge<P>> for PluginParamFlagsExt {
    const IMPLEMENTATION: RawExtensionImplementation =
        RawExtensionImplementation::new(&PluginParamFlags {
            is_state_excluded: Some(is_state_excluded::<P>),
        });
}

/// `[thread-safe]`: answered from the shared param metadata, fixed at
/// construction, so it needs neither the main thread nor the plugin.
#[allow(clippy::missing_safety_doc)]
unsafe extern "C" fn is_state_excluded<P: ResonancePlugin>(
    plugin: *const c_void,
    param_id: u32,
) -> bool {
    PluginWrapper::<ClapBridge<P>>::handle(plugin as *const clap_plugin, |plugin| {
        let shared = plugin.shared();
        Ok(shared
            .find_slot(param_id)
            .is_some_and(|slot| shared.param_metas[slot].state_excluded))
    })
    .unwrap_or(false)
}
