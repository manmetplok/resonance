//! The declared surface: the §6.1 parameter set, its defaults and
//! choice tables, and the plugin's identity.

use resonance_color::params::{
    ColorParams, Mode, TapeQuality, PARAM_COUNT, SPEED_IPS, SPEED_LABELS,
};
use resonance_color::ResonanceColor;
use resonance_dsp::{HysteresisSolver, OversampleFactor};
use resonance_plugin::{Param, ResonancePlugin};

#[test]
fn identity() {
    assert_eq!(ResonanceColor::CLAP_ID, "com.resonance.color");
    assert_eq!(ResonanceColor::NAME, "Resonance Color");
    assert_eq!(ResonanceColor::INPUT_CHANNELS, Some(2));
    assert_eq!(ResonanceColor::SIDECHAIN_INPUT, None);
}

#[test]
fn the_param_ids_are_the_spec_names_and_unique() {
    let plugin = ResonanceColor::new();
    assert_eq!(plugin.param_count(), PARAM_COUNT);
    let ids: Vec<&str> = (0..PARAM_COUNT).map(|i| plugin.param(i).id()).collect();
    assert_eq!(
        ids,
        [
            "mode",
            "drive",
            "bias",
            "response",
            "tone",
            "mix",
            "auto_gain",
            "output",
            "oversample",
            "speed",
            "flutter",
            "tape_quality",
            "tape_solver"
        ]
    );
    let mut unique = ids.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), ids.len());
}


#[test]
fn defaults() {
    let p = ColorParams::default();
    assert_eq!(p.mode(), Mode::Tube);
    assert!(p.auto_gain.value(), "auto-gain is on by default (§6.1)");
    assert_eq!(p.mix.value(), 1.0);
    assert_eq!(p.output.value(), 0.0);
    assert_eq!(p.response.value(), 0.0);
    assert_eq!(p.tone.value(), 0.0);
    assert_eq!(p.flutter.value(), 0.0, "flutter defaults to 0 = bypass (§6.1)");
    assert_eq!(p.oversample_factor(), OversampleFactor::X2);
    assert_eq!(SPEED_IPS[p.speed.value() as usize], 15.0);
}

#[test]
fn choice_tables_read_as_names_on_every_surface() {
    let p = ColorParams::default();
    for (i, label) in Mode::LABELS.iter().enumerate() {
        assert_eq!(p.mode.display(i as f64), *label);
        assert_eq!(p.mode.parse(label), Some(i as f64));
    }
    for (i, label) in OversampleFactor::LABELS.iter().enumerate() {
        assert_eq!(p.oversample.display(i as f64), *label);
    }
    for (i, label) in SPEED_LABELS.iter().enumerate() {
        assert_eq!(p.speed.display(i as f64), *label);
        assert_eq!(p.speed.parse(label), Some(i as f64));
    }
    assert_eq!(Mode::LABELS.len(), Mode::ALL.len());
    for (i, m) in Mode::ALL.iter().enumerate() {
        assert_eq!(*m as usize, i);
        assert_eq!(Mode::from_int(i as i32), *m);
    }
}

#[test]
fn percent_params_parse_what_they_display() {
    let p = ColorParams::default();
    for param in [&p.drive, &p.bias, &p.mix, &p.flutter] {
        let shown = param.display(0.4);
        assert_eq!(shown, "40%");
        let back = param.parse(&shown).unwrap();
        assert!((back - 0.4).abs() < 1e-6, "{} parsed {shown} as {back}", param.id());
    }
    assert_eq!(p.response.display(-3.0), "-3.0 dB");
    assert_eq!(p.tone.display(0.0), "0.0 dB");
    assert_eq!(p.response.parse("-3.0 dB"), Some(-3.0));
}

#[test]
fn the_tape_only_params_are_grouped_for_hosts() {
    let p = ColorParams::default();
    assert_eq!(p.speed.module(), "Tape");
    assert_eq!(p.flutter.module(), "Tape");
    assert_eq!(p.tape_quality.module(), "Tape");
    assert_eq!(p.tape_solver.module(), "Tape");
}

/// W6b's two params: appended after the W6 set (host indices of the
/// first eleven unchanged), Standard at 0 and the default, so state
/// saved before they existed loads as Standard (`tests/legacy_state.rs`
/// pins the render), and RK4 as the default solver.
#[test]
fn tape_quality_is_appended_with_standard_at_index_0() {
    let plugin = ResonanceColor::new();
    assert_eq!(plugin.param(11).id(), "tape_quality");
    assert_eq!(plugin.param(12).id(), "tape_solver");
    let p = ColorParams::default();
    assert_eq!(p.tape_quality(), TapeQuality::Standard);
    assert_eq!(TapeQuality::Standard as i32, 0);
    assert_eq!(p.tape_quality.default_plain(), 0.0);
    assert_eq!(p.tape_solver(), HysteresisSolver::Rk4);
    for (i, label) in TapeQuality::LABELS.iter().enumerate() {
        assert_eq!(p.tape_quality.display(i as f64), *label);
        assert_eq!(p.tape_quality.parse(label), Some(i as f64));
    }
    for (i, label) in HysteresisSolver::LABELS.iter().enumerate() {
        assert_eq!(p.tape_solver.display(i as f64), *label);
        assert_eq!(HysteresisSolver::ALL[i] as usize, i);
        assert_eq!(HysteresisSolver::from_int(i as i32), HysteresisSolver::ALL[i]);
    }
    // A blob with no `tape_quality` key (every W6 project) loads into a
    // fresh instance as Standard.
    let mut plugin = ResonanceColor::new();
    assert!(plugin.load_state(br#"{"params":{"mode":1,"drive":0.5}}"#));
    assert_eq!(plugin.params.tape_quality(), TapeQuality::Standard);
}
