//! `com.resonance.kit-info` — what a Resonance drum instance says about
//! the pads of the kit it is playing (drums-plugin-rework.md §8, slice K9):
//! each pad's MIDI note, its name in that kit, and whether the kit has a
//! piece for it at all.
//!
//! CLAP has no channel for this. Its `note-name` extension names notes but
//! cannot say a pad is silent, and the kit's *name* already travels as the
//! text of the drums' `kit_select` parameter, so this carries only the
//! pads. The app's drum-group kit picker reads it, so it lists the pads of
//! the kit the track really plays instead of a fixed table.
//!
//! - **Plugin side** ([`PluginKitInfo`], served by the plugin):
//!   `get(plugin, buf, cap)`, `[main-thread]`: writes the [`KitInfo`] JSON
//!   (UTF-8, no terminating NUL) into `buf` when it fits in `cap` bytes,
//!   and returns its length in bytes either way — `0` when the plugin has
//!   nothing to report. A host calls it with a buffer, and again with a
//!   bigger one when the answer did not fit.
//!
//! **When the host reads it:** once after creating the instance, and again
//! after every parameter rescan the plugin requests
//! (`clap_host_params.rescan`). A plugin whose pads change requests one —
//! the drums do whenever a kit hand-off changes the pads they report.
//!
//! There is no host side. The id and layout live here so both ends read
//! them from one place; pointers are `c_void` so neither end needs the
//! other's CLAP bindings (`plugin` is a `clap_plugin` pointer).

use std::ffi::{c_void, CStr};

use serde::{Deserialize, Serialize};

/// The extension id.
pub const EXTENSION_ID: &CStr = c"com.resonance.kit-info/1";

/// The plugin's half.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PluginKitInfo {
    /// `plugin` is the `clap_plugin` pointer; `buf` a buffer of `cap`
    /// bytes (may be null when `cap` is 0). `[main-thread]`.
    pub get: Option<unsafe extern "C" fn(plugin: *const c_void, buf: *mut u8, cap: usize) -> usize>,
}

/// One pad of the kit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KitInfoPad {
    /// The MIDI note that plays it.
    pub note: u8,
    /// Its name in this kit ("Kick", or the kit's own `_meta` name).
    pub name: String,
    /// Whether it sounds: the kit has a piece for it. An absent pad plays
    /// nothing.
    pub present: bool,
}

/// The pads of the kit an instance plays, in the plugin's pad order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KitInfo {
    /// `false` while the instance plays its built-in kit (every pad
    /// present, under its General MIDI name).
    #[serde(default)]
    pub from_kit: bool,
    pub pads: Vec<KitInfoPad>,
}

impl KitInfo {
    /// The JSON `get` returns.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Parse what `get` returned; `None` for nothing or garbage.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() {
            return None;
        }
        serde_json::from_slice(bytes).ok()
    }
}

/// The plugin half's `get`, given the JSON to answer with: copies it into
/// `buf` when it fits and returns its length.
///
/// # Safety
/// `buf` must be valid for `cap` bytes of writes (or `cap` must be 0).
pub unsafe fn answer(json: &str, buf: *mut u8, cap: usize) -> usize {
    let bytes = json.as_bytes();
    if !buf.is_null() && bytes.len() <= cap {
        // SAFETY: `buf` holds `cap >= bytes.len()` bytes (caller's contract).
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, bytes.len()) };
    }
    bytes.len()
}
