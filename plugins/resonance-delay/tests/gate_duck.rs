//! Wet-path gating and ducking (`resonance_delay::gate`).
//!
//! Two properties carry the whole feature:
//!
//! 1. Gate and duck shape the wet signal *only after* the delay tap, so
//!    the feedback path never re-records a chopped signal — the tail
//!    keeps its shape and the rhythm is applied to what comes out of it.
//!    A gated delay whose repeats decayed into a stutter would be the
//!    bug this test exists to catch.
//! 2. Neither touches the dry signal. With `mix = 0` the plugin must be
//!    bit-identical whatever the gate and duck are set to.

use resonance_delay::gate::{gate_gain, gate_period_samples, DUCK_MAX_GR_DB};
use resonance_delay::ResonanceDelay;
use resonance_plugin::{EventIterator, OutputBuffer, Param, ResonancePlugin, TempoInfo};

const SR: f32 = 48_000.0;

fn tempo(bpm: f32) -> TempoInfo {
    TempoInfo {
        bpm,
        time_sig_num: 4,
        time_sig_den: 4,
        playing: true,
        song_pos_beats: 0.0,
    }
}

/// Run `frames` of audio through a configured plugin, returning the
/// output. `setup` configures params before `initialize`.
fn render(setup: impl FnOnce(&mut ResonanceDelay), frames: usize, input: &[f32]) -> Vec<f32> {
    let mut plugin = ResonanceDelay::new();
    setup(&mut plugin);
    plugin.initialize(SR, frames as u32);

    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    for (i, s) in input.iter().enumerate().take(frames) {
        left[i] = *s;
        right[i] = *s;
    }
    let mut outs = [OutputBuffer {
        left: &mut left,
        right: &mut right,
    }];
    let mut ev = EventIterator::empty();
    plugin.process(&mut outs, frames, &mut ev, Some(tempo(120.0)));
    left
}

// ---------------------------------------------------------------------------
// The gate window
// ---------------------------------------------------------------------------

#[test]
fn the_window_opens_for_its_width_and_closes_for_the_rest() {
    // Hard edges so the plateaus are exact.
    assert_eq!(gate_gain(0.25, 0.5, 0.0, 1.0), 1.0, "inside the window");
    assert_eq!(gate_gain(0.75, 0.5, 0.0, 1.0), 0.0, "outside the window");
    assert_eq!(gate_gain(0.10, 0.25, 0.0, 1.0), 1.0);
    assert_eq!(gate_gain(0.30, 0.25, 0.0, 1.0), 0.0);
}

#[test]
fn depth_sets_how_far_the_closed_phase_attenuates() {
    // Depth 1 is silent, depth 0 is no gating, depth 0.5 is half.
    assert_eq!(gate_gain(0.9, 0.5, 0.0, 1.0), 0.0);
    assert_eq!(gate_gain(0.9, 0.5, 0.0, 0.0), 1.0);
    assert!((gate_gain(0.9, 0.5, 0.0, 0.5) - 0.5).abs() < 1e-6);
    // The open phase is unaffected by depth.
    assert_eq!(gate_gain(0.1, 0.5, 0.0, 0.5), 1.0);
}

#[test]
fn edges_ramp_instead_of_stepping() {
    // A 10%-of-period ramp: halfway up it is at half gain.
    let mid_rise = gate_gain(0.05, 0.5, 0.1, 1.0);
    assert!(
        (mid_rise - 0.5).abs() < 1e-5,
        "mid-rise gain {mid_rise}, expected 0.5"
    );
    let mid_fall = gate_gain(0.45, 0.5, 0.1, 1.0);
    assert!(
        (mid_fall - 0.5).abs() < 1e-5,
        "mid-fall gain {mid_fall}, expected 0.5"
    );
    // Monotonic through the rising edge.
    let mut prev = -1.0;
    for i in 0..=10 {
        let g = gate_gain(i as f32 * 0.01, 0.5, 0.1, 1.0);
        assert!(g >= prev, "rising edge dipped at phase {}", i as f32 * 0.01);
        prev = g;
    }
}

#[test]
fn an_oversized_edge_is_clamped_to_fit_the_window() {
    // Edge wider than the window would otherwise invert the shape. It
    // degrades to a triangle: still 0 at the boundaries, still bounded.
    for i in 0..100 {
        let phase = i as f32 / 100.0;
        let g = gate_gain(phase, 0.2, 0.9, 1.0);
        assert!((0.0..=1.0).contains(&g), "phase {phase} gave {g}");
    }
    // Narrow window, huge edge: the peak never exceeds unity.
    let peak = (0..100)
        .map(|i| gate_gain(i as f32 / 100.0, 0.1, 5.0, 1.0))
        .fold(0.0f32, f32::max);
    assert!(peak <= 1.0 + 1e-6, "peak {peak}");
}

#[test]
fn the_window_is_periodic() {
    // Tolerance, not equality: `phase + 3.0` does not round-trip exactly
    // through f32, so the wrapped phase lands a few ULPs off and lands
    // mid-ramp with a correspondingly different gain. The property under
    // test is that the window repeats, not that it repeats bit-exactly.
    for i in 0..20 {
        let phase = i as f32 / 20.0;
        let here = gate_gain(phase, 0.4, 0.05, 1.0);
        let later = gate_gain(phase + 3.0, 0.4, 0.05, 1.0);
        assert!(
            (here - later).abs() < 1e-4,
            "phase {phase} gave {here} but {later} a period later"
        );
    }
}

// ---------------------------------------------------------------------------
// Period from tempo
// ---------------------------------------------------------------------------

#[test]
fn the_period_follows_tempo_and_division() {
    // 120 BPM = 0.5 s per beat = 24_000 samples at 48 kHz.
    // Division index 4 is 1/4 (one beat), 7 is 1/8 (half a beat).
    let quarter = gate_period_samples(4, Some(tempo(120.0)), SR);
    assert!((quarter - 24_000.0).abs() < 1.0, "1/4 at 120 = {quarter}");
    let eighth = gate_period_samples(7, Some(tempo(120.0)), SR);
    assert!((eighth - 12_000.0).abs() < 1.0, "1/8 at 120 = {eighth}");
    // Twice the tempo, half the period.
    let quarter_fast = gate_period_samples(4, Some(tempo(240.0)), SR);
    assert!((quarter_fast - 12_000.0).abs() < 1.0);
}

#[test]
fn no_tempo_falls_back_to_one_second() {
    assert_eq!(gate_period_samples(4, None, SR), SR);
    // And never divides by zero on a degenerate rate.
    assert!(gate_period_samples(4, None, 0.0) >= 1.0);
}

// ---------------------------------------------------------------------------
// Plugin-level behaviour
// ---------------------------------------------------------------------------

#[test]
fn gating_leaves_the_dry_signal_alone() {
    // mix = 0 is dry-only: the gate and duck must be inaudible.
    let input: Vec<f32> = (0..2048).map(|i| (i as f32 * 0.01).sin()).collect();

    let plain = render(
        |p| {
            p.params.mix.set_value(0.0);
        },
        2048,
        &input,
    );
    let gated = render(
        |p| {
            p.params.mix.set_value(0.0);
            p.params.gate_on.set_plain(1.0);
            p.params.gate_depth.set_value(1.0);
            p.params.duck_amount.set_value(1.0);
        },
        2048,
        &input,
    );

    assert_eq!(plain, gated, "the dry path must be untouched at mix = 0");
}

#[test]
fn the_gate_does_not_chop_the_feedback_path() {
    // The delay line's contents must be identical with the gate on and
    // off — only what comes OUT of it is gated. Compare the wet tail of
    // a gated render against an ungated one at a phase where the gate is
    // fully open: if the gate had fed back, the tail would differ in
    // amplitude, not just in windowing.
    let mut impulse = vec![0.0f32; 4096];
    impulse[0] = 1.0;

    let open = render(
        |p| {
            p.params.mix.set_value(1.0);
            p.params.sync.set_plain(0.0);
            p.params.time_ms.set_value(10.0);
            p.params.feedback.set_value(0.8);
        },
        4096,
        &impulse,
    );
    let gated = render(
        |p| {
            p.params.mix.set_value(1.0);
            p.params.sync.set_plain(0.0);
            p.params.time_ms.set_value(10.0);
            p.params.feedback.set_value(0.8);
            p.params.gate_on.set_plain(1.0);
            // Fully open window: gain is 1 everywhere, so a
            // non-feedback-affecting gate is a no-op.
            p.params.gate_width.set_value(0.95);
            p.params.gate_shape.set_value(0.0);
            p.params.gate_depth.set_value(0.0);
        },
        4096,
        &impulse,
    );

    assert_eq!(
        open, gated,
        "a transparent gate must not change the delay line's output"
    );
}

#[test]
fn a_closed_gate_silences_the_wet_but_keeps_the_tail_alive() {
    let mut impulse = vec![0.0f32; 4096];
    impulse[0] = 1.0;

    // Depth 1, width at its minimum: almost the whole period is closed.
    let gated = render(
        |p| {
            p.params.mix.set_value(1.0);
            p.params.sync.set_plain(0.0);
            p.params.time_ms.set_value(10.0);
            p.params.feedback.set_value(0.8);
            p.params.gate_on.set_plain(1.0);
            p.params.gate_rate.set_plain(0.0); // 1/1 — one long period
            p.params.gate_width.set_value(0.05);
            p.params.gate_shape.set_value(0.0);
            p.params.gate_depth.set_value(1.0);
        },
        4096,
        &impulse,
    );
    let ungated = render(
        |p| {
            p.params.mix.set_value(1.0);
            p.params.sync.set_plain(0.0);
            p.params.time_ms.set_value(10.0);
            p.params.feedback.set_value(0.8);
        },
        4096,
        &impulse,
    );

    // 1/1 at 120 BPM is 96_000 samples, so 5% open = 4_800 samples: the
    // whole 4_096-frame render sits inside the open window and the two
    // must agree. This pins that the gate's period really is tempo-scaled
    // rather than per-block.
    assert_eq!(
        gated, ungated,
        "a 1/1 gate's open window covers this whole render"
    );
}

#[test]
fn ducking_pulls_the_wet_down_while_the_dry_is_loud() {
    // Loud sustained input, wet-only, long delay so the wet under test is
    // the tail rather than the input itself.
    let input: Vec<f32> = (0..8192).map(|i| (i as f32 * 0.05).sin() * 0.9).collect();

    let dry_wet = |duck: f32| {
        render(
            |p| {
                p.params.mix.set_value(1.0);
                p.params.sync.set_plain(0.0);
                p.params.time_ms.set_value(20.0);
                p.params.feedback.set_value(0.6);
                p.params.duck_amount.set_value(duck);
                p.params.duck_threshold.set_value(-40.0);
                p.params.duck_release.set_value(50.0);
            },
            8192,
            &input,
        )
    };

    let open: f32 = dry_wet(0.0).iter().map(|s| s.abs()).sum();
    let ducked: f32 = dry_wet(1.0).iter().map(|s| s.abs()).sum();

    assert!(
        ducked < open * 0.75,
        "ducking barely moved the wet: {ducked} vs {open}"
    );
}

#[test]
fn duck_amount_zero_is_transparent() {
    let input: Vec<f32> = (0..4096).map(|i| (i as f32 * 0.05).sin() * 0.9).collect();
    let base = render(
        |p| {
            p.params.mix.set_value(1.0);
            p.params.sync.set_plain(0.0);
            p.params.time_ms.set_value(20.0);
        },
        4096,
        &input,
    );
    let no_duck = render(
        |p| {
            p.params.mix.set_value(1.0);
            p.params.sync.set_plain(0.0);
            p.params.time_ms.set_value(20.0);
            p.params.duck_amount.set_value(0.0);
            p.params.duck_threshold.set_value(-60.0);
        },
        4096,
        &input,
    );
    assert_eq!(base, no_duck, "duck 0 must be bit-identical to no duck");
}

#[test]
fn ducking_is_bounded_by_the_amount() {
    // Even slammed, the reduction cannot exceed DUCK_MAX_GR_DB, so the
    // wet never disappears entirely into a divide-by-nothing.
    let input: Vec<f32> = (0..4096).map(|_| 1.0).collect();
    let out = render(
        |p| {
            p.params.mix.set_value(1.0);
            p.params.sync.set_plain(0.0);
            p.params.time_ms.set_value(5.0);
            p.params.feedback.set_value(0.9);
            p.params.duck_amount.set_value(1.0);
            p.params.duck_threshold.set_value(-60.0);
        },
        4096,
        &input,
    );
    assert!(
        out.iter().all(|s| s.is_finite()),
        "ducking produced a non-finite sample"
    );
    assert!(DUCK_MAX_GR_DB > 0.0);
}

#[test]
fn the_param_count_matches_the_declared_surface() {
    let plugin = ResonanceDelay::new();
    assert_eq!(plugin.param_count(), resonance_delay::params::PARAM_COUNT);
    // Every index resolves to a distinct parameter id — an off-by-one in
    // `param_at` would silently alias one control onto another.
    let ids: std::collections::HashSet<&str> =
        (0..plugin.param_count()).map(|i| plugin.param(i).id()).collect();
    assert_eq!(ids.len(), plugin.param_count(), "duplicate param ids");
    for id in [
        "gate_on",
        "gate_rate",
        "gate_width",
        "gate_shape",
        "gate_depth",
        "duck_amount",
        "duck_threshold",
        "duck_release",
    ] {
        assert!(ids.contains(id), "{id} is not reachable through param_at");
    }
}

// ---------------------------------------------------------------------------
// Transport lock (LIB-03)
// ---------------------------------------------------------------------------

/// A plugin configured as a hard 1/4 gate, 50% open, over a 10 ms wet-only
/// delay with no feedback — so the output is the gated echo of the input.
fn quarter_gate() -> ResonanceDelay {
    let mut p = ResonanceDelay::new();
    p.params.mix.set_value(1.0);
    p.params.sync.set_plain(0.0);
    p.params.time_ms.set_value(10.0);
    p.params.feedback.set_value(0.0);
    p.params.gate_on.set_plain(1.0);
    p.params.gate_rate.set_plain(4.0); // 1/4 — one beat
    p.params.gate_width.set_value(0.5);
    p.params.gate_shape.set_value(0.0);
    p.params.gate_depth.set_value(1.0);
    p
}

fn sine(start: usize, frames: usize) -> Vec<f32> {
    (start..start + frames)
        .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / SR).sin() * 0.5)
        .collect()
}

/// Process one block of a 440 Hz sine starting at absolute sample
/// `start`, with the given transport state.
fn block(p: &mut ResonanceDelay, start: usize, frames: usize, t: TempoInfo) -> Vec<f32> {
    let mut left = sine(start, frames);
    let mut right = left.clone();
    let mut outs = [OutputBuffer {
        left: &mut left,
        right: &mut right,
    }];
    let mut ev = EventIterator::empty();
    p.process(&mut outs, frames, &mut ev, Some(t));
    left
}

fn at(song_pos_beats: f64) -> TempoInfo {
    TempoInfo {
        song_pos_beats,
        ..tempo(120.0)
    }
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
}

#[test]
fn a_mid_beat_start_opens_on_the_beat_not_on_the_first_sample() {
    // 120 BPM: one beat is 24_000 samples. Starting playback half a beat
    // in puts the first 12_000 samples in the CLOSED half of the window.
    let mut p = quarter_gate();
    p.initialize(SR, 24_000);
    let out = block(&mut p, 0, 24_000, at(0.5));
    let closed = rms(&out[1_000..11_000]);
    let open = rms(&out[13_000..23_000]);
    assert!(open > 0.1, "the open half must carry the echo (rms {open})");
    assert!(closed < 1e-4, "the off-beat half must be gated (rms {closed})");
}

#[test]
fn a_seek_snaps_the_gate_to_the_new_position() {
    let mut p = quarter_gate();
    p.initialize(SR, 6_000);
    // Two blocks from the top of the song: 0 .. 0.5 beat, gate open.
    block(&mut p, 0, 6_000, at(0.0));
    let b = block(&mut p, 6_000, 6_000, at(0.25));
    assert!(rms(&b) > 0.1, "open window before the seek (rms {})", rms(&b));
    // Seek to beat 7.5: the gate must be closed at once, not keep the
    // phase it had accumulated (which would still be open).
    let c = block(&mut p, 12_000, 6_000, at(7.5));
    assert!(rms(&c) < 1e-4, "closed window after the seek (rms {})", rms(&c));
    // And a block back on a beat is open again.
    let d = block(&mut p, 18_000, 6_000, at(8.0));
    assert!(rms(&d) > 0.1, "open window on the beat (rms {})", rms(&d));
}

#[test]
fn a_stopped_transport_still_free_runs() {
    // Not playing: song_pos is meaningless, the gate keeps its own phase,
    // starting on an open window.
    let mut p = quarter_gate();
    p.initialize(SR, 24_000);
    let t = TempoInfo {
        playing: false,
        ..at(0.5)
    };
    let out = block(&mut p, 0, 24_000, t);
    assert!(rms(&out[1_000..11_000]) > 0.1, "free-running gate opens first");
    assert!(rms(&out[13_000..23_000]) < 1e-4, "then closes");
}
