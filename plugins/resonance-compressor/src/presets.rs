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
        name: "Kick — Punch",
        json: include_str!("../presets/kick_punch.json"),
    },
    PresetEntry {
        name: "Snare — Slam",
        json: include_str!("../presets/snare_slam.json"),
    },
    PresetEntry {
        name: "Bass — Glue",
        json: include_str!("../presets/bass_glue.json"),
    },
    PresetEntry {
        name: "Vocal — Lead",
        json: include_str!("../presets/vocal_lead.json"),
    },
    PresetEntry {
        name: "Guitar — Control",
        json: include_str!("../presets/guitar_control.json"),
    },
    PresetEntry {
        name: "Drum Bus",
        json: include_str!("../presets/drum_bus.json"),
    },
    PresetEntry {
        name: "Mix Bus",
        json: include_str!("../presets/mix_bus.json"),
    },
    PresetEntry {
        name: "Master — Glue",
        json: include_str!("../presets/master_glue.json"),
    },
    PresetEntry {
        name: "Parallel Smash",
        json: include_str!("../presets/parallel_smash.json"),
    },
    PresetEntry {
        name: "Transparent",
        json: include_str!("../presets/transparent.json"),
    },
];
