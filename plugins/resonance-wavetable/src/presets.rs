//! Factory presets baked into the binary via `include_str!`. Each entry is
//! a full parameter snapshot in the same JSON format the plugin writes
//! natively, so loading a preset just walks the param list and calls
//! `set_plain` on each matching id.

/// The factory-preset entry type is the shared one, so this crate's
/// `PRESETS` can be handed straight to `presets::PresetBank` (ba todo
/// #1358). The alias keeps the crate-local name every call site uses.
pub use resonance_plugin::presets::FactoryPreset as PresetEntry;

pub const PRESETS: &[PresetEntry] = &[
    PresetEntry {
        id: "init",
        name: "Init",
        json: include_str!("../presets/init.json"),
    },
    PresetEntry {
        id: "lead-supersaw",
        name: "Lead — Supersaw",
        json: include_str!("../presets/lead_supersaw.json"),
    },
    PresetEntry {
        id: "lead-analog-square",
        name: "Lead — Analog Square",
        json: include_str!("../presets/lead_analog_square.json"),
    },
    PresetEntry {
        id: "lead-sync-screamer",
        name: "Lead — Sync Screamer",
        json: include_str!("../presets/lead_sync_screamer.json"),
    },
    PresetEntry {
        id: "lead-hard-sync-sweep",
        name: "Lead — Hard Sync Sweep",
        json: include_str!("../presets/lead_hard_sync_sweep.json"),
    },
    PresetEntry {
        id: "bass-reese",
        name: "Bass — Reese",
        json: include_str!("../presets/bass_reese.json"),
    },
    PresetEntry {
        id: "bass-sub-round",
        name: "Bass — Sub Round",
        json: include_str!("../presets/bass_sub_round.json"),
    },
    PresetEntry {
        id: "bass-acid-squelch",
        name: "Bass — Acid Squelch",
        json: include_str!("../presets/bass_acid_squelch.json"),
    },
    PresetEntry {
        id: "bass-ladder-fm-growl",
        name: "Bass — Ladder FM Growl",
        json: include_str!("../presets/bass_ladder_fm_growl.json"),
    },
    PresetEntry {
        id: "bass-wobble",
        name: "Bass — Wobble",
        json: include_str!("../presets/bass_wobble.json"),
    },
    PresetEntry {
        id: "pad-warm-analog",
        name: "Pad — Warm Analog",
        json: include_str!("../presets/pad_warm_analog.json"),
    },
    PresetEntry {
        id: "pad-juno-chorus",
        name: "Pad — Juno Chorus",
        json: include_str!("../presets/pad_juno_chorus.json"),
    },
    PresetEntry {
        id: "pad-glass-shimmer",
        name: "Pad — Glass Shimmer",
        json: include_str!("../presets/pad_glass_shimmer.json"),
    },
    PresetEntry {
        id: "pad-evolving-choir",
        name: "Pad — Evolving Choir",
        json: include_str!("../presets/pad_evolving_choir.json"),
    },
    PresetEntry {
        id: "pluck-digital-bell",
        name: "Pluck — Digital Bell",
        json: include_str!("../presets/pluck_digital_bell.json"),
    },
    PresetEntry {
        id: "pluck-nylon-harp",
        name: "Pluck — Nylon Harp",
        json: include_str!("../presets/pluck_nylon_harp.json"),
    },
    PresetEntry {
        id: "pluck-stack",
        name: "Pluck — Stack",
        json: include_str!("../presets/pluck_stack.json"),
    },
    PresetEntry {
        id: "keys-electric-piano",
        name: "Keys — Electric Piano",
        json: include_str!("../presets/keys_electric_piano.json"),
    },
    PresetEntry {
        // Showcases the drive stages: per-voice pre-filter drive (pushed
        // harder by velocity through the mod matrix — slot 1 → Voice Drive)
        // into a 2x-oversampled tube stage, tone rolled off, auto gain on.
        id: "keys-driven-chords",
        name: "Keys — Driven Chords",
        json: include_str!("../presets/keys_driven_chords.json"),
    },
    PresetEntry {
        id: "keys-cathedral-organ",
        name: "Keys — Cathedral Organ",
        json: include_str!("../presets/keys_cathedral_organ.json"),
    },
    PresetEntry {
        id: "keys-vintage-poly",
        name: "Keys — Vintage Poly",
        json: include_str!("../presets/keys_vintage_poly.json"),
    },
    PresetEntry {
        id: "arp-formant-talker",
        name: "Arp — Formant Talker",
        json: include_str!("../presets/arp_formant_talker.json"),
    },
    PresetEntry {
        id: "arp-metallic-sequence",
        name: "Arp — Metallic Sequence",
        json: include_str!("../presets/arp_metallic_sequence.json"),
    },
    PresetEntry {
        id: "fx-risers",
        name: "FX — Risers",
        json: include_str!("../presets/fx_risers.json"),
    },
    PresetEntry {
        id: "fx-noise-sweep",
        name: "FX — Noise Sweep",
        json: include_str!("../presets/fx_noise_sweep.json"),
    },
    PresetEntry {
        id: "fx-drone-texture",
        name: "FX — Drone Texture",
        json: include_str!("../presets/fx_drone_texture.json"),
    },
    PresetEntry {
        id: "brass-stab",
        name: "Brass — Stab",
        json: include_str!("../presets/brass_stab.json"),
    },
    PresetEntry {
        id: "strings-ensemble",
        name: "Strings — Ensemble",
        json: include_str!("../presets/strings_ensemble.json"),
    },
];
