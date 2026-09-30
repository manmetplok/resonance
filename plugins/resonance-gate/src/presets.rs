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
//! ## Attack opens, release closes (ba todo #1343)
//!
//! These values read the conventional way round: `attack` is how fast
//! the gate OPENS, `release` how fast it closes. Percussive patches
//! therefore carry a short attack and a longer release.
//!
//! That was not always true. The ballistics are the compressor's and run
//! on the *gain-reduction* envelope, where reduction RISES as the gate
//! closes — so used as named they made `attack` the closing ramp and
//! `release` the opening one, inverting both controls. This bank was
//! originally tuned against that inverted behaviour, and #1343 swapped
//! the DSP and these JSONs together in one commit. If you are comparing
//! against a build from before that, the numbers here will look
//! reversed; they are not.
//!
//! Two of the nine did not simply swap:
//!
//! * **Init — Default** was left alone. It mirrors the declared param
//!   defaults (attack 1 ms, release 100 ms), which already read
//!   conventionally — the inversion is what made the default patch fade
//!   in over ~0.4 s, and fixing the DSP is what makes it snap open as it
//!   always claimed to. Swapping it would have re-broken it.
//! * **Trance Gate (Keyed)** wanted a 3 ms close, which was reachable
//!   only because the value used to travel through the `attack` param
//!   (floor 0.05 ms). The `release` param's floor is 5 ms, so its close
//!   ramp is now 5 ms. Inaudible on a keyed stutter, but it is a real
//!   2 ms deviation from the original patch rather than a pure swap.

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
        id: "init-default",
        name: "Init — Default",
        json: include_str!("../presets/init_default.json"),
    },
    PresetEntry {
        id: "vocal-noise-gate",
        name: "Vocal — Noise Gate",
        json: include_str!("../presets/vocal_noise_gate.json"),
    },
    PresetEntry {
        id: "drums-snare-gate",
        name: "Drums — Snare Gate",
        json: include_str!("../presets/snare_gate.json"),
    },
    PresetEntry {
        id: "drums-tom-gate",
        name: "Drums — Tom Gate",
        json: include_str!("../presets/tom_gate.json"),
    },
    PresetEntry {
        id: "guitar-noise-floor",
        name: "Guitar — Noise Floor",
        json: include_str!("../presets/guitar_noise_floor.json"),
    },
    PresetEntry {
        id: "gentle-expander",
        name: "Gentle Expander",
        json: include_str!("../presets/gentle_expander.json"),
    },
    PresetEntry {
        id: "dialogue-room-tone",
        name: "Dialogue — Room Tone",
        json: include_str!("../presets/dialogue_room_tone.json"),
    },
    PresetEntry {
        id: "keyed-open-on-kick",
        name: "Keyed — Open on Kick",
        json: include_str!("../presets/keyed_open_on_kick.json"),
    },
    PresetEntry {
        id: "keyed-trance-gate",
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
