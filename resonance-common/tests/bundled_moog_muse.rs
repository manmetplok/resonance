//! The bundled Moog Muse device definition (todo #721, doc #201 §3, built from
//! the R1 MIDI chart in doc #202): it ships in the registry's bundled set, loads,
//! passes validation, and serde round-trips.

use resonance_common::device_definition::MidiBinding;
use resonance_common::{
    bundled_definitions, DeviceDefinition, DeviceDefinitionRegistry, ParamCurve,
};

/// Fetch the `moog-muse` definition from the embedded bundled set.
fn moog_muse() -> DeviceDefinition {
    bundled_definitions()
        .into_iter()
        .find(|d| d.id == "moog-muse")
        .expect("moog-muse must be in the bundled set")
}

#[test]
fn moog_muse_is_in_the_bundled_set() {
    let def = moog_muse();
    assert_eq!(def.manufacturer, "Moog");
    assert_eq!(def.model, "Muse");
    // The registry can be seeded from the embedded bundled set without any
    // on-disk bundled directory.
    let mut reg = DeviceDefinitionRegistry::default();
    reg.scan_bundled();
    assert!(reg.errors().is_empty(), "bundled set must load clean");
    assert_eq!(reg.get("moog-muse"), Some(&def));
}

#[test]
fn bundled_moog_muse_validates() {
    moog_muse()
        .validate()
        .expect("bundled moog-muse must pass validation");
}

#[test]
fn bundled_moog_muse_round_trips_through_json() {
    let def = moog_muse();
    let json = def.to_json().expect("serialize");
    let back = DeviceDefinition::from_json(json.as_bytes()).expect("deserialize");
    assert_eq!(def, back, "moog-muse must survive a JSON round-trip");
}

#[test]
fn every_param_is_a_documented_cc() {
    // R1 (doc #202): the Muse exposes its parameters over 7-bit CC only — no
    // NRPN, no 14-bit — each `min: 0, max: 127, Linear`. Guard that the content
    // never drifts from the chart.
    let def = moog_muse();
    assert!(!def.params.is_empty());
    for p in &def.params {
        match p.binding {
            MidiBinding::Cc { cc } => assert!(cc <= 127, "CC out of range for {}", p.id),
            other => panic!("param {} must be a CC, got {other:?}", p.id),
        }
        assert_eq!((p.min, p.max), (0, 127), "param {} range", p.id);
        assert_eq!(p.curve, ParamCurve::Linear, "param {} curve", p.id);
    }
}

#[test]
fn documented_performance_controls_are_present() {
    // Spot-check the headline controls the DoD calls out, with their R1 CC#s.
    let def = moog_muse();
    let cc_of = |id: &str| match def.param(id).unwrap_or_else(|| panic!("missing {id}")).binding {
        MidiBinding::Cc { cc } => cc,
        other => panic!("{id} bound to {other:?}"),
    };
    assert_eq!(cc_of("filter1-cutoff"), 67);
    assert_eq!(cc_of("filter1-resonance"), 68);
    assert_eq!(cc_of("filter-env-attack"), 79);
    assert_eq!(cc_of("filter-env-release"), 82);
    assert_eq!(cc_of("vca-env-attack"), 86);
    assert_eq!(cc_of("vca-env-release"), 89);
    assert_eq!(cc_of("lfo1-rate"), 12);
    assert_eq!(cc_of("lfo1-amount"), 13);
    assert_eq!(cc_of("osc1-level"), 58);
    assert_eq!(cc_of("osc2-level"), 59);
    assert_eq!(cc_of("glide-time"), 5);
}

#[test]
fn ships_the_full_bank_program_grid() {
    // R1 §2: 16 banks x 16 patches = 256 coordinate slots, Bank Select MSB always
    // 0, bank/program raw bytes 0-indexed (0..=15).
    let def = moog_muse();
    assert_eq!(def.patches.len(), 256);
    assert!(def.patches.iter().all(|p| p.bank_msb == 0));
    assert!(def.patches.iter().all(|p| p.bank_lsb <= 15 && p.program <= 15));
    let first = &def.patches[0];
    assert_eq!((first.bank_lsb, first.program), (0, 0));
    let last = &def.patches[255];
    assert_eq!((last.bank_lsb, last.program), (15, 15));
    // Coordinate names only — factory patch names are undocumented (R1 §4).
    assert_eq!(first.name, "Bank 1 · Patch 1");
}
