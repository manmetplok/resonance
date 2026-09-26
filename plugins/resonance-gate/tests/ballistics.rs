//! `attack` opens the gate, `release` closes it (ba todo #1343).
//!
//! The ballistics are the compressor's and run on the GAIN-REDUCTION
//! envelope, where reduction RISES as the gate CLOSES. Used as named that
//! makes `attack` the closing ramp and `release` the opening one — the
//! inverse of what every gate ever built means, and of what this crate's
//! knob labels, param ids and docs say. `gate_ballistics` crosses the two
//! coefficients to fix it; this file is what stops them drifting back.
//!
//! # How the gain is measured
//!
//! The gate multiplies both channels by one scalar per sample, so with a
//! non-zero input the applied gain is recoverable exactly as
//! `out[i] / in[i]` — no envelope follower, no windowing, no tolerance
//! introduced by the measurement itself. Every assertion below is on that
//! gain curve.
//!
//! The probe signals are deliberately never silent: silence would make
//! the division undefined at exactly the moment the closing ramp is most
//! interesting. The "closed" probe is a tone well under the close
//! threshold instead.

use resonance_gate::dsp::{GateDsp, GateSettings};

const SR: f32 = 48_000.0;

/// Base settings: a hard gate with a wide range, no hold and no key
/// filter, so the only things shaping the gain curve are the two
/// ballistics times under test.
fn settings(attack_ms: f32, release_ms: f32) -> GateSettings {
    GateSettings {
        threshold_db: -30.0,
        ratio: 20.0,
        attack_ms,
        hold_ms: 0.0,
        release_ms,
        range_db: 60.0,
        hysteresis_db: 6.0,
        key_hpf_hz: 0.0,
    }
}

/// A 1 kHz sine at `amp`, never exactly zero at the sample points that
/// matter — `sin` is only zero on exact multiples of the period, and the
/// gain readings guard on magnitude anyway.
fn tone(amp: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| amp * (std::f32::consts::TAU * 1000.0 * i as f32 / SR).sin())
        .collect()
}

fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Run a signal through a gate and return the per-sample applied gain.
/// `None` where the input was too small to divide by.
fn gain_curve(dsp: &mut GateDsp, input: &[f32], s: &GateSettings) -> Vec<Option<f32>> {
    let mut l = input.to_vec();
    let mut r = input.to_vec();
    dsp.process_block(&mut l, &mut r, None, input.len(), s);
    input
        .iter()
        .zip(&l)
        .map(|(x, y)| (x.abs() > 1e-4).then(|| y / x))
        .collect()
}

fn ms(samples: usize) -> f32 {
    samples as f32 * 1000.0 / SR
}

/// Both ramps are one-pole steps toward a target `range_db` away, so the
/// time to traverse a fixed fraction is a fixed multiple of the time
/// constant — which makes the expected millisecond figures closed-form
/// rather than something to eyeball.
///
/// OPENING runs the gain reduction from `range_db` (60) down to 0, and
/// the 90 %-open criterion is `gain >= 0.9`, i.e. 0.915 dB of remaining
/// reduction: `t = attack_ms * ln(60 / 0.915)`.
const OPEN_TIME_CONSTANTS: f32 = 4.185;
/// CLOSING runs it from 0 up toward 60 and is measured to `gain <= 0.1`,
/// i.e. 20 dB of reduction: `t = release_ms * ln(60 / (60 - 20))`.
const CLOSE_TIME_CONSTANTS: f32 = 0.405;

/// Assert a measured ramp time against its predicted value. The 15 %
/// band absorbs the one-sample quantisation of the crossing index and
/// the detector's brief head start, not a wiring error — an inverted or
/// cross-wired coefficient is out by 20x or more, never by 15 %.
fn assert_ramp(measured_ms: f32, predicted_ms: f32, what: &str) {
    let ratio = measured_ms / predicted_ms;
    assert!(
        (0.85..1.15).contains(&ratio),
        "{what}: measured {measured_ms:.2}ms, predicted {predicted_ms:.2}ms \
         ({ratio:.2}x). The ramp is not being driven by the time it is \
         named after."
    );
}

/// First sample index whose measured gain is at or above `level`.
fn first_at_or_above(curve: &[Option<f32>], level: f32) -> Option<usize> {
    curve
        .iter()
        .position(|g| g.is_some_and(|g| g >= level))
}

/// First sample index whose measured gain is at or below `level`.
fn first_at_or_below(curve: &[Option<f32>], level: f32) -> Option<usize> {
    curve
        .iter()
        .position(|g| g.is_some_and(|g| g <= level))
}

/// Milliseconds for a closed gate to open onto a full-scale tone, taken
/// as the time to reach 90 % of the input amplitude — the same criterion
/// the todo's measurements used.
fn open_ms(attack_ms: f32, release_ms: f32) -> f32 {
    let s = settings(attack_ms, release_ms);
    let mut dsp = GateDsp::new(SR);

    // Settle fully closed on a tone far below the close threshold.
    //
    // The pre-roll has to scale with `release`, because release is what
    // drives the CLOSING ramp: a fixed 0.5 s settle left a 1000 ms-release
    // gate only 39 % closed, so it then "opened" faster simply by having
    // less reduction to undo. That is a starting-condition artefact and it
    // made an earlier draft of `the_opening_ramp_ignores_release_...`
    // fail against correct code. Eight time constants puts the residual
    // under 0.04 % for any setting.
    let settle_ms = (release_ms * 8.0).max(500.0);
    let quiet = tone(db_to_lin(-70.0), (SR * settle_ms * 0.001) as usize);
    let _ = gain_curve(&mut dsp, &quiet, &s);

    let loud = tone(1.0, (SR * 1.5) as usize);
    let curve = gain_curve(&mut dsp, &loud, &s);
    let idx = first_at_or_above(&curve, 0.9)
        .unwrap_or_else(|| panic!("gate never reached 90 % open with attack={attack_ms}ms"));
    ms(idx)
}

/// Milliseconds for an open gate to close, measured from the moment the
/// gain starts moving rather than from the signal drop.
///
/// The detector has its own 15 ms release, so the threshold comparison
/// does not even notice the level drop for several milliseconds. That
/// delay is real but it is not the `release` ramp, and including it would
/// blur exactly the quantity under test.
fn close_ms(attack_ms: f32, release_ms: f32) -> f32 {
    let s = settings(attack_ms, release_ms);
    let mut dsp = GateDsp::new(SR);

    // Symmetrically to `open_ms`: the settle scales with `attack`, since
    // attack is what drives the OPENING ramp and the gate has to reach a
    // genuinely open state before the closing ramp can be timed from it.
    let settle_ms = (attack_ms * 8.0).max(500.0);
    let loud = tone(1.0, (SR * settle_ms * 0.001) as usize);
    let _ = gain_curve(&mut dsp, &loud, &s);

    // -50 dBFS is under the close threshold (-30 - 6 hysteresis = -36)
    // but still divisible, so the ramp stays measurable throughout.
    let quiet = tone(db_to_lin(-50.0), (SR * 3.0) as usize);
    let curve = gain_curve(&mut dsp, &quiet, &s);

    let start = first_at_or_below(&curve, 0.99)
        .unwrap_or_else(|| panic!("gate never began closing with release={release_ms}ms"));
    let end = first_at_or_below(&curve, 0.1)
        .unwrap_or_else(|| panic!("gate never closed with release={release_ms}ms"));
    assert!(end >= start, "close ramp ran backwards");
    ms(end - start)
}

#[test]
fn attack_drives_the_opening_ramp() {
    // The exact inversion of the todo's measurement. Before the fix:
    // attack 100 / release 1 opened in 3.7 ms, attack 1 / release 100
    // took 418 ms. Both must now be the other way round.
    let fast_attack = open_ms(1.0, 100.0);
    let slow_attack = open_ms(100.0, 1.0);

    assert_ramp(fast_attack, 1.0 * OPEN_TIME_CONSTANTS, "attack=1ms opening");
    assert_ramp(
        slow_attack,
        100.0 * OPEN_TIME_CONSTANTS,
        "attack=100ms opening",
    );
    assert!(
        slow_attack > fast_attack * 20.0,
        "opening time barely responded to a 100x attack change: \
         {fast_attack:.2}ms vs {slow_attack:.2}ms — is `attack` still \
         wired to the closing ramp?"
    );
}

#[test]
fn release_drives_the_closing_ramp() {
    let fast_release = close_ms(1.0, 5.0);
    let slow_release = close_ms(1.0, 500.0);

    assert_ramp(
        fast_release,
        5.0 * CLOSE_TIME_CONSTANTS,
        "release=5ms closing",
    );
    assert_ramp(
        slow_release,
        500.0 * CLOSE_TIME_CONSTANTS,
        "release=500ms closing",
    );
    assert!(
        slow_release > fast_release * 10.0,
        "closing time barely responded to a 100x release change: \
         {fast_release:.2}ms vs {slow_release:.2}ms — is `release` still \
         wired to the opening ramp?"
    );
}

#[test]
fn the_opening_ramp_ignores_release_and_the_closing_ramp_ignores_attack() {
    // The cross-check the two tests above cannot make on their own: each
    // control must move ITS ramp and leave the other alone. A swap that
    // wired both ramps to the same coefficient would satisfy both of the
    // tests above and fail here.
    let open_a = open_ms(2.0, 5.0);
    let open_b = open_ms(2.0, 1000.0);
    assert!(
        (open_a - open_b).abs() < 1.0,
        "changing release from 5ms to 1000ms moved the OPENING time \
         ({open_a:.2}ms -> {open_b:.2}ms); it must not"
    );

    let close_a = close_ms(0.5, 50.0);
    let close_b = close_ms(80.0, 50.0);
    assert!(
        (close_a - close_b).abs() < 2.0,
        "changing attack from 0.5ms to 80ms moved the CLOSING time \
         ({close_a:.2}ms -> {close_b:.2}ms); it must not"
    );
}

#[test]
fn the_opening_time_scales_with_the_attack_setting() {
    // A one-pole ramp traverses a fixed fraction in a fixed number of
    // time constants, so doubling `attack` must roughly double the time
    // to 90 % open. Pins the mapping as proportional rather than merely
    // monotonic — a coefficient that responded to attack in the wrong
    // direction, or barely at all, would still pass a monotonic check.
    let t10 = open_ms(10.0, 100.0);
    let t20 = open_ms(20.0, 100.0);
    let ratio = t20 / t10;
    assert!(
        (1.7..2.3).contains(&ratio),
        "doubling attack (10ms -> 20ms) changed the opening time by {ratio:.2}x, \
         expected ~2x ({t10:.2}ms -> {t20:.2}ms)"
    );
}

#[test]
fn the_default_patch_opens_promptly() {
    // The user-visible symptom that opened ba todo #1343: the shipped
    // default (attack 1 ms, release 100 ms) faded in over ~0.4 s, because
    // the 100 ms release was driving the opening ramp.
    let opened = open_ms(1.0, 100.0);
    assert!(
        opened < 15.0,
        "the default patch takes {opened:.1}ms to open — the #1343 \
         inversion is back"
    );
}

/// LIB-07: the attack knob reaches down to 0.05 ms, and the bottom half of
/// its lowest decade must not be dead — 0.05 ms has to open faster than
/// 0.1 ms, not snap to the same 0.1 ms coefficient.
#[test]
fn attack_below_a_tenth_of_a_millisecond_is_honoured() {
    let at_0_1 = open_ms(0.1, 100.0);
    let at_0_05 = open_ms(0.05, 100.0);
    assert!(at_0_1 > 0.0, "0.1 ms attack opened on the first sample");
    assert!(
        at_0_05 < at_0_1 * 0.75,
        "0.05 ms attack opened in {at_0_05:.3}ms vs {at_0_1:.3}ms at 0.1 ms"
    );
}
