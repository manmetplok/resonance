//! Factory presets baked into the binary via `include_str!` (ba todo
//! #1330, audit finding C4) — the same mechanism the compressor and the
//! granular delay use. Each entry is a **full** snapshot of all
//! [`PARAM_COUNT`] parameters in the plugin's native JSON state format
//! (`{"params": {id: plain value, ...}}`).
//!
//! Full snapshots are not optional. The shared loader only writes the
//! ids it finds in the map, so a partial preset silently leaves the
//! previous patch's values in place — the bug the delay's presets
//! shipped with (finding P7). `tests/presets.rs` fails if any preset
//! here covers fewer than every declared id.
//!
//! ## Why the attack values look large and the release values small
//!
//! This crate's ballistics are the compressor's, and they are applied to
//! the *gain-reduction* envelope: reduction rising (the gate closing)
//! uses the `attack` coefficient, reduction falling (the gate opening)
//! uses `release`. So in this plugin, as it stands today, `attack` is
//! the closing ramp and `release` is the opening ramp — the opposite of
//! the usual gate convention, where attack is how fast the gate opens.
//! Measured, not assumed: with `attack` 100 ms / `release` 1 ms the gate
//! opens on a step in 3.7 ms; swap them and it takes 418 ms.
//!
//! These presets are tuned to what the plugin actually does, so they
//! sound right in today's build: percussive patches carry a short
//! `release` (snap open) and a longer `attack` (controlled close).
//! **If the inversion is fixed (ba todo #1343), the `attack` and
//! `release` values in `presets/*.json` must be swapped in the same
//! commit** — otherwise every percussive preset here starts fading in
//! over tens of milliseconds.

use crate::params::{GateParams, PARAM_COUNT};

/// The factory-preset entry type is the shared one, so this crate's
/// `PRESETS` can be handed straight to `presets::PresetBank` (ba todo
/// #1358). The alias keeps the crate-local name every call site uses.
pub use resonance_plugin::presets::FactoryPreset as PresetEntry;

/// The factory bank, in menu order: a reset, six self-keyed patches
/// spanning hard gate → gentle expander → partial duck, and two that
/// only make sense with something connected to the key port.
pub const PRESETS: &[PresetEntry] = &[
    PresetEntry {
        name: "Init — Default",
        json: include_str!("../presets/init_default.json"),
    },
    PresetEntry {
        name: "Vocal — Noise Gate",
        json: include_str!("../presets/vocal_noise_gate.json"),
    },
    PresetEntry {
        name: "Drums — Snare Gate",
        json: include_str!("../presets/snare_gate.json"),
    },
    PresetEntry {
        name: "Drums — Tom Gate",
        json: include_str!("../presets/tom_gate.json"),
    },
    PresetEntry {
        name: "Guitar — Noise Floor",
        json: include_str!("../presets/guitar_noise_floor.json"),
    },
    PresetEntry {
        name: "Gentle Expander",
        json: include_str!("../presets/gentle_expander.json"),
    },
    PresetEntry {
        name: "Dialogue — Room Tone",
        json: include_str!("../presets/dialogue_room_tone.json"),
    },
    PresetEntry {
        name: "Keyed — Open on Kick",
        json: include_str!("../presets/keyed_open_on_kick.json"),
    },
    PresetEntry {
        name: "Keyed — Trance Gate",
        json: include_str!("../presets/trance_gate_keyed.json"),
    },
];

/// Load one preset JSON blob onto the param surface through the shared
/// loader (`resonance_plugin::presets::load`). Returns `false` when the
/// blob does not parse or carries no `"params"` object.
pub fn load_preset(params: &GateParams, json: &str) -> bool {
    resonance_plugin::presets::load(json, PARAM_COUNT, |i| params.param_at(i))
}
