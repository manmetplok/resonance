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
        id: "tight-room",
        name: "Tight Room",
        json: include_str!("../presets/tight_room.json"),
    },
    PresetEntry {
        id: "vocal-plate",
        name: "Vocal Plate",
        json: include_str!("../presets/vocal_plate.json"),
    },
    PresetEntry {
        id: "warm-hall",
        name: "Warm Hall",
        json: include_str!("../presets/warm_hall.json"),
    },
    PresetEntry {
        id: "cathedral",
        name: "Cathedral",
        json: include_str!("../presets/cathedral.json"),
    },
    PresetEntry {
        id: "ambient-bloom",
        name: "Ambient Bloom",
        json: include_str!("../presets/ambient_bloom.json"),
    },
    PresetEntry {
        id: "shimmer-drone",
        name: "Shimmer Drone",
        json: include_str!("../presets/shimmer_drone.json"),
    },
    PresetEntry {
        id: "snare-plate",
        name: "Snare Plate",
        json: include_str!("../presets/snare_plate.json"),
    },
    PresetEntry {
        id: "snare-tight",
        name: "Snare Tight",
        json: include_str!("../presets/snare_tight.json"),
    },
    PresetEntry {
        id: "snare-ambient",
        name: "Snare Ambient",
        json: include_str!("../presets/snare_ambient.json"),
    },
    PresetEntry {
        id: "snare-gated",
        name: "Snare Gated",
        json: include_str!("../presets/snare_gated.json"),
    },
    PresetEntry {
        id: "bright-plate",
        name: "Bright Plate",
        json: include_str!("../presets/bright_plate.json"),
    },
    PresetEntry {
        id: "drum-room",
        name: "Drum Room",
        json: include_str!("../presets/drum_room.json"),
    },
    PresetEntry {
        id: "vocal-chamber",
        name: "Vocal Chamber",
        json: include_str!("../presets/vocal_chamber.json"),
    },
    PresetEntry {
        id: "string-hall",
        name: "String Hall",
        json: include_str!("../presets/string_hall.json"),
    },
    PresetEntry {
        id: "mix-glue",
        name: "Mix Glue",
        json: include_str!("../presets/mix_glue.json"),
    },
    PresetEntry {
        id: "short-ambience",
        name: "Short Ambience",
        json: include_str!("../presets/short_ambience.json"),
    },
    PresetEntry {
        id: "surf-spring",
        name: "Surf Spring",
        json: include_str!("../presets/surf_spring.json"),
    },
    PresetEntry {
        id: "eighties-gate",
        name: "80s Gate",
        json: include_str!("../presets/eighties_gate.json"),
    },
    PresetEntry {
        id: "octave-halo",
        name: "Octave Halo",
        json: include_str!("../presets/octave_halo.json"),
    },
    PresetEntry {
        id: "fifth-bloom",
        name: "Fifth Bloom",
        json: include_str!("../presets/fifth_bloom.json"),
    },
];
