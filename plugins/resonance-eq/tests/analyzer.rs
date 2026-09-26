//! Behavioural tests for the spectrum analyzer tap (`src/analyzer.rs`).
//!
//! The analyzer is display-only: `process()` pushes a mono downmix into a
//! lock-free ring for a background FFT worker and must never change the
//! rendered audio. `analyzer_tap_leaves_audio_bit_identical` pins that by
//! rendering the same signal through the full plugin (workers running)
//! and through a bare `EqDsp` + `Smoother` chain with no analyzer at all.
//! The spectrum tests then check the other direction: that what the audio
//! thread pushes actually comes out of the worker as a spectrum peaking
//! in the right 1/6-octave band.

use std::time::{Duration, Instant};

use resonance_eq::analyzer::NUM_OCTAVE_BINS;
use resonance_eq::dsp::EqDsp;
use resonance_eq::params::EqParams;
use resonance_eq::ResonanceEq;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin, Smoother, SmoothingStyle};

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

/// Apply the same non-default band setup to any param block, so the
/// plugin under test and the analyzer-free reference chain are
/// configured identically.
fn configure(params: &EqParams) {
    params.bands[0].enabled.set_value(true);
    params.bands[0].kind.set_value(3); // low cut
    params.bands[0].freq.set_value(60.0);
    params.bands[0].slope.set_value(2); // 48 dB/oct
    params.bands[3].enabled.set_value(true);
    params.bands[3].kind.set_value(0); // bell
    params.bands[3].freq.set_value(2_000.0);
    params.bands[3].gain.set_value(6.0);
    params.bands[3].q.set_value(2.0);
    params.output_gain.set_value(1.5);
}

/// Deterministic test signal: two tones plus an impulse at the start of
/// the render so both sustained and transient content pass through.
fn fill_block(left: &mut [f32], right: &mut [f32], start_sample: usize) {
    for i in 0..left.len() {
        let n = (start_sample + i) as f32;
        let a = (std::f32::consts::TAU * 220.0 * n / SR).sin() * 0.4;
        let b = (std::f32::consts::TAU * 3_130.0 * n / SR).sin() * 0.2;
        let impulse = if start_sample + i == 7 { 0.9 } else { 0.0 };
        left[i] = a + impulse;
        right[i] = b - 0.5 * a;
    }
}

#[test]
fn analyzer_tap_leaves_audio_bit_identical() {
    // Pin the denormal mode up front: the plugin's `process()` sets
    // flush-to-zero on its calling thread, and the reference chain must
    // run under the same mode for a bit-exact comparison.
    resonance_dsp::flush_denormals();

    const BLOCKS: usize = 24;

    // Full plugin, spectrum workers spawned and fed every block.
    let mut plugin = ResonanceEq::new();
    configure(&plugin.params);
    assert!(plugin.initialize(SR, BLOCK as u32));

    // Reference: the audio chain exactly as `lib.rs::process` drives it,
    // with no analyzer in sight.
    let ref_params = EqParams::default();
    configure(&ref_params);
    let mut ref_dsp = EqDsp::new(SR);
    let mut ref_smoother = Smoother::new(SmoothingStyle::Logarithmic(20.0));
    ref_smoother.set_sample_rate(SR);
    ref_smoother.reset(resonance_dsp::db_to_linear(ref_params.output_gain.value()));

    let mut ev = EventIterator::empty();
    for block in 0..BLOCKS {
        let start = block * BLOCK;

        let mut left = vec![0.0f32; BLOCK];
        let mut right = vec![0.0f32; BLOCK];
        fill_block(&mut left, &mut right, start);
        let mut outs = [OutputBuffer {
            left: &mut left,
            right: &mut right,
        }];
        plugin.process(&mut outs, BLOCK, &mut ev, None);

        let mut ref_left = vec![0.0f32; BLOCK];
        let mut ref_right = vec![0.0f32; BLOCK];
        fill_block(&mut ref_left, &mut ref_right, start);
        ref_dsp.update_from_params(&ref_params);
        ref_smoother.set_target(resonance_dsp::db_to_linear(ref_params.output_gain.value()));
        ref_dsp.process_stereo(&mut ref_left, &mut ref_right, &mut ref_smoother);

        for i in 0..BLOCK {
            assert_eq!(
                left[i].to_bits(),
                ref_left[i].to_bits(),
                "L differs at sample {} (block {block})",
                start + i
            );
            assert_eq!(
                right[i].to_bits(),
                ref_right[i].to_bits(),
                "R differs at sample {} (block {block})",
                start + i
            );
        }
    }
}

/// 1/6-octave band index whose log-frequency range contains `freq`.
/// Bands span 20 Hz – 20 kHz: index = N * log(f/20) / log(1000).
fn expected_band(freq: f32) -> usize {
    (NUM_OCTAVE_BINS as f32 * (freq / 20.0).log10() / 3.0).floor() as usize
}

#[test]
fn sine_spectrum_peaks_in_expected_band() {
    // 532 Hz sits at the centre of band 28, comfortably away from a band
    // edge, so FFT leakage cannot push the peak more than one band off.
    const FREQ: f32 = 532.0;
    // 16384 samples fits entirely inside the worker's 32768-sample ring,
    // so nothing is dropped even if the worker sleeps through the whole
    // push — and it covers two full 8192-point FFT windows.
    const TOTAL: usize = 16_384;

    let mut plugin = ResonanceEq::new();
    assert!(plugin.initialize(SR, BLOCK as u32));

    let mut ev = EventIterator::empty();
    for block in 0..(TOTAL / BLOCK) {
        let mut left = vec![0.0f32; BLOCK];
        let mut right = vec![0.0f32; BLOCK];
        for i in 0..BLOCK {
            let n = (block * BLOCK + i) as f32;
            let s = (std::f32::consts::TAU * FREQ * n / SR).sin() * 0.5;
            left[i] = s;
            right[i] = s;
        }
        let mut outs = [OutputBuffer {
            left: &mut left,
            right: &mut right,
        }];
        plugin.process(&mut outs, BLOCK, &mut ev, None);
    }

    // The worker polls its ring every ~16 ms; give it (generously) until
    // the deadline to drain, FFT, and publish.
    let state = plugin.analyzer_state().clone();
    let want = expected_band(FREQ);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let pre = state.latest_pre().expect("initialize() installed handles");
        let post = state.latest_post().expect("initialize() installed handles");
        if spectrum_peaks_at(&pre.magnitudes_db, want)
            && spectrum_peaks_at(&post.magnitudes_db, want)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "spectrum never peaked in band {want}±1: pre={:?} post={:?}",
            &pre.magnitudes_db[..],
            &post.magnitudes_db[..]
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// True once the loudest band is within one band of `want`, at a level
/// consistent with a 0.5-amplitude sine (~−6 dBFS, minus scalloping),
/// and clearly above spectrally distant bands.
fn spectrum_peaks_at(bands: &[f32; NUM_OCTAVE_BINS], want: usize) -> bool {
    let (peak_band, peak_db) = bands
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, v)| (i, *v))
        .expect("bands is non-empty");
    peak_band.abs_diff(want) <= 1
        && peak_db > -15.0
        && bands[10] < peak_db - 20.0
        && bands[50] < peak_db - 20.0
}
