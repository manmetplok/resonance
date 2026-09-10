//! Loop-gain contract of the feedback path: the in-loop saturator is
//! normalized to unity small-signal gain (`tanh(x·d)/d`), so the
//! feedback knob reads as the true loop gain — 0.7 *decays* by ~0.7 per
//! repeat — and Freeze bypasses the shaper entirely, so a frozen buffer
//! *holds* instead of growing into a saturated drone. Before the
//! normalization, effective loop gain was `feedback × (1 + 3·drive)`,
//! which self-oscillated from feedback ≈ 0.7 upward at the default
//! drive.

use resonance_delay::ResonanceDelay;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};

const SR: f32 = 48_000.0;
/// 20 ms free-running delay: one loop period is exactly 960 samples at
/// 48 kHz, so output windows of 960 samples line up with the repeats.
const PERIOD: usize = 960;

/// Free-running, wet-only, filters wide open, modulation off: the loop
/// gain is the only thing shaping the level from repeat to repeat.
/// Drive stays at its non-zero factory default on purpose — the defect
/// was drive leaking into the loop *gain*, and these tests must fail if
/// it ever does again.
fn wet_only_plugin(feedback: f32) -> ResonanceDelay {
    let plugin = ResonanceDelay::new();
    plugin.params.sync.set_value(false);
    plugin.params.time_ms.set_value(PERIOD as f32 / SR * 1000.0);
    plugin.params.feedback.set_value(feedback);
    plugin.params.mix.set_value(1.0);
    plugin.params.hi_cut.set_value(20000.0);
    plugin.params.lo_cut.set_value(20.0);
    plugin.params.mod_depth.set_value(0.0);
    plugin
}

/// Run one loop-period-sized block through the plugin in place.
fn process_period(plugin: &mut ResonanceDelay, left: &mut [f32; PERIOD], right: &mut [f32; PERIOD]) {
    let mut outs = [OutputBuffer {
        left: &mut left[..],
        right: &mut right[..],
    }];
    let mut ev = EventIterator::empty();
    plugin.process(&mut outs, PERIOD, &mut ev, None);
}

fn rms(buf: &[f32]) -> f32 {
    (buf.iter().map(|x| x * x).sum::<f32>() / buf.len() as f32).sqrt()
}

/// One full-scale impulse at feedback 0.7: every repeat's peak must be
/// at most 0.75× the previous one. With the broken loop gain
/// (0.7 × 1.45 at default drive) the ratio sat near 1 and the tail
/// grew toward the tanh ceiling instead.
#[test]
fn feedback_0_7_decays_per_repeat() {
    let mut plugin = wet_only_plugin(0.7);
    plugin.initialize(SR, PERIOD as u32);

    let mut left = [0.0f32; PERIOD];
    let mut right = [0.0f32; PERIOD];
    let mut peaks = Vec::new();
    for period in 0..8 {
        left.fill(0.0);
        right.fill(0.0);
        if period == 0 {
            left[0] = 1.0;
            right[0] = 1.0;
        }
        process_period(&mut plugin, &mut left, &mut right);
        peaks.push(left.iter().fold(0.0f32, |m, x| m.max(x.abs())));
    }

    // Period 0 taps an empty line; period 1 is the impulse itself, so
    // the loop gain shows up from period 2 onward.
    assert!(peaks[1] > 0.5, "first repeat missing: peaks {peaks:?}");
    for k in 1..peaks.len() - 1 {
        assert!(
            peaks[k + 1] <= 0.75 * peaks[k],
            "repeat {} did not decay: {} -> {} (peaks {peaks:?})",
            k + 1,
            peaks[k],
            peaks[k + 1],
        );
    }
}

/// Freeze must *hold* the captured buffer: after 50 loop periods the
/// RMS stays within [0.9, 1.0]× of the first frozen period — the only
/// per-pass loss is the (wide-open here) tone filters, which stay in
/// the frozen loop by design. Before the fix the frozen loop grew to a
/// ~0.9-amplitude saturated drone (the fixed point of tanh(1.45·x)=x).
#[test]
fn freeze_holds_captured_buffer() {
    let mut plugin = wet_only_plugin(0.7);
    plugin.initialize(SR, PERIOD as u32);

    let mut left = [0.0f32; PERIOD];
    let mut right = [0.0f32; PERIOD];

    // Capture: three periods of a 200 Hz tone (240 samples per cycle,
    // so each period holds exactly four whole cycles).
    let mut n = 0usize;
    for _ in 0..3 {
        for i in 0..PERIOD {
            let t = (n + i) as f32 / SR;
            let v = 0.5 * (200.0 * t * std::f32::consts::TAU).sin();
            left[i] = v;
            right[i] = v;
        }
        n += PERIOD;
        process_period(&mut plugin, &mut left, &mut right);
    }

    plugin.params.freeze.set_value(true);

    let mut frozen_rms = Vec::new();
    for _ in 0..50 {
        left.fill(0.0);
        right.fill(0.0);
        process_period(&mut plugin, &mut left, &mut right);
        frozen_rms.push(rms(&left));
    }

    let captured = frozen_rms[0];
    let held = *frozen_rms.last().unwrap();
    assert!(captured > 0.1, "freeze captured silence: {captured}");
    assert!(
        held >= 0.9 * captured && held <= 1.001 * captured,
        "freeze did not hold: captured RMS {captured}, after 50 periods {held}"
    );
    // Not growing at any point along the way, either.
    for (k, w) in frozen_rms.windows(2).enumerate() {
        assert!(
            w[1] <= w[0] * 1.001,
            "frozen loop grew at period {}: {} -> {}",
            k + 1,
            w[0],
            w[1],
        );
    }
}

/// Small-signal transparency: at tiny levels the saturator must be a
/// straight wire, so the repeat-to-repeat gain equals the feedback
/// amount within 2%. A narrowband burst is the probe — an impulse's
/// *peak* is reshaped by the in-loop filters, which would measure the
/// filters, not the loop gain.
#[test]
fn small_signal_loop_gain_equals_feedback() {
    let feedback = 0.7;
    let mut plugin = wet_only_plugin(feedback);
    plugin.initialize(SR, PERIOD as u32);

    let mut left = [0.0f32; PERIOD];
    let mut right = [0.0f32; PERIOD];

    // One period of a 1e-3 amplitude 200 Hz tone, then silence.
    let mut window_rms = Vec::new();
    for period in 0..4 {
        for i in 0..PERIOD {
            left[i] = if period == 0 {
                let t = i as f32 / SR;
                1e-3 * (200.0 * t * std::f32::consts::TAU).sin()
            } else {
                0.0
            };
            right[i] = left[i];
        }
        process_period(&mut plugin, &mut left, &mut right);
        window_rms.push(rms(&left));
    }

    // Period 1 is the burst played back dry; period 2 is its first pass
    // through the saturated feedback loop.
    let first = window_rms[1];
    let second = window_rms[2];
    assert!(first > 1e-4, "burst repeat missing: {window_rms:?}");
    let gain = second / first;
    assert!(
        (gain - feedback).abs() <= 0.02 * feedback,
        "small-signal loop gain {gain} is not feedback {feedback} ±2% \
         (window RMS {window_rms:?})"
    );
}
