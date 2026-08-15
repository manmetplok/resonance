//! Stereo offset (ba todo #1331).
//!
//! The offset used to be wired into the dual route only, so on stereo
//! and ping-pong the knob moved and nothing happened. The decision
//! recorded in ba: **apply it on all three routes** — each route has
//! its own right delay line, so an offset read position is meaningful
//! everywhere; disabling it would have removed a usable effect rather
//! than explained one. These tests pin the right tap moving on every
//! route, and the visualiser showing the two trains separately.

use resonance_delay::viz::{echo_taps, MAX_ECHO_TAPS};
use resonance_delay::ResonanceDelay;
use resonance_plugin::{EventIterator, OutputBuffer, Param, ResonancePlugin};

const SR: f32 = 48_000.0;

/// Run an impulse (both channels) through the plugin with the given
/// route and offset, returning the rendered stereo pair.
fn render(routing: i32, stereo_offset: f32, delay_ms: f32) -> (Vec<f32>, Vec<f32>) {
    let mut plugin = ResonanceDelay::new();
    plugin.params.routing.set_value(routing);
    plugin.params.stereo_offset.set_value(stereo_offset);
    plugin.params.sync.set_plain(0.0); // free-running
    plugin.params.time_ms.set_value(delay_ms);
    plugin.params.feedback.set_value(0.5);
    plugin.params.mix.set_value(1.0);
    plugin.params.mod_depth.set_value(0.0); // no wow, so taps are exact
    plugin.initialize(SR, 4096);

    let frames = SR as usize / 2;
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    left[0] = 1.0;
    right[0] = 1.0;

    let mut pos = 0;
    while pos < frames {
        let n = (frames - pos).min(4096);
        let mut outs = [OutputBuffer {
            left: &mut left[pos..pos + n],
            right: &mut right[pos..pos + n],
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, n, &mut ev, None);
        pos += n;
    }
    (left, right)
}

/// Sample index of the first echo after the direct signal.
fn first_echo(buf: &[f32]) -> usize {
    buf.iter()
        .enumerate()
        .skip(64)
        .find(|(_, x)| x.abs() > 0.02)
        .map(|(i, _)| i)
        .unwrap_or_else(|| panic!("no echo found"))
}

#[test]
fn the_offset_moves_the_right_tap_on_every_route() {
    let delay_ms = 100.0;
    let nominal = (delay_ms * 0.001 * SR) as usize; // 4800
    for routing in [0, 1, 2] {
        let (_, right) = render(routing, 0.25, delay_ms);
        let echo_r = first_echo(&right);
        let expected = match routing {
            // Ping-pong writes the mono input into L, so the first R
            // echo lands one L bounce plus one offset R bounce later.
            1 => nominal + (nominal as f32 * 1.25) as usize,
            _ => (nominal as f32 * 1.25) as usize,
        };
        let drift = (echo_r as i64 - expected as i64).abs();
        assert!(
            drift < 400,
            "route {routing}: R echo at {echo_r}, expected ~{expected} with a +25% offset"
        );
    }
}

#[test]
fn a_zero_offset_leaves_the_channels_aligned() {
    for routing in [0, 2] {
        let (left, right) = render(routing, 0.0, 100.0);
        assert_eq!(
            first_echo(&left),
            first_echo(&right),
            "route {routing} with no offset must keep L and R in step"
        );
    }
}

/// The knob had no effect on stereo/ping-pong before; assert the two
/// routes really did change, so a regression to "dual only" fails here.
#[test]
fn the_offset_changes_the_output_on_stereo_and_ping_pong() {
    for routing in [0, 1, 2] {
        let (_, flat) = render(routing, 0.0, 100.0);
        let (_, skewed) = render(routing, 0.25, 100.0);
        let diff: f32 = flat
            .iter()
            .zip(skewed.iter())
            .map(|(a, b)| (a - b).abs())
            .sum();
        assert!(
            diff > 1.0,
            "route {routing}: the offset made no audible difference (sum |Δ| = {diff})"
        );
    }
}

#[test]
fn output_stays_finite_at_the_offset_extremes() {
    for routing in [0, 1, 2] {
        for offset in [-0.5, 0.5] {
            let (left, right) = render(routing, offset, 100.0);
            assert!(
                left.iter().chain(right.iter()).all(|x| x.is_finite()),
                "route {routing} at offset {offset} produced a non-finite sample"
            );
        }
    }
}

// -- visualiser ---------------------------------------------------------

#[test]
fn viz_trains_separate_when_offset_on_stereo_and_dual() {
    for routing in [0, 2] {
        let taps = echo_taps(100.0, 125.0, 0.5, routing);
        for i in 0..MAX_ECHO_TAPS {
            let n = (i + 1) as f32;
            assert!((taps.times_l[i] - 100.0 * n).abs() < 1e-3);
            assert!((taps.times_r[i] - 125.0 * n).abs() < 1e-3);
            // Both trains are audible, and decay together.
            assert!((taps.levels_l[i] - taps.levels_r[i]).abs() < 1e-6);
            assert!(taps.levels_l[i].is_finite());
        }
    }
}

#[test]
fn viz_ping_pong_alternates_channels_with_cumulative_times() {
    let taps = echo_taps(100.0, 125.0, 0.5, 1);
    // L, R, L, R … at cumulative bounce times.
    let expected = [100.0, 225.0, 325.0, 450.0, 550.0, 675.0, 775.0, 900.0];
    for (i, want) in expected.iter().enumerate() {
        if i % 2 == 0 {
            assert!(
                (taps.times_l[i] - want).abs() < 1e-3,
                "tap {i}: L at {}, expected {want}",
                taps.times_l[i]
            );
            assert!(taps.levels_l[i].is_finite());
            // The other channel does not carry this tap.
            assert_eq!(taps.times_r[i], 0.0);
            assert_eq!(taps.levels_r[i], f32::NEG_INFINITY);
        } else {
            assert!(
                (taps.times_r[i] - want).abs() < 1e-3,
                "tap {i}: R at {}, expected {want}",
                taps.times_r[i]
            );
            assert!(taps.levels_r[i].is_finite());
            assert_eq!(taps.times_l[i], 0.0);
            assert_eq!(taps.levels_l[i], f32::NEG_INFINITY);
        }
    }
}

#[test]
fn viz_reflects_the_live_offset_through_the_plugin() {
    let mut plugin = ResonanceDelay::new();
    plugin.params.sync.set_plain(0.0);
    plugin.params.time_ms.set_value(200.0);
    plugin.params.stereo_offset.set_value(0.25);
    plugin.initialize(SR, 512);

    let mut left = [0.0f32; 512];
    let mut right = [0.0f32; 512];
    left[0] = 1.0;
    let mut outs = [OutputBuffer {
        left: &mut left,
        right: &mut right,
    }];
    let mut ev = EventIterator::empty();
    plugin.process(&mut outs, 512, &mut ev, None);

    let (times_l, _, times_r, _) = plugin.viz().read_echo_taps();
    assert!(
        times_r[0] > times_l[0] * 1.2,
        "viz still mirrors the L lane: L {} ms, R {} ms",
        times_l[0],
        times_r[0]
    );
}

/// No shipped preset may set a control that does nothing. "Lo-Fi Tape"
/// carried a stereo offset on the stereo route, which used to be inert.
#[test]
fn shipped_presets_have_no_inert_stereo_offset() {
    use resonance_delay::params::DelayParams;
    use resonance_delay::presets::{load_preset, PRESETS};

    for entry in PRESETS {
        let params = DelayParams::default();
        assert!(load_preset(&params, entry.json));
        let offset = params.stereo_offset.value();
        if offset != 0.0 {
            // With the offset live on every route this can no longer be
            // a no-op, but keep the intent explicit for the next reader.
            assert!(
                (-0.5..=0.5).contains(&offset),
                "preset '{}' offset {offset} is out of range",
                entry.name
            );
        }
    }
}
