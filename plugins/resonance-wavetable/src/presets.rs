//! Factory presets baked into the binary via `include_str!`. Each entry is
//! a full parameter snapshot in the same JSON format the plugin writes
//! natively, so loading a preset just walks the param list and calls
//! `set_plain` on each matching id.

pub struct PresetEntry {
    pub name: &'static str,
    pub json: &'static str,
}

pub const PRESETS: &[PresetEntry] = &[
    PresetEntry { name: "Init",                  json: include_str!("../presets/init.json") },
    PresetEntry { name: "Lead — Supersaw",       json: include_str!("../presets/lead_supersaw.json") },
    PresetEntry { name: "Lead — Analog Square",  json: include_str!("../presets/lead_analog_square.json") },
    PresetEntry { name: "Lead — Sync Screamer",  json: include_str!("../presets/lead_sync_screamer.json") },
    PresetEntry { name: "Bass — Reese",          json: include_str!("../presets/bass_reese.json") },
    PresetEntry { name: "Bass — Sub Round",      json: include_str!("../presets/bass_sub_round.json") },
    PresetEntry { name: "Bass — Acid Squelch",   json: include_str!("../presets/bass_acid_squelch.json") },
    PresetEntry { name: "Bass — Wobble",         json: include_str!("../presets/bass_wobble.json") },
    PresetEntry { name: "Pad — Warm Analog",     json: include_str!("../presets/pad_warm_analog.json") },
    PresetEntry { name: "Pad — Glass Shimmer",   json: include_str!("../presets/pad_glass_shimmer.json") },
    PresetEntry { name: "Pad — Evolving Choir",  json: include_str!("../presets/pad_evolving_choir.json") },
    PresetEntry { name: "Pluck — Digital Bell",  json: include_str!("../presets/pluck_digital_bell.json") },
    PresetEntry { name: "Pluck — Nylon Harp",    json: include_str!("../presets/pluck_nylon_harp.json") },
    PresetEntry { name: "Pluck — Stack",         json: include_str!("../presets/pluck_stack.json") },
    PresetEntry { name: "Keys — Electric Piano", json: include_str!("../presets/keys_electric_piano.json") },
    PresetEntry { name: "Keys — Cathedral Organ",json: include_str!("../presets/keys_cathedral_organ.json") },
    PresetEntry { name: "Arp — Formant Talker",  json: include_str!("../presets/arp_formant_talker.json") },
    PresetEntry { name: "Arp — Metallic Sequence",json: include_str!("../presets/arp_metallic_sequence.json") },
    PresetEntry { name: "FX — Risers",           json: include_str!("../presets/fx_risers.json") },
    PresetEntry { name: "FX — Noise Sweep",      json: include_str!("../presets/fx_noise_sweep.json") },
    PresetEntry { name: "FX — Drone Texture",    json: include_str!("../presets/fx_drone_texture.json") },
    PresetEntry { name: "Brass — Stab",          json: include_str!("../presets/brass_stab.json") },
    PresetEntry { name: "Strings — Ensemble",    json: include_str!("../presets/strings_ensemble.json") },
];
