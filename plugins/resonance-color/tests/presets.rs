//! Factory presets: exactly the five names warmth-width-depth.md §6.1
//! lets skills name, each a full snapshot of the parameter surface that
//! loads verbatim, and each voiced into its §2.1 THD band as the crate's
//! probe measures it (1 kHz sine at −18 dBFS peak).

use resonance_color::dsp::Settings;
use resonance_color::params::{ColorParams, PARAM_COUNT};
use resonance_color::presets::{PRESETS, PRESET_THD_BANDS};
use resonance_color::probe::{probe, PROBE_LEVEL_DBFS};
use resonance_color::ResonanceColor;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};

/// The names §6.1 lists, in its order. Skills name these strings, so a
/// rename is a skill-breaking change and has to fail here first.
const SPEC_NAMES: [&str; 5] = [
    "Bus — Warm Glue",
    "Bass — Iron",
    "Vocal — Tube Air",
    "Drums — Tape 15",
    "Master — Subtle Tape",
];

fn load(json: &str) -> ColorParams {
    let params = ColorParams::default();
    assert!(resonance_plugin::presets::load(json, PARAM_COUNT, |i| params.param_at(i)));
    params
}

#[test]
fn the_bank_is_exactly_the_spec_names() {
    let names: Vec<&str> = PRESETS.iter().map(|p| p.name).collect();
    assert_eq!(names, SPEC_NAMES);
    // The exported bank is the same list.
    assert_eq!(
        <ResonanceColor as ResonancePlugin>::FACTORY_PRESETS.len(),
        PRESETS.len()
    );
}

#[test]
fn every_preset_is_a_full_snapshot_that_loads_verbatim() {
    let fresh = ColorParams::default();
    for entry in PRESETS {
        let value: serde_json::Value = serde_json::from_str(&entry.state_json())
            .unwrap_or_else(|e| panic!("preset '{}' is invalid JSON: {e}", entry.name));
        let map = value["params"]
            .as_object()
            .unwrap_or_else(|| panic!("preset '{}' lacks a params object", entry.name));
        assert_eq!(map.len(), PARAM_COUNT, "preset '{}' must snapshot every param", entry.name);
        let params = load(entry.json);
        for i in 0..PARAM_COUNT {
            let id = fresh.param_at(i).id();
            let want = map
                .get(id)
                .and_then(|v| v.as_f64())
                .unwrap_or_else(|| panic!("preset '{}' is missing '{id}'", entry.name));
            let got = params.param_at(i).get_plain();
            assert!(
                (got - want).abs() < 1e-6,
                "preset '{}': '{id}' loaded as {got}, JSON says {want} (out of range?)",
                entry.name
            );
        }
        // Every factory preset keeps auto-gain on: the bank is what a
        // skill reaches for first, and a judgement it makes on a preset
        // must be at matched loudness.
        assert!(params.auto_gain.value(), "preset '{}' turns auto-gain off", entry.name);
    }
}

#[test]
fn every_preset_measures_inside_its_thd_band() {
    assert_eq!(PRESET_THD_BANDS.len(), PRESETS.len());
    for entry in PRESETS {
        let (_, (lo, hi)) = PRESET_THD_BANDS
            .iter()
            .find(|(name, _)| *name == entry.name)
            .unwrap_or_else(|| panic!("preset '{}' has no THD band", entry.name));
        let settings = Settings::from_params(&load(entry.json));
        let sig = probe(&settings, PROBE_LEVEL_DBFS);
        eprintln!(
            "{:22} THD {:6.3} %  H2 {:6.1}  H3 {:6.1}  H2-H3 {:+5.1} dB  band {lo}-{hi} %",
            entry.name,
            sig.thd_pct,
            sig.h_dbc[2],
            sig.h_dbc[3],
            sig.h2_h3_db()
        );
        assert!(
            sig.thd_pct >= *lo && sig.thd_pct <= *hi,
            "preset '{}' measures {:.3} % THD, outside its {lo}–{hi} % band",
            entry.name,
            sig.thd_pct
        );
    }
}

/// The two presets named for warmth are even-dominant (§2.1: H2 ≥ H3).
#[test]
fn the_warm_presets_are_even_dominant() {
    for name in ["Bus — Warm Glue", "Vocal — Tube Air"] {
        let entry = PRESETS.iter().find(|p| p.name == name).unwrap();
        let sig = probe(&Settings::from_params(&load(entry.json)), PROBE_LEVEL_DBFS);
        assert!(sig.h2_h3_db() > 0.0, "'{name}' is not even-dominant: H2−H3 {:.1} dB", sig.h2_h3_db());
    }
}

#[test]
fn every_preset_renders_audio_through_the_plugin() {
    for entry in PRESETS {
        let mut plugin = ResonanceColor::new();
        assert!(plugin.load_state(entry.json.as_bytes()), "'{}' did not load", entry.name);
        plugin.initialize(48_000.0, 512);
        let mut peak = 0.0f32;
        for block in 0..20 {
            let mut l = vec![0.0f32; 512];
            let mut r = vec![0.0f32; 512];
            for i in 0..512 {
                let t = (block * 512 + i) as f32 / 48_000.0;
                l[i] = 0.25 * (std::f32::consts::TAU * 220.0 * t).sin();
                r[i] = l[i];
            }
            let mut outs = [OutputBuffer {
                left: &mut l,
                right: &mut r,
            }];
            plugin.process(&mut outs, 512, &mut EventIterator::empty(), None);
            for &s in l.iter().chain(r.iter()) {
                assert!(s.is_finite(), "'{}' rendered a non-finite sample", entry.name);
                peak = peak.max(s.abs());
            }
        }
        assert!(peak > 0.05, "'{}' rendered (near) silence: peak {peak}", entry.name);
    }
}
