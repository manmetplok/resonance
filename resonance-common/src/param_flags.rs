//! `com.resonance.param-flags` — what a Resonance plugin says about its
//! parameters that CLAP's `clap_param_info.flags` has no bit for
//! (drums-plugin-rework.md §5.1).
//!
//! CLAP carries `IS_AUTOMATABLE` and `IS_READONLY`, which the host reads
//! straight from `get_info`. It has nothing for "my state leaves this
//! parameter out": the drums' `kit_select` is a slot into this machine's
//! kit library, and the kit travels inside the state as a content
//! reference instead. A host that persisted the slot next to the state
//! blob — as Resonance does with every non-default parameter in
//! `project.json` — and re-sent it after loading the blob would override
//! the reference with whatever kit sits in that slot on the loading
//! machine.
//!
//! - **Plugin side** ([`PluginParamFlags`], served by the plugin):
//!   `is_state_excluded(plugin, param_id)`, callable from any thread (it
//!   answers from metadata fixed at construction): `true` when the
//!   plugin's state neither writes nor recalls `param_id`. A host must
//!   then leave that parameter out of everything it persists or restores
//!   on the plugin's behalf — saved projects, undo snapshots, parameter
//!   re-sends after a state load. Every `IS_READONLY` parameter answers
//!   `true` as well.
//!
//! There is no host side. The id and layout live here so both ends read
//! them from one place; pointers are `c_void` so neither end needs the
//! other's CLAP bindings (`plugin` is a `clap_plugin` pointer).

use std::ffi::{c_void, CStr};

/// The extension id.
pub const EXTENSION_ID: &CStr = c"com.resonance.param-flags/1";

/// The plugin's half.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PluginParamFlags {
    /// `plugin` is the `clap_plugin` pointer, `param_id` a CLAP param id.
    /// `[thread-safe]`. `false` for an unknown id.
    pub is_state_excluded:
        Option<unsafe extern "C" fn(plugin: *const c_void, param_id: u32) -> bool>,
}
