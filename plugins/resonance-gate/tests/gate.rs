//! The gate's static curve, its open/close behaviour, and — the reason
//! this plugin exists — that an external key really does replace the
//! detector source.

use resonance_gate::dsp::{expander_gain_reduction_db, GateDsp, GateSettings};
use resonance_gate::ResonanceGate;
use resonance_plugin::{
    EventIterator, KeyBuffer, OutputBuffer, ResonancePlugin, TempoInfo,
};

const SR: f32 = 48_000.0;

fn settings() -> GateSettings {
    GateSettings {
        threshold_db: -40.0,
        ratio: 8.0,
        attack_ms: 0.1,
        hold_ms: 0.0,
        release_ms: 5.0,
        range_db: 60.0,
        hysteresis_db: 0.0,
        key_hpf_hz: 0.0,
    }
}

/// Peak absolute sample of a slice.
fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

/// Constant-amplitude signal.
fn tone(amp: f32, n: usize) -> Vec<f32> {
    (0..n).map(|i| (i as f32 * 0.05).sin() * amp).collect()
}

// ---------------------------------------------------------------------------
// The static curve
// ---------------------------------------------------------------------------

#[test]
fn signal_above_the_threshold_is_untouched() {
    assert_eq!(expander_gain_reduction_db(-10.0, -40.0, 8.0, 60.0), 0.0);
    assert_eq!(expander_gain_reduction_db(-40.0, -40.0, 8.0, 60.0), 0.0);
}

#[test]
fn signal_below_the_threshold_attenuates_by_the_expander_slope() {
    // 10 dB under at ratio 4 => slope 3 => 30 dB of reduction.
    let gr = expander_gain_reduction_db(-50.0, -40.0, 4.0, 60.0);
    assert!((gr - 30.0).abs() < 1e-4, "got {gr} dB");
    // Twice as far under is twice the reduction, until the range caps it.
    let gr2 = expander_gain_reduction_db(-45.0, -40.0, 4.0, 60.0);
    assert!((gr2 - 15.0).abs() < 1e-4, "got {gr2} dB");
}

#[test]
fn range_caps_the_attenuation() {
    // Far below threshold at a steep ratio would be hundreds of dB.
    let gr = expander_gain_reduction_db(-120.0, -40.0, 20.0, 24.0);
    assert!((gr - 24.0).abs() < 1e-4, "range must cap at 24 dB, got {gr}");
    // A range of zero is a bypass, not a mute.
    assert_eq!(expander_gain_reduction_db(-120.0, -40.0, 20.0, 0.0), 0.0);
}

#[test]
fn ratio_one_never_attenuates() {
    // Slope 0: the control cannot silently turn into a gate.
    for det in [-50.0, -80.0, -120.0] {
        assert_eq!(expander_gain_reduction_db(det, -40.0, 1.0, 60.0), 0.0);
    }
}

// ---------------------------------------------------------------------------
// Gating a signal
// ---------------------------------------------------------------------------

#[test]
fn a_loud_signal_passes_and_a_quiet_one_is_gated() {
    let mut dsp = GateDsp::new(SR);
    let s = settings();

    let mut l = tone(0.5, 4096);
    let mut r = l.clone();
    dsp.process_block(&mut l, &mut r, None, 4096, &s);
    assert!(peak(&l) > 0.4, "a loud signal must pass, got {}", peak(&l));

    let mut dsp = GateDsp::new(SR);
    // -60 dBFS, well under the -40 dB threshold.
    let mut l = tone(0.001, 4096);
    let mut r = l.clone();
    dsp.process_block(&mut l, &mut r, None, 4096, &s);
    // The tail of the block is past the release, so check there.
    assert!(
        peak(&l[2048..]) < 0.0001,
        "a quiet signal must be gated, got {}",
        peak(&l[2048..])
    );
}

#[test]
fn hold_keeps_the_gate_open_across_a_gap() {
    // A gap shorter than the hold must not close the gate: this is what
    // stops a gate chopping the space between snare hits.
    let mut s = settings();
    s.hold_ms = 50.0;
    s.release_ms = 1.0;

    let mut dsp = GateDsp::new(SR);
    // Open the gate in its own block first. `last_gr_db` is the PEAK
    // reduction across a block, and every gate starts closed, so mixing
    // the opening transient into the measured block would report the
    // startup ramp rather than what the hold did.
    let mut l = tone(0.5, 2048);
    let mut r = l.clone();
    dsp.process_block(&mut l, &mut r, None, 2048, &s);
    assert!(dsp.last_open, "the loud block should have opened the gate");

    // Now a silent gap of ~21 ms — well inside the 50 ms hold.
    let mut l = vec![0.0f32; 1024];
    let mut r = vec![0.0f32; 1024];
    dsp.process_block(&mut l, &mut r, None, 1024, &s);

    assert!(dsp.last_open, "the hold timer should still be running");
    assert!(
        dsp.last_gr_db < 1.0,
        "the gate closed inside its hold window ({} dB)",
        dsp.last_gr_db
    );

    // Past the hold it does close.
    let mut l = vec![0.0f32; 8192];
    let mut r = vec![0.0f32; 8192];
    dsp.process_block(&mut l, &mut r, None, 8192, &s);
    assert!(!dsp.last_open, "the gate should close once the hold expires");
}

#[test]
fn hysteresis_stops_a_borderline_signal_chattering() {
    // A signal sitting between the open and close thresholds must hold
    // whatever state it was in rather than flapping.
    let mut s = settings();
    s.hysteresis_db = 12.0;
    s.release_ms = 1.0;
    s.attack_ms = 0.05;

    let mut dsp = GateDsp::new(SR);
    // Open it with a loud burst…
    let mut l = tone(0.5, 1024);
    let mut r = l.clone();
    dsp.process_block(&mut l, &mut r, None, 1024, &s);
    assert!(dsp.last_open);

    // …then drop to -45 dBFS: below the -40 open threshold, but above the
    // -52 close threshold. It must stay open.
    let mut l = tone(0.0056, 4096);
    let mut r = l.clone();
    dsp.process_block(&mut l, &mut r, None, 4096, &s);
    assert!(
        dsp.last_open,
        "hysteresis should have held the gate open ({} dB GR)",
        dsp.last_gr_db
    );

    // Drop below the close threshold and it finally shuts.
    let mut l = tone(0.0005, 8192);
    let mut r = l.clone();
    dsp.process_block(&mut l, &mut r, None, 8192, &s);
    assert!(!dsp.last_open, "the gate should have closed");
}

// ---------------------------------------------------------------------------
// The external key — the whole point
// ---------------------------------------------------------------------------

#[test]
fn an_external_key_opens_the_gate_on_a_silent_input() {
    // Input near silence would gate itself shut. A loud key must open it
    // anyway — this is "duck/open this pad from the kick", and it is the
    // behaviour that no amount of internal detection can produce.
    let mut s = settings();
    s.attack_ms = 0.05;
    s.range_db = 60.0;

    // -60 dBFS: unambiguously under the -40 dB threshold.
    let quiet = tone(0.001, 4096);
    let key = tone(0.9, 4096);

    // Without a key: gated.
    let mut dsp = GateDsp::new(SR);
    let mut l = quiet.clone();
    let mut r = quiet.clone();
    dsp.process_block(&mut l, &mut r, None, 4096, &s);
    let closed_peak = peak(&l[2048..]);

    // With a loud key: open, and the input passes at unity.
    let mut dsp = GateDsp::new(SR);
    let mut l = quiet.clone();
    let mut r = quiet.clone();
    dsp.process_block(&mut l, &mut r, Some((&key, &key)), 4096, &s);
    let open_peak = peak(&l[2048..]);

    assert!(
        open_peak > closed_peak * 10.0,
        "the key did not open the gate: {open_peak} vs {closed_peak}"
    );
    // And the output is the INPUT, not the key: the key only detects.
    assert!(
        open_peak <= 0.0011,
        "the key leaked into the output ({open_peak})"
    );
}

#[test]
fn a_silent_key_closes_the_gate_on_a_loud_input() {
    // The inverse: a loud input that would pass on its own is gated shut
    // because the key says nothing is happening.
    let s = settings();
    let loud = tone(0.8, 4096);
    let silent_key = vec![0.0f32; 4096];

    let mut dsp = GateDsp::new(SR);
    let mut l = loud.clone();
    let mut r = loud.clone();
    dsp.process_block(&mut l, &mut r, Some((&silent_key, &silent_key)), 4096, &s);

    assert!(
        peak(&l[2048..]) < 0.01,
        "a silent key should have closed the gate, got {}",
        peak(&l[2048..])
    );
}

#[test]
fn the_key_high_pass_keeps_low_bleed_from_holding_the_gate_open() {
    // A 30 Hz key: with the HPF off it opens the gate, with a 500 Hz HPF
    // it should not. This is the kick-bleed-into-a-snare-gate case.
    let mut s = settings();
    s.attack_ms = 0.05;
    s.release_ms = 1.0;
    // A one-pole rejects 30 Hz by ~20·log10(30/500) ≈ 24 dB — real
    // attenuation, not a brick wall. Put the threshold where that 24 dB
    // decides the outcome: the raw key is ~-2 dBFS, the filtered one
    // ~-26 dBFS.
    s.threshold_db = -12.0;
    let frames = 8192;
    let low_key: Vec<f32> = (0..frames)
        .map(|i| (i as f32 / SR * 30.0 * std::f32::consts::TAU).sin() * 0.8)
        .collect();
    let quiet = tone(0.01, frames);

    let mut dsp = GateDsp::new(SR);
    let mut l = quiet.clone();
    let mut r = quiet.clone();
    dsp.process_block(&mut l, &mut r, Some((&low_key, &low_key)), frames, &s);
    assert!(dsp.last_open, "without the HPF the low key opens the gate");

    s.key_hpf_hz = 500.0;
    let mut dsp = GateDsp::new(SR);
    let mut l = quiet.clone();
    let mut r = quiet;
    dsp.process_block(&mut l, &mut r, Some((&low_key, &low_key)), frames, &s);
    assert!(
        !dsp.last_open,
        "the 500 Hz key HPF should have rejected a 30 Hz key"
    );
}

#[test]
fn a_short_key_buffer_is_treated_as_silence_not_a_panic() {
    // The host is supposed to hand over a full-length key, but a
    // truncated one must degrade rather than index out of bounds.
    let s = settings();
    let mut dsp = GateDsp::new(SR);
    let mut l = tone(0.5, 1024);
    let mut r = l.clone();
    let short_key = tone(0.9, 16);
    dsp.process_block(&mut l, &mut r, Some((&short_key, &short_key)), 1024, &s);
    assert!(l.iter().all(|s| s.is_finite()));
}

// ---------------------------------------------------------------------------
// Plugin surface
// ---------------------------------------------------------------------------

#[test]
fn the_plugin_declares_a_stereo_sidechain_port() {
    assert_eq!(ResonanceGate::SIDECHAIN_INPUT, Some(2));
    assert_eq!(ResonanceGate::INPUT_CHANNELS, Some(2));
}

#[test]
fn every_param_index_resolves_to_a_distinct_control() {
    let plugin = ResonanceGate::new();
    assert_eq!(plugin.param_count(), resonance_gate::params::PARAM_COUNT);
    let ids: std::collections::HashSet<&str> = (0..plugin.param_count())
        .map(|i| plugin.param(i).id())
        .collect();
    assert_eq!(ids.len(), plugin.param_count(), "duplicate param ids");
    for id in [
        "threshold",
        "ratio",
        "attack",
        "hold",
        "release",
        "range",
        "hysteresis",
        "key_hpf",
    ] {
        assert!(ids.contains(id), "{id} is not reachable through param_at");
    }
}

#[test]
fn process_with_key_reports_whether_a_key_was_connected() {
    let mut plugin = ResonanceGate::new();
    plugin.initialize(SR, 512);

    let mut left = tone(0.5, 512);
    let mut right = left.clone();
    let mut outs = [OutputBuffer {
        left: &mut left,
        right: &mut right,
    }];
    let mut ev = EventIterator::empty();
    plugin.process_with_key(&mut outs, None, 512, &mut ev, None::<TempoInfo>);
    assert!(!plugin.key_connected());

    let key_l = tone(0.9, 512);
    let key_r = key_l.clone();
    let mut left = tone(0.5, 512);
    let mut right = left.clone();
    let mut outs = [OutputBuffer {
        left: &mut left,
        right: &mut right,
    }];
    plugin.process_with_key(
        &mut outs,
        Some(KeyBuffer {
            left: &key_l,
            right: &key_r,
        }),
        512,
        &mut ev,
        None::<TempoInfo>,
    );
    assert!(plugin.key_connected());
}
