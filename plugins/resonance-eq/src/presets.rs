//! Factory presets baked into the binary via `include_str!`.
//!
//! Each entry is a full parameter snapshot in the same JSON format the
//! plugin writes natively, so the editor can load one by walking the
//! param list and calling `set_plain` for each matching id.

pub struct PresetEntry {
    pub name: &'static str,
    pub json: &'static str,
}

pub const PRESETS: &[PresetEntry] = &[
    PresetEntry { name: "Kick — Punch",    json: include_str!("../presets/kick_punch.json") },
    PresetEntry { name: "Kick — Sub",      json: include_str!("../presets/kick_sub.json") },
    PresetEntry { name: "Snare — Crack",   json: include_str!("../presets/snare_crack.json") },
    PresetEntry { name: "Snare — Body",    json: include_str!("../presets/snare_body.json") },
    PresetEntry { name: "Bass — Tight",    json: include_str!("../presets/bass_tight.json") },
    PresetEntry { name: "Bass — Warm",     json: include_str!("../presets/bass_warm.json") },
    PresetEntry { name: "Guitar — Body",   json: include_str!("../presets/guitar_body.json") },
    PresetEntry { name: "Guitar — Air",    json: include_str!("../presets/guitar_air.json") },
    PresetEntry { name: "Vocal — Clarity", json: include_str!("../presets/vocal_clarity.json") },
    PresetEntry { name: "Synth — Wide",    json: include_str!("../presets/synth_wide.json") },
    PresetEntry { name: "Master — Polish", json: include_str!("../presets/master_polish.json") },
];
