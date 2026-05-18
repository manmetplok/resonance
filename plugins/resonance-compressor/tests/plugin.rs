use resonance_compressor::params::PARAM_COUNT;
use resonance_compressor::presets::PRESETS;
use resonance_compressor::ResonanceCompressor;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};

#[test]
fn param_enumeration_covers_declared_count() {
    let plugin = ResonanceCompressor::new();
    assert_eq!(plugin.param_count(), PARAM_COUNT);
    let mut seen = std::collections::HashSet::new();
    for i in 0..plugin.param_count() {
        let id = plugin.param(i).id().to_string();
        assert!(seen.insert(id.clone()), "duplicate param id: {id}");
    }
}

#[test]
fn every_factory_preset_parses_and_loads() {
    assert!(!PRESETS.is_empty());
    for entry in PRESETS {
        let mut plugin = ResonanceCompressor::new();
        assert!(
            plugin.load_state(entry.json.as_bytes()),
            "preset {:?} failed to load",
            entry.name
        );
    }
}

#[test]
fn state_round_trips_through_save_load() {
    let plugin = ResonanceCompressor::new();
    plugin.params.threshold.set_value(-22.5);
    plugin.params.ratio.set_value(6.0);
    plugin.params.attack.set_value(5.0);
    plugin.params.release.set_value(180.0);
    plugin.params.knee.set_value(8.0);
    plugin.params.mix.set_value(0.75);
    plugin.params.auto_makeup.set_value(true);

    let saved = plugin.save_state();
    let mut other = ResonanceCompressor::new();
    assert!(other.load_state(&saved));

    assert!((plugin.params.threshold.value() - other.params.threshold.value()).abs() < 1e-3);
    assert!((plugin.params.ratio.value() - other.params.ratio.value()).abs() < 1e-3);
    assert!((plugin.params.attack.value() - other.params.attack.value()).abs() < 1e-3);
    assert!((plugin.params.release.value() - other.params.release.value()).abs() < 1e-3);
    assert!((plugin.params.knee.value() - other.params.knee.value()).abs() < 1e-3);
    assert!((plugin.params.mix.value() - other.params.mix.value()).abs() < 1e-3);
    assert_eq!(
        plugin.params.auto_makeup.value(),
        other.params.auto_makeup.value()
    );
}

#[test]
fn dsp_processes_loud_signal_without_nans() {
    let mut plugin = ResonanceCompressor::new();
    plugin.initialize(48_000.0, 4096);
    plugin.params.threshold.set_value(-20.0);
    plugin.params.ratio.set_value(8.0);
    plugin.params.attack.set_value(3.0);
    plugin.params.release.set_value(80.0);
    plugin.params.knee.set_value(6.0);
    plugin.params.sc_hpf_on.set_value(true);
    // Disable makeup entirely so the check is unambiguous — any
    // output peak below the input peak is pure gain reduction.
    plugin.params.makeup.set_value(0.0);
    plugin.params.auto_makeup.set_value(false);

    // Four blocks (~85 ms) is more than enough for the GR envelope to
    // fully converge with a 3 ms attack and 80 ms release.
    let frames = 4096usize;
    let mut left = vec![0.0_f32; frames];
    let mut right = vec![0.0_f32; frames];
    for i in 0..frames {
        let t = i as f32 / 48_000.0;
        let s = (t * 440.0 * std::f32::consts::TAU).sin() * 0.8;
        left[i] = s;
        right[i] = s;
    }
    let mut outs = [OutputBuffer {
        left: &mut left,
        right: &mut right,
    }];
    let mut ev = EventIterator::empty();
    plugin.process(&mut outs, frames, &mut ev, None);

    for &x in left.iter().chain(right.iter()) {
        assert!(x.is_finite(), "non-finite sample: {x}");
    }

    // Measure the settled tail of the block, not the attack ramp.
    let tail_start = frames * 3 / 4;
    let peak_tail = left[tail_start..]
        .iter()
        .copied()
        .map(f32::abs)
        .fold(0.0_f32, f32::max);
    assert!(
        peak_tail < 0.8,
        "expected gain reduction in the settled tail, got peak {peak_tail}"
    );
}
