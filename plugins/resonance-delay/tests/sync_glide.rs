//! Toggling tempo sync or changing the division must glide the read tap
//! (smoothed delay-in-samples) instead of relocating it discontinuously.

use resonance_delay::ResonanceDelay;
use resonance_plugin::{EventIterator, OutputBuffer, Param, ResonancePlugin, TempoInfo};

const SR: usize = 48_000;
const BLOCK: usize = 480;
const AMP: f32 = 0.5;
const FREQ: f32 = 221.25;

fn tempo() -> Option<TempoInfo> {
    Some(TempoInfo {
        bpm: 120.0,
        time_sig_num: 4,
        time_sig_den: 4,
        playing: true,
        song_pos_beats: 0.0,
    })
}

/// Buffers holding a sine burst over `[burst_start, burst_end)`, silence
/// elsewhere. The sine starts at phase 0 and gets a short fade-out, so
/// the recorded delay-line content is continuous at both burst edges —
/// any step the tap reads must then come from the tap itself jumping.
fn sine_burst(total: usize, burst_start: usize, burst_end: usize) -> (Vec<f32>, Vec<f32>) {
    let fade = SR / 200; // 5 ms
    let mut left = vec![0.0f32; total];
    for i in burst_start..burst_end {
        let t = (i - burst_start) as f32 / SR as f32;
        let gain = ((burst_end - i) as f32 / fade as f32).min(1.0);
        left[i] = AMP * gain * (FREQ * t * std::f32::consts::TAU).sin();
    }
    let right = left.clone();
    (left, right)
}

fn run(plugin: &mut ResonanceDelay, left: &mut [f32], right: &mut [f32], from: usize, to: usize) {
    let mut pos = from;
    while pos < to {
        let n = (to - pos).min(BLOCK);
        let mut outs = [OutputBuffer {
            left: &mut left[pos..pos + n],
            right: &mut right[pos..pos + n],
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, n, &mut ev, tempo());
        pos += n;
    }
}

fn max_abs_step(samples: &[f32]) -> f32 {
    samples
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

// Max sample-to-sample step of the wet sine when the tap glides: the
// 100 ms ramp from 4800 to 24000 samples sweeps the read position at up
// to 3x real time, so the bound is ~3 * 2π * FREQ / SR * AMP ≈ 0.044.
// An unsmoothed jump relocates the tap from mid-sine to silence in one
// sample (step ≈ 0.5), so 0.08 separates the two with margin.
const STEP_LIMIT: f32 = 0.08;

#[test]
fn sync_toggle_glides_delay_tap() {
    let mut plugin = ResonanceDelay::new();
    plugin.params.sync.set_plain(0.0); // free, 100 ms
    plugin.params.division.set_value(4); // 1/4 @ 120 BPM = 500 ms once synced
    plugin.params.time_ms.set_value(100.0);
    plugin.params.feedback.set_value(0.0);
    plugin.params.mix.set_value(1.0);
    plugin.params.mod_depth.set_value(0.0);
    plugin.initialize(SR as f32, BLOCK as u32);

    let total = 2 * SR;
    let toggle_at = SR;
    // Sine occupies the last 300 ms before the toggle, so the old tap
    // (100 ms) reads loud sine while the new tap (500 ms) reads silence.
    let (mut left, mut right) = sine_burst(total, toggle_at - 3 * SR / 10, toggle_at);

    run(&mut plugin, &mut left, &mut right, 0, toggle_at);
    plugin.params.sync.set_plain(1.0);
    run(&mut plugin, &mut left, &mut right, toggle_at, total);

    let step = max_abs_step(&left[toggle_at - 1..toggle_at + SR / 2]);
    assert!(
        step < STEP_LIMIT,
        "sync toggle produced a discontinuity: max step {step}"
    );
    // Sanity: the glide actually emitted signal (tap swept the burst).
    let energy: f32 = left[toggle_at..toggle_at + SR / 2]
        .iter()
        .map(|x| x.abs())
        .sum();
    assert!(energy > 1.0, "expected wet signal during glide, got {energy}");
}

#[test]
fn division_change_glides_delay_tap() {
    let mut plugin = ResonanceDelay::new();
    plugin.params.sync.set_plain(1.0);
    plugin.params.division.set_value(7); // 1/8 @ 120 BPM = 250 ms
    plugin.params.feedback.set_value(0.0);
    plugin.params.mix.set_value(1.0);
    plugin.params.mod_depth.set_value(0.0);
    plugin.initialize(SR as f32, BLOCK as u32);

    let total = 2 * SR;
    let toggle_at = SR;
    // Sine occupies the last 500 ms before the change, so the old tap
    // (250 ms) reads mid-sine while the new tap (500 ms) lands at the
    // burst onset.
    let (mut left, mut right) = sine_burst(total, toggle_at - SR / 2, toggle_at);

    run(&mut plugin, &mut left, &mut right, 0, toggle_at);
    plugin.params.division.set_value(4); // 1/4 = 500 ms
    run(&mut plugin, &mut left, &mut right, toggle_at, total);

    let step = max_abs_step(&left[toggle_at - 1..toggle_at + SR / 2]);
    assert!(
        step < STEP_LIMIT,
        "division change produced a discontinuity: max step {step}"
    );
}
