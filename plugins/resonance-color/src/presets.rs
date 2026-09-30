//! Factory presets, baked in via `include_str!`. Each is a **full**
//! snapshot of every declared parameter (the shared loader writes only
//! the ids it finds, so a partial preset would inherit the previous
//! patch's values).
//!
//! The names are the ones warmth-width-depth.md §6.1 lets skills name,
//! verbatim. Each preset is voiced to a §2.1 THD band, measured with the
//! crate's own probe (1 kHz sine at −18 dBFS peak, [`crate::probe`]):
//!
//! | Preset | Placement | THD band |
//! |---|---|---|
//! | Bus — Warm Glue | bus | 0.5–3 % |
//! | Bass — Iron | single track | 3–10 % |
//! | Vocal — Tube Air | single track | 3–10 % |
//! | Drums — Tape 15 | drum bus | 0.5–3 % |
//! | Master — Subtle Tape | master | 0.1–1 % |
//!
//! [`PRESET_THD_BANDS`] carries the same table for `tests/presets.rs`,
//! which fails if a preset drifts out of its band.

pub use resonance_plugin::presets::FactoryPreset as PresetEntry;

pub const PRESETS: &[PresetEntry] = &[
    PresetEntry {
        id: "bus-warm-glue",
        name: "Bus — Warm Glue",
        json: include_str!("../presets/bus_warm_glue.json"),
    },
    PresetEntry {
        id: "bass-iron",
        name: "Bass — Iron",
        json: include_str!("../presets/bass_iron.json"),
    },
    PresetEntry {
        id: "vocal-tube-air",
        name: "Vocal — Tube Air",
        json: include_str!("../presets/vocal_tube_air.json"),
    },
    PresetEntry {
        id: "drums-tape-15",
        name: "Drums — Tape 15",
        json: include_str!("../presets/drums_tape_15.json"),
    },
    PresetEntry {
        id: "master-subtle-tape",
        name: "Master — Subtle Tape",
        json: include_str!("../presets/master_subtle_tape.json"),
    },
];

/// §2.1's THD targets, in percent.
pub const THD_MASTER: (f64, f64) = (0.1, 1.0);
pub const THD_BUS: (f64, f64) = (0.5, 3.0);
pub const THD_TRACK: (f64, f64) = (3.0, 10.0);

/// Each factory preset's THD band, by name (the table above).
pub const PRESET_THD_BANDS: &[(&str, (f64, f64))] = &[
    ("Bus — Warm Glue", THD_BUS),
    ("Bass — Iron", THD_TRACK),
    ("Vocal — Tube Air", THD_TRACK),
    ("Drums — Tape 15", THD_BUS),
    ("Master — Subtle Tape", THD_MASTER),
];
