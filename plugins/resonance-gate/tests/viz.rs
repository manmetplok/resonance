//! The detector status the editor reads (ba todo #1314, finding C3).
//!
//! `key_connected` used to be computed, documented as UI-facing, and
//! read by nothing but a test. These tests follow it the whole way:
//! audio thread → `GateViz` → the `DetectorSummary` the header draws
//! from, so a regression that leaves the editor showing the wrong
//! detector fails here.

#![cfg(feature = "editor")]

use resonance_gate::dsp::GateState;
use resonance_gate::editor::status::DetectorSummary;
use resonance_gate::viz::DetectorSource;
use resonance_gate::ResonanceGate;
use resonance_plugin::{EventIterator, KeyBuffer, OutputBuffer, ResonancePlugin, TempoInfo};

const SR: f32 = 48_000.0;
const FRAMES: usize = 512;

fn tone(amp: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| amp * (i as f32 / SR * 1000.0 * std::f32::consts::TAU).sin())
        .collect()
}

/// Run one block through the plugin, optionally with a key, and return
/// the summary the editor header would draw afterwards.
fn run_block(plugin: &mut ResonanceGate, input: &[f32], key: Option<&[f32]>) -> DetectorSummary {
    let mut left = input.to_vec();
    let mut right = input.to_vec();
    let mut outs = [OutputBuffer {
        left: &mut left,
        right: &mut right,
    }];
    let mut ev = EventIterator::empty();
    plugin.process_with_key(
        &mut outs,
        key.map(|k| KeyBuffer { left: k, right: k }),
        FRAMES,
        &mut ev,
        None::<TempoInfo>,
    );
    DetectorSummary::from_viz(plugin.viz())
}

#[test]
fn a_fresh_plugin_claims_nothing() {
    let plugin = ResonanceGate::new();
    let summary = DetectorSummary::from_viz(plugin.viz());
    assert_eq!(summary.source, DetectorSource::SelfInput);
    assert_eq!(summary.state, GateState::Closed);
    assert_eq!(
        summary.detector_text, "—",
        "nothing has been processed, so there is no detector level to report"
    );
}

#[test]
fn the_key_connected_flag_reaches_the_editor_facing_struct() {
    let mut plugin = ResonanceGate::new();
    plugin.initialize(SR, FRAMES as u32);
    let input = tone(0.5, FRAMES);
    let key = tone(0.9, FRAMES);

    let keyless = run_block(&mut plugin, &input, None);
    assert_eq!(keyless.source, DetectorSource::SelfInput);
    assert_eq!(keyless.source_label, "INPUT");

    let keyed = run_block(&mut plugin, &input, Some(&key));
    assert_eq!(keyed.source, DetectorSource::ExternalKey);
    assert_eq!(keyed.source_label, "EXTERNAL KEY");

    // And back again when the host disconnects it — the flag is per
    // block, not sticky.
    let keyless = run_block(&mut plugin, &input, None);
    assert_eq!(keyless.source, DetectorSource::SelfInput);
}

#[test]
fn the_detector_state_reaches_the_editor_facing_struct() {
    let mut plugin = ResonanceGate::new();
    plugin.initialize(SR, FRAMES as u32);
    // Default threshold is -40 dBFS: a loud block opens the gate, a
    // silent one closes it once hold (20 ms default) has expired.
    let loud = tone(0.9, FRAMES);
    let silence = vec![0.0f32; FRAMES];

    let opening = run_block(&mut plugin, &loud, None);
    assert_eq!(opening.state, GateState::Open);
    assert_eq!(opening.state_label, "OPEN");

    // The gate reports Open from the first block, but the gain is still
    // ramping up out of the closed state; hold the loud signal until the
    // ramp has run out (the default release is 100 ms per time
    // constant) and the reduction readout reaches a flat zero.
    let mut open = opening;
    for _ in 0..150 {
        open = run_block(&mut plugin, &loud, None);
    }
    assert_eq!(open.state, GateState::Open);
    assert_eq!(
        open.gr_text, "0.0 dB",
        "a settled open gate is not attenuating anything"
    );

    // On silence the detector envelope has to decay ~45 dB to the close
    // threshold (15 ms per 8.7 dB) before the 20 ms hold timer even
    // starts, so "closed" is ~100 ms away, not one block. Run well past
    // that so the gate is genuinely closed rather than still holding.
    let mut closed = run_block(&mut plugin, &silence, None);
    for _ in 0..40 {
        closed = run_block(&mut plugin, &silence, None);
    }
    assert_eq!(closed.state, GateState::Closed);
    assert_eq!(closed.state_label, "CLOSED");
}

#[test]
fn the_reported_detector_level_follows_the_key_not_the_input() {
    let mut plugin = ResonanceGate::new();
    plugin.initialize(SR, FRAMES as u32);
    let quiet_input = tone(0.01, FRAMES); // ≈ -40 dBFS
    let loud_key = tone(0.9, FRAMES); // ≈ -1 dBFS

    let keyed = run_block(&mut plugin, &quiet_input, Some(&loud_key));
    let level: f32 = keyed
        .detector_text
        .trim_end_matches(" dB")
        .parse()
        .expect("detector readout should be a number once a block has run");
    assert!(
        level > -6.0,
        "the readout should follow the loud key, not the quiet input; got {}",
        keyed.detector_text
    );

    // Same input, no key: now the detector reads the quiet input.
    plugin.reset();
    let keyless = run_block(&mut plugin, &quiet_input, None);
    let level: f32 = keyless
        .detector_text
        .trim_end_matches(" dB")
        .parse()
        .expect("detector readout should be a number once a block has run");
    assert!(
        level < -20.0,
        "without a key the readout should follow the quiet input; got {}",
        keyless.detector_text
    );
}

#[test]
fn reset_clears_what_the_editor_reports() {
    let mut plugin = ResonanceGate::new();
    plugin.initialize(SR, FRAMES as u32);
    let loud = tone(0.9, FRAMES);
    let key = tone(0.9, FRAMES);
    let keyed = run_block(&mut plugin, &loud, Some(&key));
    assert_eq!(keyed.source, DetectorSource::ExternalKey);

    plugin.reset();
    let summary = DetectorSummary::from_viz(plugin.viz());
    assert_eq!(summary.source, DetectorSource::SelfInput);
    assert_eq!(summary.state, GateState::Closed);
    assert_eq!(summary.detector_text, "—");
}
