//! Factory presets baked into the binary via `include_str!` (ba todo
//! #1137, design doc #264 req-6) — the same mechanism as
//! `plugins/resonance-compressor/src/presets.rs`. Each entry is a full
//! 29-parameter snapshot in the plugin's native JSON state format
//! (`{"params": {id: plain value, ...}}`), so loading is deterministic:
//! the loader walks the whole param surface and calls `set_plain`.
//!
//! Preset names follow the demo states of the approved design prototype
//! (design/granular-delay-editor/index.html on ba/design-199), plus a
//! reverse-texture patch.

use crate::params::{GranularDelayParams, PARAM_COUNT};

pub struct PresetEntry {
    pub name: &'static str,
    pub json: &'static str,
}

pub const PRESETS: &[PresetEntry] = &[
    PresetEntry {
        name: "Init — Per-Grain Cloud",
        json: include_str!("../presets/init_per_grain_cloud.json"),
    },
    PresetEntry {
        name: "Eighth-Triplet Echo",
        json: include_str!("../presets/eighth_triplet_echo.json"),
    },
    PresetEntry {
        name: "D-Minor Shimmer Cloud",
        json: include_str!("../presets/d_minor_shimmer_cloud.json"),
    },
    PresetEntry {
        name: "Vocal Doubler (PSOLA)",
        json: include_str!("../presets/vocal_doubler_psola.json"),
    },
    PresetEntry {
        name: "Reverse Haze",
        json: include_str!("../presets/reverse_haze.json"),
    },
    PresetEntry {
        name: "Frozen Drone",
        json: include_str!("../presets/frozen_drone.json"),
    },
];

/// Load one preset JSON blob onto the param surface through the shared
/// loader (`resonance_plugin::presets::load`), mirroring the
/// compressor editor's `load_preset`. Returns `false` on parse failure.
pub fn load_preset(params: &GranularDelayParams, json: &str) -> bool {
    resonance_plugin::presets::load(json, PARAM_COUNT, |i| params.param_at(i))
}
