//! Factory presets.
//!
//! Every entry is a **full snapshot**: all [`PARAM_COUNT`] ids are
//! present. The shared loader ([`resonance_plugin::presets::load`])
//! only writes ids it finds in the map, so a partial preset silently
//! inherits whatever the previous patch left behind — loading
//! "Slapback" over a trance-gated patch used to leave the gate
//! running. `tests/presets.rs` enforces the full-coverage rule so a
//! newly added parameter cannot quietly reintroduce a partial recall.

use crate::params::{DelayParams, PARAM_COUNT};

/// The factory-preset entry type is the shared one, so this crate's
/// `PRESETS` can be handed straight to `presets::PresetBank` (ba todo
/// #1358). The alias keeps the crate-local name every call site uses.
pub use resonance_plugin::presets::FactoryPreset as PresetEntry;

/// Apply a preset JSON blob to `params`. Returns `false` when the blob
/// is not a `{"params": {...}}` object.
pub fn load_preset(params: &DelayParams, json: &str) -> bool {
    resonance_plugin::presets::load(json, PARAM_COUNT, |i| params.param_at(i))
}

pub const PRESETS: &[PresetEntry] = &[
    PresetEntry {
        id: "quarter-note",
        name: "Quarter Note",
        json: include_str!("../presets/quarter_note.json"),
    },
    PresetEntry {
        id: "dotted-eighth",
        name: "Dotted Eighth",
        json: include_str!("../presets/dotted_eighth.json"),
    },
    PresetEntry {
        id: "slapback",
        name: "Slapback",
        json: include_str!("../presets/slapback.json"),
    },
    PresetEntry {
        id: "dub",
        name: "Dub",
        json: include_str!("../presets/dub.json"),
    },
    PresetEntry {
        id: "ping-pong-eighth",
        name: "Ping-Pong Eighth",
        json: include_str!("../presets/ping_pong_eighth.json"),
    },
    PresetEntry {
        id: "lo-fi-tape",
        name: "Lo-Fi Tape",
        json: include_str!("../presets/lo_fi_tape.json"),
    },
];
