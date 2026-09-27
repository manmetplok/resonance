//! Analog instability: `osc_phase_random` and `analog`.
//!
//! The defaults must be sound-neutral — `render_block_regression.rs` and
//! `null_test.rs` pin that bit-for-bit against their existing goldens, and
//! the tests here pin it for the paths those scenarios do not reach
//! (retriggering, and the knobs explicitly at 0 after being moved). The rest
//! prove each knob actually does what it says, within its stated bounds,
//! and reproducibly.

use resonance_plugin::{EventIterator, NoteEvent, Param};
use resonance_wavetable::dsp::analog::{
    AnalogRng, DriftCoeffs, DriftWalk, CUTOFF_SPREAD_OCT, DRIFT_MAX_CENTS,
};
use resonance_wavetable::dsp::engine::SynthEngine;
use resonance_wavetable::dsp::oscillator::midi_to_freq;
use resonance_wavetable::params::WavetableParams;
use resonance_wavetable::viz::WavetableVizState;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

fn engine() -> SynthEngine {
    let mut e = SynthEngine::new();
    e.initialize(SR);
    e
}

fn on(note: u8) -> NoteEvent {
    NoteEvent::NoteOn {
        note,
        velocity: 0.8,
        timing: 0,
    }
}

fn off(note: u8) -> NoteEvent {
    NoteEvent::NoteOff { note, timing: 0 }
}

fn render(engine: &mut SynthEngine, params: &WavetableParams, events: &[NoteEvent]) -> Vec<f32> {
    let mut left = vec![0.0f32; BLOCK];
    let mut right = vec![0.0f32; BLOCK];
    let mut iter = EventIterator::new(events);
    engine.render_block(&mut left, &mut right, BLOCK, params, &mut iter, None);
    left.extend_from_slice(&right);
    left
}

fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|s| s.to_bits()).collect()
}

/// A patch with both oscillators, unison and a moving filter, so every term
/// the analog knob touches (pitch, level, cutoff) is live.
fn rich_patch() -> WavetableParams {
    let p = WavetableParams::new();
    p.osc2.enabled.set_value(true);
    p.unison.voices.set_value(3);
    p.unison.detune.set_value(20.0);
    p.filter.cutoff.set_value(1500.0);
    p.filter.resonance.set_value(0.3);
    p.filter.env_depth.set_value(0.3);
    // Short releases so `drain` leaves a quiescent engine.
    p.amp_env.release.set_value(0.02);
    p.mod_env.release.set_value(0.02);
    p
}

/// Play a short sequence (overlapping notes, retriggers, releases) and return
/// the whole render.
fn play_sequence(engine: &mut SynthEngine, params: &WavetableParams) -> Vec<f32> {
    let script: [&[NoteEvent]; 12] = [
        &[on(60)],
        &[],
        &[on(64)],
        &[off(60)],
        &[on(67)],
        &[],
        &[off(64), on(60)],
        &[],
        &[off(67)],
        &[off(60)],
        &[],
        &[],
    ];
    let mut out = Vec::new();
    for events in script {
        out.extend(render(engine, params, events));
    }
    out
}

/// Render silence until the master-volume de-zipper (which starts at 0 on a
/// bare engine) has settled, so the first note is not rendered under a ramp
/// that a later one would not be.
fn settle(engine: &mut SynthEngine, params: &WavetableParams) {
    for _ in 0..4 {
        render(engine, params, &[]);
    }
}

/// Render a note until it has fully released, so the next note-on lands on a
/// quiescent engine.
fn drain(engine: &mut SynthEngine, params: &WavetableParams) {
    for _ in 0..40 {
        render(engine, params, &[]);
    }
    assert_eq!(engine.sounding_voices().count(), 0, "voice did not drain");
}

// ---------------------------------------------------------------------------
// Defaults are neutral
// ---------------------------------------------------------------------------

#[test]
fn both_knobs_default_to_zero() {
    let p = WavetableParams::new();
    assert_eq!(p.analog.phase_random.value(), 0.0);
    assert_eq!(p.analog.drift.value(), 0.0);
    assert_eq!(p.analog.phase_random.id(), "osc_phase_random");
    assert_eq!(p.analog.drift.id(), "analog");
}

#[test]
fn at_zero_a_knob_that_was_moved_renders_identically_to_one_never_touched() {
    // The analog draws happen on every note-on whatever the knobs say; only
    // the scaling depends on them. So an engine that played with the knobs
    // up, then had them returned to 0, must render the next notes exactly as
    // an engine that never saw them move.
    let params_a = rich_patch();
    let mut a = engine();
    play_sequence(&mut a, &params_a);
    drain(&mut a, &params_a);
    let reference = play_sequence(&mut a, &params_a);

    let params_b = rich_patch();
    params_b.analog.phase_random.set_value(1.0);
    params_b.analog.drift.set_value(1.0);
    let mut b = engine();
    let moved = play_sequence(&mut b, &params_b);
    drain(&mut b, &params_b);
    params_b.analog.phase_random.set_value(0.0);
    params_b.analog.drift.set_value(0.0);
    let again = play_sequence(&mut b, &params_b);

    assert!(reference.iter().any(|s| s.abs() > 1e-3));
    assert_ne!(bits(&moved), bits(&again), "the knobs had no effect while up");
    assert_eq!(bits(&reference), bits(&again));
}

#[test]
fn with_phase_random_at_zero_every_note_starts_identically() {
    let params = rich_patch();
    let mut e = engine();
    settle(&mut e, &params);
    let first = render(&mut e, &params, &[on(60)]);
    render(&mut e, &params, &[off(60)]);
    drain(&mut e, &params);
    let second = render(&mut e, &params, &[on(60)]);
    assert_eq!(
        bits(&first),
        bits(&second),
        "a retriggered note no longer starts at phase 0 with the knob at 0"
    );
}

#[test]
fn with_analog_at_zero_pitch_is_exactly_the_note() {
    let params = WavetableParams::new();
    let mut e = engine();
    render(&mut e, &params, &[on(69)]);
    for _ in 0..50 {
        render(&mut e, &params, &[]);
        for (_, hz) in e.sounding_osc1_freqs() {
            assert_eq!(hz.to_bits(), midi_to_freq(69.0).to_bits());
        }
    }
}

// ---------------------------------------------------------------------------
// Random start phase
// ---------------------------------------------------------------------------

#[test]
fn random_phase_makes_successive_notes_start_differently() {
    let params = rich_patch();
    params.analog.phase_random.set_value(1.0);
    let mut e = engine();
    settle(&mut e, &params);

    let first = render(&mut e, &params, &[on(60)]);
    render(&mut e, &params, &[off(60)]);
    drain(&mut e, &params);
    let second = render(&mut e, &params, &[on(60)]);

    assert!(first.iter().all(|s| s.is_finite()));
    assert!(second.iter().any(|s| s.abs() > 1e-3), "note rendered silence");
    assert_ne!(
        bits(&first),
        bits(&second),
        "random start phase produced two identical note onsets"
    );
}

#[test]
fn random_phase_alone_does_not_detune() {
    // Phase randomisation must not leak into pitch: with `analog` at 0 the
    // resolved frequency stays exactly the note's.
    let params = WavetableParams::new();
    params.analog.phase_random.set_value(1.0);
    let mut e = engine();
    render(&mut e, &params, &[on(69)]);
    render(&mut e, &params, &[]);
    for (_, hz) in e.sounding_osc1_freqs() {
        assert_eq!(hz.to_bits(), midi_to_freq(69.0).to_bits());
    }
}

// ---------------------------------------------------------------------------
// Drift
// ---------------------------------------------------------------------------

/// Cents between `hz` and the equal-tempered pitch of `note`.
fn cents(hz: f32, note: u8) -> f32 {
    1200.0 * (hz / midi_to_freq(note as f32)).log2()
}

#[test]
fn drift_moves_pitch_slowly_and_within_bounds() {
    let params = WavetableParams::new();
    params.analog.drift.set_value(1.0);
    params.unison.voices.set_value(3);
    params.unison.detune.set_value(0.0);
    let mut e = engine();
    render(&mut e, &params, &[on(69)]);

    // Four seconds, one reading per block (~5 ms).
    let blocks = (4.0 * SR) as usize / BLOCK;
    let mut per_sub: Vec<Vec<f32>> = vec![Vec::new(); 3];
    for _ in 0..blocks {
        render(&mut e, &params, &[]);
        for (i, (_, hz)) in e.sounding_osc1_freqs().enumerate() {
            per_sub[i].push(cents(hz, 69));
        }
    }

    // Tolerance for the f32 pitch -> Hz -> cents round trip.
    let eps = 0.01;
    for (i, trace) in per_sub.iter().enumerate() {
        assert_eq!(trace.len(), blocks);
        let (lo, hi) = trace
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &c| (lo.min(c), hi.max(c)));
        assert!(
            lo >= -DRIFT_MAX_CENTS - eps && hi <= DRIFT_MAX_CENTS + eps,
            "sub-voice {i} drifted outside ±{DRIFT_MAX_CENTS} ct: {lo}..{hi}"
        );
        assert!(
            hi - lo > 1.0,
            "sub-voice {i} barely drifted over 4 s: {lo}..{hi} ct"
        );
        // Slow: a 5 ms block never moves more than a small fraction of the
        // range (a ~1 Hz walk over ±6 ct moves ~0.2 ct per block at most).
        for w in trace.windows(2) {
            assert!(
                (w[1] - w[0]).abs() < 0.5,
                "sub-voice {i} jumped {} ct in one block",
                w[1] - w[0]
            );
        }
    }

    // Each sub-voice walks independently.
    assert_ne!(per_sub[0], per_sub[1]);
    assert_ne!(per_sub[1], per_sub[2]);
}

#[test]
fn drift_scales_with_the_knob() {
    let peak = |amount: f32| {
        let params = WavetableParams::new();
        params.analog.drift.set_value(amount);
        let mut e = engine();
        render(&mut e, &params, &[on(69)]);
        let mut peak = 0.0f32;
        for _ in 0..200 {
            render(&mut e, &params, &[]);
            for (_, hz) in e.sounding_osc1_freqs() {
                peak = peak.max(cents(hz, 69).abs());
            }
        }
        peak
    };
    let quarter = peak(0.25);
    assert!(quarter <= 0.25 * DRIFT_MAX_CENTS + 0.01, "{quarter}");
    assert!(quarter > 0.0);
}

#[test]
fn drift_walk_is_bounded_for_any_step_count() {
    // The walk is a one-pole over targets in [-1, 1]; hammer it far past any
    // realistic note length.
    let coeffs = DriftCoeffs::for_sample_rate(SR);
    let mut rng = AnalogRng::new(12345);
    let mut walk = DriftWalk::default();
    walk.start(&mut rng, &coeffs);
    for _ in 0..1_000_000 {
        walk.step(&mut rng, &coeffs);
        assert!((-1.0..=1.0).contains(&walk.value), "{}", walk.value);
    }
}

#[test]
fn analog_spreads_filter_cutoff_per_note_within_bounds() {
    let params = WavetableParams::new();
    params.filter.cutoff.set_value(1000.0);
    params.analog.drift.set_value(1.0);
    params.amp_env.release.set_value(0.02);
    let viz = WavetableVizState::new();
    let mut e = engine();

    let mut cutoffs = Vec::new();
    for _ in 0..8 {
        render(&mut e, &params, &[on(60)]);
        e.publish_viz(&params, &viz);
        cutoffs.push(viz.read_snapshot().filter_cutoff_live);
        render(&mut e, &params, &[off(60)]);
        drain(&mut e, &params);
    }

    let lo = 1000.0 * 2f32.powf(-CUTOFF_SPREAD_OCT) * 0.999;
    let hi = 1000.0 * 2f32.powf(CUTOFF_SPREAD_OCT) * 1.001;
    for c in &cutoffs {
        assert!((lo..=hi).contains(c), "cutoff {c} outside {lo}..{hi}");
    }
    let distinct = {
        let mut b: Vec<u32> = cutoffs.iter().map(|c| c.to_bits()).collect();
        b.sort_unstable();
        b.dedup();
        b.len()
    };
    assert!(distinct > 1, "every note got the same cutoff: {cutoffs:?}");
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

#[test]
fn analog_render_is_reproducible_from_a_fresh_engine() {
    let render_once = || {
        let params = rich_patch();
        params.analog.phase_random.set_value(1.0);
        params.analog.drift.set_value(1.0);
        let mut e = engine();
        play_sequence(&mut e, &params)
    };
    let a = render_once();
    let b = render_once();
    assert!(a.iter().any(|s| s.abs() > 1e-3));
    assert_eq!(bits(&a), bits(&b));
}

#[test]
fn reset_reseeds_the_analog_rng() {
    let params = rich_patch();
    params.analog.phase_random.set_value(1.0);
    params.analog.drift.set_value(1.0);
    let mut e = engine();
    settle(&mut e, &params);
    let first = play_sequence(&mut e, &params);
    e.reset();
    // `reset` does not rewind the free-running global LFOs' S&H RNG or the
    // effects' smoothers, but this patch uses neither, so the only state
    // that would differ is the analog seed.
    let second = play_sequence(&mut e, &params);
    assert_eq!(bits(&first), bits(&second));
}

#[test]
fn analog_actually_changes_the_sound() {
    let base = {
        let params = rich_patch();
        play_sequence(&mut engine(), &params)
    };
    let analog = {
        let params = rich_patch();
        params.analog.drift.set_value(1.0);
        play_sequence(&mut engine(), &params)
    };
    assert_ne!(bits(&base), bits(&analog));
}
