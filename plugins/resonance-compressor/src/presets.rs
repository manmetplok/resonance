//! Factory presets baked into the binary via `include_str!`. Each entry is
//! a full parameter snapshot in the same JSON format the plugin writes
//! natively, so loading a preset just walks the param list and calls
//! `set_plain`.

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
        id: "snare-slam",
        name: "Snare — Slam",
        json: include_str!("../presets/snare_slam.json"),
    },
    PresetEntry {
        id: "bass-glue",
        name: "Bass — Glue",
        json: include_str!("../presets/bass_glue.json"),
    },
    PresetEntry {
        id: "vocal-lead",
        name: "Vocal — Lead",
        json: include_str!("../presets/vocal_lead.json"),
    },
    PresetEntry {
        id: "guitar-control",
        name: "Guitar — Control",
        json: include_str!("../presets/guitar_control.json"),
    },
    PresetEntry {
        id: "drum-bus",
        name: "Drum Bus",
        json: include_str!("../presets/drum_bus.json"),
    },
    PresetEntry {
        id: "mix-bus",
        name: "Mix Bus",
        json: include_str!("../presets/mix_bus.json"),
    },
    PresetEntry {
        id: "master-glue",
        name: "Master — Glue",
        json: include_str!("../presets/master_glue.json"),
    },
    PresetEntry {
        id: "parallel-smash",
        name: "Parallel Smash",
        json: include_str!("../presets/parallel_smash.json"),
    },
    PresetEntry {
        id: "transparent",
        name: "Transparent",
        json: include_str!("../presets/transparent.json"),
    },
    // Bus glue by the book (warmth-width-depth.md §4 step 5): 2:1, a
    // 20 ms attack that lets transients through, and the program-
    // dependent Auto release, so a bus held in compression recovers
    // slowly while single hits recover fast.
    PresetEntry {
        id: "bus-auto-glue",
        name: "Bus — Auto Glue",
        json: include_str!("../presets/bus_auto_glue.json"),
    },
];
