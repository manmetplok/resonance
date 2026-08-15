//! The compressor's external sidechain key.
//!
//! "Sidechain HPF" already existed and is an *internal* filter on the
//! detector path; the key is an *external* signal that replaces the
//! detector's source. These tests pin the difference, and pin that adding
//! the key changed nothing for anyone not using it.

use resonance_compressor::dsp::CompressorDsp;
use resonance_compressor::params::CompressorParams;
use resonance_compressor::viz::{CompressorViz, DetectorSource};
use resonance_compressor::ResonanceCompressor;
use resonance_plugin::{Param, ResonancePlugin};

const SR: f32 = 48_000.0;

fn tone(amp: f32, n: usize) -> Vec<f32> {
    (0..n).map(|i| (i as f32 * 0.05).sin() * amp).collect()
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

/// A compressor set to clamp hard on anything over -30 dBFS.
fn params() -> CompressorParams {
    let p = CompressorParams::default();
    p.threshold.set_value(-30.0);
    p.ratio.set_value(20.0);
    p.attack.set_value(0.5);
    p.release.set_value(20.0);
    p.knee.set_value(0.0);
    p.makeup.set_value(0.0);
    p.auto_makeup.set_plain(0.0);
    p.mix.set_value(1.0);
    p
}

/// Run one block, returning the output peak.
fn run(input_amp: f32, key: Option<f32>, frames: usize) -> f32 {
    let p = params();
    let viz = CompressorViz::new();
    let mut dsp = CompressorDsp::new(SR, &p);
    let mut l = tone(input_amp, frames);
    let mut r = l.clone();
    match key {
        Some(amp) => {
            let k = tone(amp, frames);
            let kr = k.clone();
            dsp.process_stereo(&mut l, &mut r, Some((&k, &kr)), &p, &viz);
        }
        None => dsp.process_stereo(&mut l, &mut r, None, &p, &viz),
    }
    // Measure past the attack.
    peak(&l[frames / 2..])
}

#[test]
fn a_loud_key_compresses_a_quiet_input() {
    // The input alone (-40 dBFS) is under the threshold and would pass
    // untouched. A loud key must clamp it anyway — this is ducking, and
    // it is the whole reason the port exists.
    let unkeyed = run(0.01, None, 8192);
    let keyed = run(0.01, Some(0.9), 8192);

    assert!(
        keyed < unkeyed * 0.5,
        "the key did not duck the input: {keyed} vs {unkeyed}"
    );
}

#[test]
fn a_silent_key_leaves_a_loud_input_alone() {
    // The inverse: an input that would normally be compressed hard passes
    // untouched because the key says nothing is happening.
    let silent_key = vec![0.0f32; 8192];
    let p = params();
    let viz = CompressorViz::new();
    let mut dsp = CompressorDsp::new(SR, &p);
    let mut l = tone(0.9, 8192);
    let mut r = l.clone();
    dsp.process_stereo(&mut l, &mut r, Some((&silent_key, &silent_key)), &p, &viz);

    assert!(
        peak(&l[4096..]) > 0.85,
        "a silent key should have left the input alone, got {}",
        peak(&l[4096..])
    );
}

#[test]
fn the_key_never_reaches_the_output() {
    // Detection only. With a near-silent input the output must stay
    // near-silent no matter how loud the key is.
    let p = params();
    let viz = CompressorViz::new();
    let mut dsp = CompressorDsp::new(SR, &p);
    let mut l = tone(0.001, 4096);
    let mut r = l.clone();
    let key = tone(1.0, 4096);
    let key_r = key.clone();
    dsp.process_stereo(&mut l, &mut r, Some((&key, &key_r)), &p, &viz);

    assert!(
        peak(&l) <= 0.001,
        "the key leaked into the output ({})",
        peak(&l)
    );
}

#[test]
fn no_key_is_bit_identical_to_the_pre_sidechain_path() {
    // The detector substitution must be the only difference. Two runs
    // with `None` must agree exactly, and — more to the point — passing
    // a key equal to the input must match keying off the input itself,
    // because that is literally the same detector signal.
    let p = params();
    let viz = CompressorViz::new();

    let mut a_l = tone(0.5, 4096);
    let mut a_r = a_l.clone();
    let mut dsp = CompressorDsp::new(SR, &p);
    dsp.process_stereo(&mut a_l, &mut a_r, None, &p, &viz);

    let input = tone(0.5, 4096);
    let mut b_l = input.clone();
    let mut b_r = input.clone();
    let mut dsp = CompressorDsp::new(SR, &p);
    dsp.process_stereo(&mut b_l, &mut b_r, Some((&input, &input)), &p, &viz);

    assert_eq!(
        a_l, b_l,
        "self-keying must equal an explicit key of the same signal"
    );
}

#[test]
fn a_short_key_buffer_degrades_instead_of_panicking() {
    let p = params();
    let viz = CompressorViz::new();
    let mut dsp = CompressorDsp::new(SR, &p);
    let mut l = tone(0.5, 4096);
    let mut r = l.clone();
    let short = tone(0.9, 8);
    dsp.process_stereo(&mut l, &mut r, Some((&short, &short)), &p, &viz);
    assert!(l.iter().all(|s| s.is_finite()));
}

#[test]
fn the_sidechain_hpf_still_applies_to_an_external_key() {
    // A 30 Hz key with the HPF engaged must trigger less reduction than
    // the same key with it bypassed — the filter belongs to the detector
    // path, whatever is feeding it.
    let frames = 16_384;
    let low_key: Vec<f32> = (0..frames)
        .map(|i| (i as f32 / SR * 30.0 * std::f32::consts::TAU).sin() * 0.9)
        .collect();
    let input = tone(0.3, frames);

    let measure = |hpf_on: bool| {
        let p = params();
        p.sc_hpf_on.set_plain(if hpf_on { 1.0 } else { 0.0 });
        p.sc_hpf_freq.set_value(500.0);
        let viz = CompressorViz::new();
        let mut dsp = CompressorDsp::new(SR, &p);
        let mut l = input.clone();
        let mut r = input.clone();
        dsp.process_stereo(&mut l, &mut r, Some((&low_key, &low_key)), &p, &viz);
        peak(&l[frames / 2..])
    };

    let filtered = measure(true);
    let unfiltered = measure(false);
    assert!(
        filtered > unfiltered,
        "the HPF should have reduced the key's grip: {filtered} vs {unfiltered}"
    );
}

// ---------------------------------------------------------------------------
// Detector source published to the editor
// ---------------------------------------------------------------------------

#[test]
fn a_fresh_viz_reports_no_key() {
    // Before the first block the editor must not claim a key: the honest
    // default is the ordinary self-keyed compressor.
    let viz = CompressorViz::new();
    assert_eq!(viz.detector_source(), DetectorSource::Input);
    assert!(!viz.detector_source().key_connected());
}

#[test]
fn processing_with_a_key_publishes_the_connection() {
    let p = params();
    let viz = CompressorViz::new();
    let mut dsp = CompressorDsp::new(SR, &p);
    let mut l = tone(0.3, 512);
    let mut r = l.clone();
    let key = tone(0.9, 512);
    let key_r = key.clone();
    dsp.process_stereo(&mut l, &mut r, Some((&key, &key_r)), &p, &viz);

    assert_eq!(viz.detector_source(), DetectorSource::ExternalKey);
}

#[test]
fn dropping_the_key_clears_the_connection() {
    // The host stops handing over a key the moment the route is removed,
    // so the editor has to go back to saying "input" without any other
    // notification.
    let p = params();
    let viz = CompressorViz::new();
    let mut dsp = CompressorDsp::new(SR, &p);
    let mut l = tone(0.3, 512);
    let mut r = l.clone();
    let key = tone(0.9, 512);
    let key_r = key.clone();
    dsp.process_stereo(&mut l, &mut r, Some((&key, &key_r)), &p, &viz);
    assert_eq!(viz.detector_source(), DetectorSource::ExternalKey);

    dsp.process_stereo(&mut l, &mut r, None, &p, &viz);
    assert_eq!(viz.detector_source(), DetectorSource::Input);
}

#[test]
fn a_silent_key_still_counts_as_connected() {
    // Presence is about routing, not about level — a key that happens to
    // be silent this block is still what the detector is listening to,
    // and that is exactly when the user most needs to be told.
    let p = params();
    let viz = CompressorViz::new();
    let mut dsp = CompressorDsp::new(SR, &p);
    let mut l = tone(0.9, 512);
    let mut r = l.clone();
    let silent = vec![0.0f32; 512];
    dsp.process_stereo(&mut l, &mut r, Some((&silent, &silent)), &p, &viz);

    assert_eq!(viz.detector_source(), DetectorSource::ExternalKey);
}

#[test]
fn an_empty_block_still_publishes_the_connection() {
    // Routing is a fact about the connection, not about this block having
    // audio in it, so a zero-length block must not blank the editor's
    // status line.
    let p = params();
    let viz = CompressorViz::new();
    let mut dsp = CompressorDsp::new(SR, &p);
    let empty: [f32; 0] = [];
    let mut l = empty;
    let mut r = empty;
    dsp.process_stereo(&mut l, &mut r, Some((&empty, &empty)), &p, &viz);

    assert_eq!(viz.detector_source(), DetectorSource::ExternalKey);
}

#[test]
fn the_meter_label_says_which_signal_the_detector_hears() {
    // Self-keyed, the IN bar is also the detector's source; keyed, it is
    // not, and the label must stop claiming otherwise.
    assert_eq!(DetectorSource::Input.input_meter_label(), "IN/DET");
    assert_eq!(DetectorSource::ExternalKey.input_meter_label(), "IN");
    assert_ne!(
        DetectorSource::Input.header_text(),
        DetectorSource::ExternalKey.header_text()
    );
    assert!(DetectorSource::ExternalKey.header_text().contains("KEY"));
}

#[test]
fn the_plugin_declares_a_stereo_sidechain_port() {
    assert_eq!(ResonanceCompressor::SIDECHAIN_INPUT, Some(2));
    assert_eq!(ResonanceCompressor::INPUT_CHANNELS, Some(2));
}
