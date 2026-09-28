//! Factory presets baked into the binary via `include_str!`. Each entry
//! is a **full** snapshot of all [`PARAM_COUNT`] parameters in the
//! plugin's native JSON state format (`{"params": {id: plain value}}`):
//! the shared loader writes only the ids it finds, so a partial preset
//! would leave the previous patch's values behind. `tests/presets.rs`
//! fails on one.
//!
//! The names are what the skills may cite; keep them stable.

use crate::params::{StereoParams, PARAM_COUNT};

pub use resonance_plugin::presets::FactoryPreset as PresetEntry;

/// The factory bank, in menu order: a reset, then the moves in the order
/// the width procedure makes them (mono the lows, widen a mono source,
/// gentle master width), then the character modes.
pub const PRESETS: &[PresetEntry] = &[
    PresetEntry {
        name: "Init — Transparent",
        json: include_str!("../presets/init_transparent.json"),
    },
    PresetEntry {
        name: "Mono Bass Below 120",
        json: include_str!("../presets/mono_bass_below_120.json"),
    },
    PresetEntry {
        name: "Widen Mono Source",
        json: include_str!("../presets/widen_mono_source.json"),
    },
    PresetEntry {
        name: "Master — Gentle Width",
        json: include_str!("../presets/master_gentle_width.json"),
    },
    PresetEntry {
        name: "Vocal — Micro-shift Double",
        json: include_str!("../presets/vocal_double_micro_shift.json"),
    },
    PresetEntry {
        name: "Pad — Diffuse Wide",
        json: include_str!("../presets/pad_diffuse_wide.json"),
    },
    PresetEntry {
        name: "Haas — Safe",
        json: include_str!("../presets/haas_safe.json"),
    },
];

/// Load one preset JSON blob onto the param surface through the shared
/// loader. Returns `false` when the blob does not parse or carries no
/// `"params"` object.
pub fn load_preset(params: &StereoParams, json: &str) -> bool {
    resonance_plugin::presets::load(json, PARAM_COUNT, |i| params.param_at(i))
}
