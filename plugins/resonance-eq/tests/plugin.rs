use resonance_eq::params::PARAM_COUNT;
use resonance_eq::presets::PRESETS;
use resonance_eq::ResonanceEq;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};

#[test]
fn param_enumeration_covers_declared_count() {
    let plugin = ResonanceEq::new();
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
        let mut plugin = ResonanceEq::new();
        assert!(
            plugin.load_state(entry.json.as_bytes()),
            "preset {:?} failed to load",
            entry.name
        );
    }
}

#[test]
fn state_round_trips_through_save_load() {
    let plugin = ResonanceEq::new();
    // Mutate a handful of params to non-default values.
    plugin.params.bands[2].enabled.set_value(true);
    plugin.params.bands[2].freq.set_value(1234.0);
    plugin.params.bands[2].gain.set_value(-7.5);
    plugin.params.bands[2].q.set_value(2.5);
    plugin.params.bands[2].kind.set_value(0);
    plugin.params.output_gain.set_value(3.25);

    let saved = plugin.save_state();

    let mut other = ResonanceEq::new();
    assert!(other.load_state(&saved));

    let a = &plugin.params.bands[2];
    let b = &other.params.bands[2];
    assert_eq!(a.enabled.value(), b.enabled.value());
    assert!((a.freq.value() - b.freq.value()).abs() < 1e-3);
    assert!((a.gain.value() - b.gain.value()).abs() < 1e-3);
    assert!((a.q.value() - b.q.value()).abs() < 1e-3);
    assert_eq!(a.kind.value(), b.kind.value());
    assert!(
        (plugin.params.output_gain.value() - other.params.output_gain.value()).abs() < 1e-3
    );
}

#[test]
fn dsp_processes_without_nans() {
    let mut plugin = ResonanceEq::new();
    plugin.initialize(48_000.0, 512);
    // Enable a couple of bands and set extreme settings.
    plugin.params.bands[0].enabled.set_value(true);
    plugin.params.bands[0].kind.set_value(3); // low cut
    plugin.params.bands[0].freq.set_value(60.0);
    plugin.params.bands[0].slope.set_value(2); // 48 dB/oct
    plugin.params.bands[3].enabled.set_value(true);
    plugin.params.bands[3].kind.set_value(0); // bell
    plugin.params.bands[3].freq.set_value(2_000.0);
    plugin.params.bands[3].gain.set_value(12.0);
    plugin.params.bands[3].q.set_value(4.0);

    let mut left = vec![0.5_f32; 256];
    let mut right = vec![-0.3_f32; 256];
    let mut outs = [OutputBuffer {
        left: &mut left,
        right: &mut right,
    }];
    let mut ev = EventIterator::empty();
    plugin.process(&mut outs, 256, &mut ev, None);

    for &x in left.iter().chain(right.iter()) {
        assert!(x.is_finite(), "output contains non-finite value: {x}");
    }
}

// ---------------------------------------------------------------------------
// LIB-05: stages that leave and re-enter the active set start clean
// ---------------------------------------------------------------------------

/// Deterministic loud noise, block `n` of it.
fn loud_noise(n: u64, frames: usize) -> Vec<f32> {
    let mut s = 0x9E37_79B9_7F4A_7C15u64 ^ n.wrapping_mul(0x2545_F491_4F6C_DD1D);
    (0..frames)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            ((s >> 40) as f32 / (1u64 << 24) as f32) * 1.8 - 0.9
        })
        .collect()
}

fn run(plugin: &mut ResonanceEq, input: &[f32]) -> Vec<f32> {
    let mut left = input.to_vec();
    let mut right = input.to_vec();
    let frames = left.len();
    let mut outs = [OutputBuffer {
        left: &mut left,
        right: &mut right,
    }];
    let mut ev = EventIterator::empty();
    plugin.process(&mut outs, frames, &mut ev, None);
    left
}

fn low_cut_48(plugin: &ResonanceEq) {
    let b = &plugin.params.bands[0];
    b.enabled.set_value(true);
    b.kind.set_value(3); // low cut
    b.freq.set_value(120.0);
    b.slope.set_value(2); // 48 dB/oct
}

#[test]
fn re_enabling_a_band_does_not_replay_frozen_state() {
    let mut plugin = ResonanceEq::new();
    plugin.initialize(48_000.0, 512);
    low_cut_48(&plugin);

    let mut loud = Vec::new();
    for n in 0..4 {
        loud = run(&mut plugin, &loud_noise(n, 512));
    }
    assert!(
        loud.iter().any(|v| v.abs() > 0.1),
        "the enabled band must pass loud input"
    );

    // Off for a block, then back on — both over silence.
    plugin.params.bands[0].enabled.set_value(false);
    run(&mut plugin, &[0.0; 512]);
    plugin.params.bands[0].enabled.set_value(true);
    let out = run(&mut plugin, &[0.0; 512]);
    assert!(
        out[..64].iter().all(|&v| v == 0.0),
        "re-enabled band replayed stale state: peak {}",
        out[..64].iter().fold(0.0f32, |m, v| m.max(v.abs()))
    );
}

#[test]
fn raising_the_slope_starts_the_new_stages_clean() {
    let mut plugin = ResonanceEq::new();
    plugin.initialize(48_000.0, 512);
    low_cut_48(&plugin);
    for n in 0..4 {
        run(&mut plugin, &loud_noise(n, 512));
    }

    // Drop to 12 dB/oct (stages 1..3 leave the active set) and let the
    // one remaining stage ring out over silence.
    plugin.params.bands[0].slope.set_value(0);
    for _ in 0..40 {
        run(&mut plugin, &[0.0; 512]);
    }
    // Back to 48 dB/oct: stages 1..3 re-enter and must not inject what
    // they held when they were dropped.
    plugin.params.bands[0].slope.set_value(2);
    let out = run(&mut plugin, &[0.0; 512]);
    let peak = out[..64].iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak < 1e-4, "re-activated stages replayed stale state: peak {peak}");
}
