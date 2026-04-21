//! Factory presets baked into the binary via `include_str!`. Each entry is
//! a full parameter snapshot in the same JSON format the plugin writes
//! natively, so loading a preset just walks the param list and calls
//! `set_plain`.

pub struct PresetEntry {
    pub name: &'static str,
    pub json: &'static str,
}

pub const PRESETS: &[PresetEntry] = &[
    PresetEntry {
        name: "Tight Room",
        json: include_str!("../presets/tight_room.json"),
    },
    PresetEntry {
        name: "Vocal Plate",
        json: include_str!("../presets/vocal_plate.json"),
    },
    PresetEntry {
        name: "Warm Hall",
        json: include_str!("../presets/warm_hall.json"),
    },
    PresetEntry {
        name: "Cathedral",
        json: include_str!("../presets/cathedral.json"),
    },
    PresetEntry {
        name: "Ambient Bloom",
        json: include_str!("../presets/ambient_bloom.json"),
    },
    PresetEntry {
        name: "Shimmer Drone",
        json: include_str!("../presets/shimmer_drone.json"),
    },
];
