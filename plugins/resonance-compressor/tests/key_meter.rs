//! The level of the sidechain key, published for the editor's key meter
//! (ba todo #1342, finding C2 of ba doc #275).
//!
//! #1313 published the key's PRESENCE, which tells the user why the GR
//! meter is moving while the input meter is idle. It does not tell them
//! by how much: with a key connected, the number that explains the gain
//! reduction — the detector level being compared against the threshold —
//! was nowhere on screen. These tests pin what the DSP now publishes.
//!
//! The published value is deliberately the DETECTOR level, not the raw
//! key samples: it is post sidechain-HPF and post peak/RMS blend, which
//! is exactly the quantity the threshold is compared against, so the
//! meter and the threshold marker drawn across it mean the same thing.

use resonance_compressor::dsp::CompressorDsp;
use resonance_compressor::params::CompressorParams;
use resonance_compressor::viz::{CompressorViz, DetectorSource, KEY_METER_LABEL};
use resonance_plugin::Param;

const SR: f32 = 48_000.0;
const FRAMES: usize = 8192;

fn tone(amp: f32, n: usize) -> Vec<f32> {
    (0..n).map(|i| (i as f32 * 0.05).sin() * amp).collect()
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

/// Run one block of `input_amp` against an optional key, returning the
/// viz the editor would read.
fn run(p: &CompressorParams, input_amp: f32, key_amp: Option<f32>) -> std::sync::Arc<CompressorViz> {
    let viz = CompressorViz::new();
    let mut dsp = CompressorDsp::new(SR, p);
    let mut l = tone(input_amp, FRAMES);
    let mut r = l.clone();
    match key_amp {
        Some(amp) => {
            let k = tone(amp, FRAMES);
            let kr = k.clone();
            dsp.process_stereo(&mut l, &mut r, Some((&k, &kr)), p, &viz);
        }
        None => dsp.process_stereo(&mut l, &mut r, None, p, &viz),
    }
    viz
}

// ---------------------------------------------------------------------------
// Presence: no key, no level
// ---------------------------------------------------------------------------

#[test]
fn a_fresh_viz_reports_no_key_level() {
    // Nothing has been processed, so there is nothing to claim — the
    // editor must draw no key meter at all rather than an empty one.
    assert_eq!(CompressorViz::new().read_key_db(), None);
}

#[test]
fn a_self_keyed_block_publishes_no_key_level() {
    // Without a key the detector reads the input, which the IN/DET meter
    // already shows; a second bar for the same signal would be a lie
    // about the routing.
    let viz = run(&params(), 0.9, None);
    assert_eq!(viz.read_key_db(), None);
    assert_eq!(viz.detector_source(), DetectorSource::Input);
}

#[test]
fn dropping_the_key_drops_the_level_rather_than_freezing_it() {
    // The host simply stops handing over a key when the route goes away.
    // A meter left holding the last level it saw would be the worst of
    // both worlds: a number that looks live and is not.
    let p = params();
    let viz = CompressorViz::new();
    let mut dsp = CompressorDsp::new(SR, &p);
    let mut l = tone(0.1, FRAMES);
    let mut r = l.clone();
    let key = tone(0.9, FRAMES);
    let key_r = key.clone();

    dsp.process_stereo(&mut l, &mut r, Some((&key, &key_r)), &p, &viz);
    assert!(viz.read_key_db().is_some(), "the key level was never published");

    dsp.process_stereo(&mut l, &mut r, None, &p, &viz);
    assert_eq!(
        viz.read_key_db(),
        None,
        "the key level survived the key being unrouted"
    );
    assert_eq!(viz.detector_source(), DetectorSource::Input);
}

#[test]
fn a_silent_key_reads_as_silence_and_not_as_absence() {
    // Connected-but-silent and not-connected look identical on a bar
    // meter and mean completely different things. Presence is about
    // routing, so a silent key still gets a meter — showing silence.
    let p = params();
    let viz = CompressorViz::new();
    let mut dsp = CompressorDsp::new(SR, &p);
    let mut l = tone(0.9, FRAMES);
    let mut r = l.clone();
    let silent = vec![0.0f32; FRAMES];
    dsp.process_stereo(&mut l, &mut r, Some((&silent, &silent)), &p, &viz);

    let key_db = viz.read_key_db().expect("a connected key must be metered");
    assert!(
        key_db < -90.0,
        "a silent key should read as silence, got {key_db} dBFS"
    );
    assert_eq!(viz.detector_source(), DetectorSource::ExternalKey);
}

// ---------------------------------------------------------------------------
// Level: it is the key's, and it is the number the threshold sees
// ---------------------------------------------------------------------------

#[test]
fn the_metered_level_is_the_keys_and_not_the_inputs() {
    // The whole point: a quiet track ducked by a loud key. The IN meter
    // shows the quiet track, and the key meter must show the loud key —
    // if it tracked the input it would explain nothing.
    let viz = run(&params(), 0.01, Some(0.9));

    let key_db = viz.read_key_db().expect("a connected key must be metered");
    let input_db = viz.read_input_db();
    assert!(
        key_db > input_db + 20.0,
        "key {key_db} dBFS vs input {input_db} dBFS — the key meter is following the input"
    );
    // A full-scale-ish sine reads near 0 dBFS however the detector is
    // blended; anything far below that means the wrong signal.
    assert!(
        (-10.0..=0.5).contains(&key_db),
        "a -1 dBFS key metered as {key_db} dBFS"
    );
}

#[test]
fn a_louder_key_meters_louder() {
    let p = params();
    let quiet = run(&p, 0.01, Some(0.05)).read_key_db().unwrap();
    let loud = run(&p, 0.01, Some(0.9)).read_key_db().unwrap();
    assert!(
        loud > quiet + 10.0,
        "a 25 dB louder key only moved the meter from {quiet} to {loud}"
    );
}

#[test]
fn the_metered_level_is_what_the_threshold_is_compared_against() {
    // The meter earns its place by explaining the GR meter next to it:
    // a key metered above the threshold must be reducing gain, and one
    // metered below it must not. Anything else and the two meters tell
    // different stories about the same block.
    let p = params();
    let threshold = p.threshold.value();

    let over = run(&p, 0.01, Some(0.9));
    let over_db = over.read_key_db().unwrap();
    assert!(over_db > threshold, "{over_db} should be over {threshold}");
    assert!(
        over.read_gr_db() > 1.0,
        "the key meters over the threshold but nothing is being reduced"
    );

    let under = run(&p, 0.01, Some(0.001));
    let under_db = under.read_key_db().unwrap();
    assert!(under_db < threshold, "{under_db} should be under {threshold}");
    assert!(
        under.read_gr_db() < 0.1,
        "the key meters under the threshold but gain is being reduced anyway"
    );
}

#[test]
fn the_sidechain_hpf_is_inside_the_metered_level() {
    // The HPF is part of the detector path, so a key the filter has
    // gutted must meter as quiet as it now is — otherwise the meter
    // would show a key hammering a threshold it no longer reaches.
    let low_key: Vec<f32> = (0..FRAMES)
        .map(|i| (i as f32 / SR * 30.0 * std::f32::consts::TAU).sin() * 0.9)
        .collect();
    let input = tone(0.3, FRAMES);

    let measure = |hpf_on: bool| {
        let p = params();
        p.sc_hpf_on.set_plain(if hpf_on { 1.0 } else { 0.0 });
        p.sc_hpf_freq.set_value(500.0);
        let viz = CompressorViz::new();
        let mut dsp = CompressorDsp::new(SR, &p);
        let mut l = input.clone();
        let mut r = input.clone();
        dsp.process_stereo(&mut l, &mut r, Some((&low_key, &low_key)), &p, &viz);
        viz.read_key_db().unwrap()
    };

    assert!(
        measure(true) < measure(false) - 6.0,
        "the HPF took the key apart but the meter did not notice: {} vs {}",
        measure(true),
        measure(false)
    );
}

#[test]
fn a_short_key_buffer_still_meters_something_finite() {
    let p = params();
    let viz = CompressorViz::new();
    let mut dsp = CompressorDsp::new(SR, &p);
    let mut l = tone(0.5, FRAMES);
    let mut r = l.clone();
    let short = tone(0.9, 8);
    dsp.process_stereo(&mut l, &mut r, Some((&short, &short)), &p, &viz);

    let key_db = viz.read_key_db().expect("a connected key must be metered");
    assert!(key_db.is_finite(), "the key metered as {key_db}");
}

// ---------------------------------------------------------------------------
// Labelling: exactly one bar claims to be the detector's source
// ---------------------------------------------------------------------------

#[test]
fn the_detector_mark_moves_from_the_input_to_the_key() {
    // `/DET` is the editor's mark for "this bar is what the detector
    // hears". Self-keyed it is on the input; keyed it must be on the key
    // meter and nowhere else, or two bars claim the same job.
    assert!(DetectorSource::Input.input_meter_label().ends_with("/DET"));
    assert!(!DetectorSource::ExternalKey
        .input_meter_label()
        .ends_with("/DET"));
    assert!(KEY_METER_LABEL.ends_with("/DET"));
    assert!(KEY_METER_LABEL.starts_with("KEY"));
}
