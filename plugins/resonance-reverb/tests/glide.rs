//! Parameter-motion guards: sweeping Size, ER-time or Pre-delay must
//! not click.
//!
//! Before the tap-glide/crossfade fix, `set_size`, `recompute_scaled`
//! and `set_predelay` rewrote integer tap lengths once per block, so the
//! read taps *relocated* discontinuously and a sweep over sustained
//! material crackled. The lib.rs smoothers only made the jumps smaller
//! and more frequent — the discontinuity itself remained.
//!
//! The guard here is a ratio: render a sustained sine through the wet
//! path twice, once with the parameter held and once swept across its
//! range, and compare the largest sample-to-sample output delta of the
//! two runs. A click is a jump unrelated to the signal's own slope, so
//! on the unfixed code the swept run's max delta is one to two orders of
//! magnitude above the steady run's; with continuous tap movement it
//! stays within a small factor (Doppler on the FDN glide raises the
//! instantaneous frequency by a bounded amount, and the ER/pre-delay
//! crossfades blend two correlated copies without any jump).
//!
//! Also here: the pre-delay cap must follow the actual sample rate
//! (chain.rs used to hardcode 48 000 samples — "1 second" only at
//! 48 kHz), and a static-parameter render must stay finite with a real
//! tail (the bit-exact steady-state guarantee itself is pinned by
//! `tests/dsp_golden.rs`).

use std::f32::consts::TAU;

use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};
use resonance_reverb::dsp::ReverbDsp;
use resonance_reverb::params::ReverbParams;
use resonance_reverb::ResonanceReverb;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;
/// Blocks rendered before measurement starts — fills the tail so the
/// measured window compares tails, not onsets. ~0.8 s.
const WARMUP_BLOCKS: usize = 150;
/// Measured blocks; the sweep crosses the parameter range over these.
/// ~2 s of audio.
const MEASURE_BLOCKS: usize = 375;

fn sine(n: u64) -> f32 {
    0.5 * (TAU * 220.0 * n as f32 / SR).sin()
}

/// Fully wet, unmodulated: clicks in the tap handling have nowhere to
/// hide behind the dry path or the FDN modulation.
fn wet_defaults(p: &ReverbParams) {
    p.mix.set_value(1.0);
    p.mod_depth.set_value(0.0);
    p.decay.set_value(2.0);
    p.predelay.set_value(0.0);
}

/// ER-dominant variant so the early-reflection taps carry the output.
fn er_dominant(p: &ReverbParams) {
    wet_defaults(p);
    p.er_level.set_value(1.0);
    p.decay.set_value(0.5);
}

/// Render warmup + measurement and return the largest sample-to-sample
/// output delta seen anywhere in the measured window (both channels,
/// across block boundaries too). `edit` runs before every measured
/// block with `t` sweeping 0..1 across the window; it is also applied
/// once with `t = 0` before `initialize` so the warmup renders at the
/// sweep's start value.
fn max_delta(setup: fn(&ReverbParams), edit: fn(&ReverbParams, f32)) -> f32 {
    let mut plugin = ResonanceReverb::new();
    // Classic, pinned: a fresh instance runs Room (reverb-algorithms.md D2).
    plugin.params.algorithm.set_value(resonance_reverb::dsp::Algorithm::Classic as i32);
    setup(&plugin.params);
    edit(&plugin.params, 0.0);
    plugin.initialize(SR, BLOCK as u32);

    let mut left = vec![0.0f32; BLOCK];
    let mut right = vec![0.0f32; BLOCK];
    let mut n: u64 = 0;
    let mut prev = (0.0f32, 0.0f32);
    let mut max = 0.0f32;
    for block in 0..WARMUP_BLOCKS + MEASURE_BLOCKS {
        let measuring = block >= WARMUP_BLOCKS;
        if measuring {
            let t = (block - WARMUP_BLOCKS) as f32 / MEASURE_BLOCKS as f32;
            edit(&plugin.params, t);
        }
        for i in 0..BLOCK {
            let s = sine(n + i as u64);
            left[i] = s;
            right[i] = s;
        }
        {
            let mut outs = [OutputBuffer {
                left: &mut left[..],
                right: &mut right[..],
            }];
            let mut ev = EventIterator::empty();
            plugin.process(&mut outs, BLOCK, &mut ev, None);
        }
        n += BLOCK as u64;
        for i in 0..BLOCK {
            if measuring {
                let d = (left[i] - prev.0).abs().max((right[i] - prev.1).abs());
                max = max.max(d);
            }
            prev = (left[i], right[i]);
        }
    }
    assert!(max.is_finite(), "render produced non-finite deltas");
    assert!(max > 0.0, "render was silent — the guard would be vacuous");
    max
}

/// Swept-to-steady delta headroom. The FDN glide's bounded Doppler and
/// the level differences across the swept range cost a small factor
/// (measured 1.0-1.7x after the fix); the pre-fix per-block tap
/// relocations cost 4.8x (pre-delay) to 25x (size).
const CLICK_FACTOR: f32 = 3.0;

#[test]
fn size_sweep_stays_click_free() {
    let steady = max_delta(wet_defaults, |p, _| p.size.set_value(0.5));
    let swept = max_delta(wet_defaults, |p, t| p.size.set_value(0.05 + 0.9 * t));
    assert!(
        swept <= CLICK_FACTOR * steady,
        "size sweep clicks: swept max delta {swept:.5} vs steady {steady:.5} \
         (allowed factor {CLICK_FACTOR})"
    );
}

#[test]
fn er_time_sweep_stays_click_free() {
    let steady = max_delta(er_dominant, |p, _| p.er_time.set_value(0.5));
    let swept = max_delta(er_dominant, |p, t| p.er_time.set_value(t));
    assert!(
        swept <= CLICK_FACTOR * steady,
        "er_time sweep clicks: swept max delta {swept:.5} vs steady {steady:.5} \
         (allowed factor {CLICK_FACTOR})"
    );
}

#[test]
fn predelay_sweep_stays_click_free() {
    // ER-dominant setup: the long tail of `wet_defaults` is fed through
    // the FDN, whose recirculation smears a pre-delay jump until it
    // hides below the click factor. With a short decay and the ER at
    // full level the pre-delayed input carries the output directly.
    let steady = max_delta(er_dominant, |p, _| p.predelay.set_value(50.0));
    let swept = max_delta(er_dominant, |p, t| p.predelay.set_value(200.0 * t));
    assert!(
        swept <= CLICK_FACTOR * steady,
        "predelay sweep clicks: swept max delta {swept:.5} vs steady {steady:.5} \
         (allowed factor {CLICK_FACTOR})"
    );
}

/// Static parameters: the render must be finite with real energy in its
/// second half. Bit-exact steady-state equivalence with the pre-glide
/// code is pinned by `tests/dsp_golden.rs`; this is the cheap explicit
/// check that the glide machinery stays inert when nothing moves.
#[test]
fn static_params_render_finite_nonzero() {
    let mut plugin = ResonanceReverb::new();
    // Classic, pinned: a fresh instance runs Room (reverb-algorithms.md D2).
    plugin.params.algorithm.set_value(resonance_reverb::dsp::Algorithm::Classic as i32);
    plugin.params.mix.set_value(1.0);
    plugin.params.size.set_value(0.7);
    plugin.params.predelay.set_value(30.0);
    plugin.initialize(SR, BLOCK as u32);

    let mut left = vec![0.0f32; BLOCK];
    let mut right = vec![0.0f32; BLOCK];
    let mut n: u64 = 0;
    let mut second_half_peak = 0.0f32;
    let blocks = 200;
    for block in 0..blocks {
        for i in 0..BLOCK {
            let s = sine(n + i as u64);
            left[i] = s;
            right[i] = s;
        }
        {
            let mut outs = [OutputBuffer {
                left: &mut left[..],
                right: &mut right[..],
            }];
            let mut ev = EventIterator::empty();
            plugin.process(&mut outs, BLOCK, &mut ev, None);
        }
        n += BLOCK as u64;
        for i in 0..BLOCK {
            assert!(
                left[i].is_finite() && right[i].is_finite(),
                "non-finite sample in block {block}"
            );
            if block >= blocks / 2 {
                second_half_peak = second_half_peak.max(left[i].abs()).max(right[i].abs());
            }
        }
    }
    assert!(
        second_half_peak > 1e-3,
        "static render lost its energy ({second_half_peak:.3e})"
    );
}

/// First sample index with output above the noise floor for an impulse
/// fed through a fresh `ReverbDsp` at `sr` with the given pre-delay.
/// Everything downstream of the pre-delay is time-invariant here
/// (modulation depth defaults to 0), so the onset shifts by exactly the
/// pre-delay tap length.
fn onset_with_predelay(sr: f32, ms: f32) -> usize {
    let mut dsp = ReverbDsp::new(sr);
    // Classic, pinned: a fresh processor runs Room (reverb-algorithms.md D2).
    dsp.set_algorithm(resonance_reverb::dsp::Algorithm::Classic);
    dsp.set_predelay(ms);
    let total = sr as usize + 8_000;
    for n in 0..total {
        let x = if n == 0 { 1.0 } else { 0.0 };
        let (l, r) = dsp.process(x, x, 0.8, 1.0);
        if l.abs() > 1e-7 || r.abs() > 1e-7 {
            return n;
        }
    }
    panic!("no output within {total} samples at {sr} Hz / {ms} ms");
}

/// The documented pre-delay maximum is 1 second — at the *actual*
/// sample rate. chain.rs used to cap the tap at a hardcoded 48 000
/// samples, which silently halved the maximum at 96 kHz.
#[test]
fn predelay_cap_follows_sample_rate() {
    // 96 kHz: 1 s must delay by ~96 000 samples (the cap keeps one
    // sample of guard, exactly as the 48 kHz code always did).
    let shift_96k = onset_with_predelay(96_000.0, 1_000.0) - onset_with_predelay(96_000.0, 0.0);
    assert_eq!(
        shift_96k, 95_999,
        "1 s of pre-delay at 96 kHz delayed by {shift_96k} samples"
    );

    // 48 kHz behavior is unchanged: same cap the hardcoded constant gave.
    let shift_48k = onset_with_predelay(48_000.0, 1_000.0) - onset_with_predelay(48_000.0, 0.0);
    assert_eq!(
        shift_48k, 47_999,
        "1 s of pre-delay at 48 kHz delayed by {shift_48k} samples"
    );

    // And an in-range setting maps through the actual rate, not 48 k.
    let shift_100ms = onset_with_predelay(96_000.0, 100.0) - onset_with_predelay(96_000.0, 0.0);
    assert_eq!(
        shift_100ms, 9_600,
        "100 ms of pre-delay at 96 kHz delayed by {shift_100ms} samples"
    );
}
