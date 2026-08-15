//! Factory presets baked into the binary via `include_str!` (ba todo
//! #1137, design doc #264 req-6) — the same mechanism as
//! `plugins/resonance-compressor/src/presets.rs`. Each entry is a full
//! 29-parameter snapshot in the plugin's native JSON state format
//! (`{"params": {id: plain value, ...}}`), so loading is deterministic:
//! the loader walks the whole param surface and calls `set_plain`.
//!
//! Preset names follow the demo states of the approved editor design
//! (ba design doc #264, epic #199), plus a reverse-texture patch. The
//! HTML prototype that design was signed off from was never merged —
//! it only exists on the unmerged `ba/design-199` branch — so doc #264
//! in ba is the reference to read, not a path in this tree
//! (ba todo #1268).

use crate::params::{GranularDelayParams, PARAM_COUNT};

pub struct PresetEntry {
    pub name: &'static str,
    pub json: &'static str,
}

/// The factory set, in browsing order. Between them the eight presets
/// use every shipped mode of every mode parameter at least once, and
/// switch every capability toggle on somewhere — the audit's LOW
/// finding was that five modes were reachable but never demonstrated
/// (ba todo #1339, guarded by `tests/presets.rs`). Each name says what
/// its preset is there to show.
pub const PRESETS: &[PresetEntry] = &[
    // Per-Grain time, Wet→Buffer, Async, no quantize, LP, Normal — the
    // neutral starting point every other preset departs from.
    PresetEntry {
        name: "Init — Per-Grain Cloud",
        json: include_str!("../presets/init_per_grain_cloud.json"),
    },
    // Fade time mode, Sync scheduler, and the tempo-locked grain rate
    // (ba todo #1322): a 1/8T tap granulated in eighth triplets.
    PresetEntry {
        name: "Eighth-Triplet Echo — Tempo-Locked Grains",
        json: include_str!("../presets/eighth_triplet_echo.json"),
    },
    // Scale-quantized per-grain transpose + shimmer feedback, smeared
    // by the diffusion stage (ba todo #1321).
    PresetEntry {
        name: "D-Minor Shimmer — Scale Quantize + Diffusion",
        json: include_str!("../presets/d_minor_shimmer_cloud.json"),
    },
    // Pitch-Sync (PSOLA) scheduler, semitone quantize, and the HQ
    // quality tier (B-spline reads + anti-alias).
    PresetEntry {
        name: "Vocal Doubler — PSOLA + HQ",
        json: include_str!("../presets/vocal_doubler_psola.json"),
    },
    // Ping-pong feedback and reversed grains, with highpass damping in
    // the loop so the repeats thin out instead of darkening.
    PresetEntry {
        name: "Reverse Haze — Ping-Pong + HP Damp",
        json: include_str!("../presets/reverse_haze.json"),
    },
    // Freeze held on the Lo-fi tier (µ-law reads, reduced grain pool).
    PresetEntry {
        name: "Frozen Drone — Lo-Fi Freeze",
        json: include_str!("../presets/frozen_drone.json"),
    },
    // Repitch time mode (the tape-style swoop) on the Output-only
    // route, where repeats stay clean instead of re-granulating.
    PresetEntry {
        name: "Tape Warble — Repitch + Clean Repeats",
        json: include_str!("../presets/tape_warble_repitch.json"),
    },
    // WSOLA onset alignment (ba todo #1320) on a spray-heavy cloud —
    // the case it exists for: sprayed onsets snapped back into phase
    // with the sounding material.
    PresetEntry {
        name: "Aligned Cloud — WSOLA Onsets",
        json: include_str!("../presets/aligned_cloud_wsola.json"),
    },
];

/// Load one preset JSON blob onto the param surface through the shared
/// loader (`resonance_plugin::presets::load`), mirroring the
/// compressor editor's `load_preset`. Returns `false` on parse failure.
pub fn load_preset(params: &GranularDelayParams, json: &str) -> bool {
    resonance_plugin::presets::load(json, PARAM_COUNT, |i| params.param_at(i))
}
