//! A non-finite sample must never wedge the compressor.
//!
//! Every envelope in `dsp.rs` is a one-pole recursion
//! (`x + (state − x) * coef`) that never leaves NaN once it holds one:
//! without the guards there, a single NaN detector sample latched
//! `gr_db` and muted or garbled the track until reset. The host scrubs
//! plugin OUTPUT at the mixer boundary, but the plugin's own state must
//! survive on its own — these are CLAP plugins usable in hosts that
//! don't scrub, and the bad value can come from the plugin's own math
//! at an extreme setting.
//!
//! The guards must be invisible for finite input. `tests/dsp_golden.rs`
//! pins that bit-exactly against the checked-in fixture; the null test
//! below additionally pins that a poisoned-then-reset DSP renders bit
//! identically to a fresh one.

use resonance_compressor::dsp::CompressorDsp;
use resonance_compressor::params::CompressorParams;
use resonance_compressor::viz::CompressorViz;

const SR: f32 = 48_000.0;
const BLOCK: usize = 512;

fn fast_params() -> CompressorParams {
    let params = CompressorParams::default();
    params.threshold.set_value(-18.0);
    // Fastest useful release so "recovers" is bounded in a few blocks.
    params.release.set_value(5.0);
    params.mix.set_value(1.0);
    params.makeup.set_value(0.0);
    params.auto_makeup.set_value(false);
    params
}

/// Deterministic tone at a constant amplitude.
fn tone(amp: f32, offset: usize, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| ((offset + i) as f32 * 0.05).sin() * amp)
        .collect()
}

/// A block of the values the guards exist for: NaN and both infinities.
fn poison(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| match i % 3 {
            0 => f32::NAN,
            1 => f32::INFINITY,
            _ => f32::NEG_INFINITY,
        })
        .collect()
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

/// One poisoned INPUT block between clean ones: every block after the
/// poisoned one must be finite, and gain reduction must return to 0 dB
/// — a quiet signal far below the threshold must pass at unity again.
#[test]
fn nan_input_block_recovers_within_bounded_blocks() {
    let params = fast_params();
    let viz = CompressorViz::new();
    let mut dsp = CompressorDsp::new(SR, &params);
    let mut n = 0;

    // Establish real gain reduction so the poison lands mid-envelope.
    for _ in 0..4 {
        let mut l = tone(0.5, n, BLOCK);
        let mut r = tone(0.5, n, BLOCK);
        dsp.process_stereo(&mut l, &mut r, None, &params, &viz);
        n += BLOCK;
    }

    // The poisoned block. Its own output is allowed to be non-finite —
    // the host scrubs output — but the STATE must not stay poisoned.
    let mut l = poison(BLOCK);
    let mut r = poison(BLOCK);
    dsp.process_stereo(&mut l, &mut r, None, &params, &viz);

    // Clean quiet blocks, -40 dBFS: far below the threshold, so a
    // recovered compressor applies no gain reduction and (mix 1,
    // makeup 0) the output converges on the input.
    const CLEAN_BLOCKS: usize = 8;
    let mut worst_last_block = 0.0f32;
    for block in 0..CLEAN_BLOCKS {
        let input = tone(0.01, n, BLOCK);
        let mut l = input.clone();
        let mut r = input.clone();
        dsp.process_stereo(&mut l, &mut r, None, &params, &viz);
        n += BLOCK;
        assert!(
            l.iter().chain(r.iter()).all(|x| x.is_finite()),
            "non-finite output {} block(s) after the poisoned one",
            block + 1
        );
        if block == CLEAN_BLOCKS - 1 {
            worst_last_block = l
                .iter()
                .zip(&input)
                .fold(0.0f32, |m, (o, i)| m.max((o - i).abs()));
        }
    }
    assert!(
        worst_last_block < 1e-4,
        "gain reduction never returned to 0 dB: output still {worst_last_block:.3e} \
         away from a sub-threshold input {CLEAN_BLOCKS} blocks after the poison"
    );
}

/// A poisoned external KEY never reaches the audio path, so with clean
/// input the output must stay finite through the poisoned blocks
/// themselves — the guarded detector reads the bad key as silence and
/// keeps the applied gain finite. Afterwards the sidechain must still
/// work: a loud key must duck the signal again.
#[test]
fn nan_key_never_reaches_the_output_and_detection_recovers() {
    let params = fast_params();
    // Exercise the biquad reset path with real HPF coefficients, not
    // the identity bypass.
    params.sc_hpf_on.set_value(true);
    params.sc_hpf_freq.set_value(500.0);
    let viz = CompressorViz::new();
    let mut dsp = CompressorDsp::new(SR, &params);
    let mut n = 0;

    let key_l = poison(BLOCK);
    let key_r = poison(BLOCK);
    for block in 0..4 {
        let mut l = tone(0.25, n, BLOCK);
        let mut r = tone(0.25, n, BLOCK);
        dsp.process_stereo(
            &mut l,
            &mut r,
            Some((&key_l, &key_r)),
            &params,
            &viz,
        );
        n += BLOCK;
        assert!(
            l.iter().chain(r.iter()).all(|x| x.is_finite()),
            "a poisoned key leaked non-finite samples into block {block}"
        );
    }

    // A clean, loud key must still drive gain reduction.
    let key = tone(0.9, 0, BLOCK);
    let mut settled = Vec::new();
    for _ in 0..8 {
        let input = tone(0.25, n, BLOCK);
        let mut l = input.clone();
        let mut r = input.clone();
        dsp.process_stereo(&mut l, &mut r, Some((&key, &key)), &params, &viz);
        n += BLOCK;
        settled = l;
    }
    let input_peak = 0.25;
    assert!(
        peak(&settled) < input_peak * 0.9,
        "the sidechain stopped compressing after a poisoned key: peak {} vs input {}",
        peak(&settled),
        input_peak
    );
}

/// Null test: poisoning the DSP and then `reset()`-ing it must leave no
/// trace — the same finite render, sample for sample, bit for bit. This
/// pins both that `reset` clears every guarded state and that the
/// guards themselves never fire on a finite path.
#[test]
fn finite_render_is_bit_identical_after_poison_and_reset() {
    let params = fast_params();

    let render = |dsp: &mut CompressorDsp, viz: &CompressorViz| -> Vec<f32> {
        let mut out = Vec::new();
        let mut n = 0;
        for block in 0..12 {
            // Level moves across blocks so the envelopes actually work.
            let amp = if block % 3 == 0 { 0.5 } else { 0.02 };
            let mut l = tone(amp, n, BLOCK);
            let mut r = tone(amp * 0.79, n, BLOCK);
            dsp.process_stereo(&mut l, &mut r, None, &params, viz);
            n += BLOCK;
            out.extend_from_slice(&l);
            out.extend_from_slice(&r);
        }
        out
    };

    let viz = CompressorViz::new();
    let mut fresh = CompressorDsp::new(SR, &params);
    let clean = render(&mut fresh, &viz);

    let viz = CompressorViz::new();
    let mut healed = CompressorDsp::new(SR, &params);
    let mut l = poison(BLOCK);
    let mut r = poison(BLOCK);
    healed.process_stereo(&mut l, &mut r, None, &params, &viz);
    healed.reset();
    let after = render(&mut healed, &viz);

    assert_eq!(clean.len(), after.len());
    for (i, (a, b)) in clean.iter().zip(&after).enumerate() {
        assert_eq!(
            a.to_bits(),
            b.to_bits(),
            "sample {i} differs after poison+reset: {a:?} vs {b:?}"
        );
    }
}
