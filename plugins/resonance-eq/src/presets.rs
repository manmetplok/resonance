//! Factory presets baked into the binary via `include_str!`.
//!
//! Each entry is a full parameter snapshot in the same JSON format the
//! plugin writes natively, so the editor can load one by walking the
//! param list and calling `set_plain` for each matching id.

/// The factory-preset entry type is the shared one, so this crate's
/// `PRESETS` can be handed straight to `presets::PresetBank` (ba todo
/// #1358). The alias keeps the crate-local name every call site uses.
pub use resonance_plugin::presets::FactoryPreset as PresetEntry;

pub const PRESETS: &[PresetEntry] = &[
    PresetEntry {
        id: "kick-punch",
        name: "Kick — Punch",
        json: include_str!("../presets/kick_punch.json"),
    },
    PresetEntry {
        id: "kick-sub",
        name: "Kick — Sub",
        json: include_str!("../presets/kick_sub.json"),
    },
    PresetEntry {
        id: "snare-crack",
        name: "Snare — Crack",
        json: include_str!("../presets/snare_crack.json"),
    },
    PresetEntry {
        id: "snare-body",
        name: "Snare — Body",
        json: include_str!("../presets/snare_body.json"),
    },
    PresetEntry {
        id: "bass-tight",
        name: "Bass — Tight",
        json: include_str!("../presets/bass_tight.json"),
    },
    PresetEntry {
        id: "bass-warm",
        name: "Bass — Warm",
        json: include_str!("../presets/bass_warm.json"),
    },
    PresetEntry {
        id: "guitar-body",
        name: "Guitar — Body",
        json: include_str!("../presets/guitar_body.json"),
    },
    PresetEntry {
        id: "guitar-air",
        name: "Guitar — Air",
        json: include_str!("../presets/guitar_air.json"),
    },
    PresetEntry {
        id: "vocal-clarity",
        name: "Vocal — Clarity",
        json: include_str!("../presets/vocal_clarity.json"),
    },
    PresetEntry {
        id: "synth-wide",
        name: "Synth — Wide",
        json: include_str!("../presets/synth_wide.json"),
    },
    PresetEntry {
        id: "master-polish",
        name: "Master — Polish",
        json: include_str!("../presets/master_polish.json"),
    },
];
