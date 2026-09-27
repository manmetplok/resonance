//! Behavioural tests for the new modulation sources (RandomBipolar,
//! RandomUnipolar, SampleHold, Alternate).
//!
//! `mod_availability.rs` already pins `evaluate_mod_matrix`'s unit-level
//! arithmetic for these four; this file drives the real engine end to end,
//! the way `tests/analog.rs` and `tests/lfo_sync.rs` pin their own per-note
//! randomness and tempo sync. Every test routes the source under test to
//! `ModDest::Osc1Pitch` at full amount, which is the cleanest observable:
//! `evaluate_mod_matrix` scales it by exactly 12 semitones (1200 cents), so
//! a source's whole range maps onto a directly measurable pitch offset with
//! no envelope, filter or level in the way, and `SynthEngine::
//! sounding_osc1_freqs` reads the *resolved* frequency straight out of the
//! render path.
//!
//! `SynthEngine::mod_sample_hold` is `pub` and `SampleHoldGen::value()` is a
//! `pub fn`, so the S&H tests read the generator directly rather than
//! through the matrix — the same shortcut `lfo_sync.rs` takes via
//! `engine.global_lfo1.phase`.

use resonance_plugin::{EventIterator, NoteEvent, Param, TempoInfo};
use resonance_wavetable::dsp::engine::SynthEngine;
use resonance_wavetable::dsp::lfo::{sync_rate_hz, SyncDivision};
use resonance_wavetable::dsp::modulation::{ModDest, ModSource};
use resonance_wavetable::dsp::oscillator::midi_to_freq;
use resonance_wavetable::params::WavetableParams;

const SR: f32 = 48_000.0;

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

fn render(engine: &mut SynthEngine, params: &WavetableParams, events: &[NoteEvent], frames: usize) {
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    let mut iter = EventIterator::new(events);
    engine.render_block(&mut left, &mut right, frames, params, &mut iter, None);
}

/// Render `frames` of silence with an explicit transport, for the S&H sync
/// tests — same shape as `lfo_sync.rs`'s `render`, but this generator runs
/// unconditionally every sample regardless of whether a voice sounds, so no
/// note is needed at all.
fn render_with_tempo(engine: &mut SynthEngine, params: &WavetableParams, frames: usize, t: TempoInfo) {
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    let events: [NoteEvent; 0] = [];
    let mut iter = EventIterator::new(&events);
    engine.render_block(&mut left, &mut right, frames, params, &mut iter, Some(t));
}

/// Render a note until it has fully released, so the next note-on lands on a
/// quiescent engine (same purpose and margin as `analog.rs`'s helper of the
/// same name -- 40 * 256 samples against a 0.02 s release, set by
/// [`quick_release`]).
fn drain(engine: &mut SynthEngine, params: &WavetableParams) {
    for _ in 0..40 {
        render(engine, params, &[], 256);
    }
    assert_eq!(engine.sounding_voices().count(), 0, "voice did not drain");
}

/// Short attack/release on both envelopes, so [`drain`] actually catches up
/// within its budget. `WavetableParams::new()`'s default amp release is
/// 0.3 s (14400 samples) -- far past what a short test drain renders.
fn quick_release(p: &WavetableParams) {
    p.amp_env.attack.set_value(0.001);
    p.amp_env.release.set_value(0.02);
    p.mod_env.attack.set_value(0.001);
    p.mod_env.release.set_value(0.02);
}

/// Cents between `hz` and the equal-tempered pitch of `note`.
fn cents(hz: f32, note: u8) -> f32 {
    1200.0 * (hz / midi_to_freq(note as f32)).log2()
}

/// The lone sounding voice's osc1 pitch offset from `note`, in cents.
/// Panics if there isn't exactly one sounding sub-voice, which every test
/// below arranges (`unison.voices` at its default of 1, one held note).
fn osc1_offset_cents(engine: &SynthEngine, note: u8) -> f32 {
    let mut freqs = engine.sounding_osc1_freqs();
    let (got_note, hz) = freqs.next().expect("no sounding voice");
    assert_eq!(got_note, note, "wrong voice sounding");
    assert!(freqs.next().is_none(), "expected exactly one sub-voice");
    cents(hz, note)
}

/// Route `source` -> `ModDest::Osc1Pitch` at full amount (slot 0).
fn route_to_pitch(p: &WavetableParams, source: ModSource) {
    p.mod_slots[0].source.set_value(source as i32);
    p.mod_slots[0].destination.set_value(ModDest::Osc1Pitch as i32);
    p.mod_slots[0].amount.set_value(1.0);
}

const EPS_CENTS: f32 = 0.05;

// ---------------------------------------------------------------------------
// Random note-on: constant within a note, differs across notes
// ---------------------------------------------------------------------------

#[test]
fn random_bipolar_is_constant_within_a_note_but_differs_across_notes() {
    let p = WavetableParams::new();
    quick_release(&p);
    route_to_pitch(&p, ModSource::RandomBipolar);
    let mut e = engine();

    let notes = [60u8, 64, 67, 72];
    let mut offsets = Vec::new();
    for &note in &notes {
        render(&mut e, &p, &[on(note)], 64);
        let first = osc1_offset_cents(&e, note);
        // Hold the note across several more blocks with nothing else
        // moving; the draw must not change while it sounds.
        for _ in 0..5 {
            render(&mut e, &p, &[], 64);
            let later = osc1_offset_cents(&e, note);
            assert_eq!(
                later.to_bits(),
                first.to_bits(),
                "random value moved mid-note for note {note}"
            );
        }
        offsets.push(first);
        render(&mut e, &p, &[off(note)], 64);
        drain(&mut e, &p);
    }

    // Every pair of notes drew a different value. With a 32-bit RNG this is
    // the overwhelming expectation and the run is deterministic (fixed
    // seed), so a pass here is a pass forever.
    for i in 0..offsets.len() {
        for j in (i + 1)..offsets.len() {
            assert_ne!(
                offsets[i], offsets[j],
                "notes {} and {} drew the same random value",
                notes[i], notes[j]
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Range: bipolar in -1..1, unipolar in 0..1
// ---------------------------------------------------------------------------

/// Trigger `count` distinct, sequential notes and collect each one's osc1
/// pitch offset in cents (full amount, so this is exactly 1200*source_value).
fn collect_offsets(p: &WavetableParams, count: usize) -> Vec<f32> {
    let mut e = engine();
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let note = 40 + (i % 40) as u8;
        render(&mut e, p, &[on(note)], 64);
        out.push(osc1_offset_cents(&e, note));
        render(&mut e, p, &[off(note)], 64);
        drain(&mut e, p);
    }
    out
}

#[test]
fn random_bipolar_spans_the_full_range_and_goes_negative() {
    let p = WavetableParams::new();
    quick_release(&p);
    route_to_pitch(&p, ModSource::RandomBipolar);
    let offsets = collect_offsets(&p, 40);

    for &c in &offsets {
        assert!(
            (-1200.0 - EPS_CENTS..=1200.0 + EPS_CENTS).contains(&c),
            "bipolar offset {c} ct escaped -1200..1200"
        );
    }
    assert!(
        offsets.iter().any(|&c| c < -100.0),
        "never went meaningfully negative across 40 draws: {offsets:?}"
    );
    assert!(
        offsets.iter().any(|&c| c > 100.0),
        "never went meaningfully positive across 40 draws: {offsets:?}"
    );
}

#[test]
fn random_unipolar_spans_zero_to_one_and_never_goes_negative() {
    let p = WavetableParams::new();
    quick_release(&p);
    route_to_pitch(&p, ModSource::RandomUnipolar);
    let offsets = collect_offsets(&p, 40);

    for &c in &offsets {
        assert!(
            (-EPS_CENTS..=1200.0 + EPS_CENTS).contains(&c),
            "unipolar offset {c} ct escaped 0..1200"
        );
    }
    assert!(
        offsets.iter().any(|&c| c > 100.0),
        "never rose meaningfully above zero across 40 draws: {offsets:?}"
    );
    assert!(
        !offsets.iter().any(|&c| c < -100.0),
        "unipolar source went clearly negative: {offsets:?}"
    );
}

// ---------------------------------------------------------------------------
// Sample & Hold: steps at the configured rate, free and synced
// ---------------------------------------------------------------------------

/// Render silence in `chunk`-sized steps for `total_samples`, sampling
/// `engine.mod_sample_hold.value()` after every chunk. Free-running, so no
/// note is needed — the generator advances unconditionally every sample.
fn sh_trace_free(p: &WavetableParams, chunk: usize, total_samples: usize) -> Vec<f32> {
    let mut e = engine();
    let mut out = Vec::new();
    let mut rendered = 0;
    while rendered < total_samples {
        render(&mut e, p, &[], chunk);
        rendered += chunk;
        out.push(e.mod_sample_hold.value());
    }
    out
}

/// Same, but with an explicit rolling transport for the synced case.
fn sh_trace_synced(p: &WavetableParams, bpm: f32, chunk: usize, total_samples: usize) -> Vec<f32> {
    let mut e = engine();
    let mut out = Vec::new();
    let mut rendered = 0usize;
    while rendered < total_samples {
        let song_pos_beats = rendered as f64 / SR as f64 * (bpm as f64 / 60.0);
        let t = TempoInfo {
            bpm,
            time_sig_num: 4,
            time_sig_den: 4,
            playing: true,
            song_pos_beats,
        };
        render_with_tempo(&mut e, p, chunk, t);
        rendered += chunk;
        out.push(e.mod_sample_hold.value());
    }
    out
}

/// Sample indices (in units of `chunk`) where consecutive readings differ.
/// At `slew = 0` every reading in between two of these is bit-identical to
/// its neighbours (the "stepped" half of the S&H contract) simply because
/// nothing wrote a new value there; this only collects where it *did*.
fn transitions(trace: &[f32]) -> Vec<usize> {
    (1..trace.len())
        .filter(|&i| trace[i].to_bits() != trace[i - 1].to_bits())
        .collect()
}

/// Assert `trace`'s transitions land close to every multiple of
/// `period_samples`, within `expected_cycles` ± 1.
fn assert_steps_at_rate(trace: &[f32], chunk: usize, period_samples: f32, expected_cycles: usize) {
    let jumps = transitions(trace);
    assert!(
        jumps.len() as i64 >= expected_cycles as i64 - 1
            && jumps.len() as i64 <= expected_cycles as i64 + 1,
        "expected ~{expected_cycles} steps, got {} at {:?}",
        jumps.len(),
        jumps
    );
    for w in jumps.windows(2) {
        let spacing = ((w[1] - w[0]) * chunk) as f32;
        assert!(
            (spacing - period_samples).abs() < period_samples * 0.1 + 4.0 * chunk as f32,
            "step spacing {spacing} samples far from the expected period {period_samples}"
        );
    }
}

#[test]
fn sample_hold_steps_at_the_free_rate() {
    let p = WavetableParams::new();
    p.mod_sh.rate.set_value(20.0); // 2400-sample period at 48 kHz
    p.mod_sh.slew.set_value(0.0);
    p.mod_sh.sync.set_value(false);

    let period_samples = SR / 20.0;
    let chunk = 16usize;
    let cycles = 6usize;
    let trace = sh_trace_free(&p, chunk, (period_samples as usize) * cycles);
    assert_steps_at_rate(&trace, chunk, period_samples, cycles);
}

#[test]
fn sample_hold_steps_at_the_synced_rate() {
    let p = WavetableParams::new();
    p.mod_sh.sync.set_value(true);
    p.mod_sh.division.set_plain(SyncDivision::Sixteenth as i32 as f64);
    p.mod_sh.slew.set_value(0.0);

    let bpm = 120.0;
    let rate_hz = sync_rate_hz(bpm, SyncDivision::Sixteenth.beats(4.0));
    let period_samples = SR / rate_hz;
    // A chunk size that shares no clean rational relationship with the
    // 6000-sample period: `set_phase` re-anchors both `phase` and
    // `prev_phase` to the same value at every block boundary, so a wrap
    // whose true moment lands exactly on one goes undetected (`phase <
    // prev_phase` is trivially false when they were just set equal). 32
    // divides 6000 into an exact half-integer (187.5), so every *other*
    // wrap landed exactly on a chunk boundary and silently vanished --
    // a chunking artifact of this test, not an engine bug (the free-rate
    // test above has no periodic re-anchor and is unaffected by chunk
    // size). A prime chunk size cannot resonate like that.
    let chunk = 101usize;
    let cycles = 5usize;
    let trace = sh_trace_synced(&p, bpm, chunk, (period_samples as usize) * cycles);
    assert_steps_at_rate(&trace, chunk, period_samples, cycles);
}

// ---------------------------------------------------------------------------
// Slew smooths the steps into a drift
// ---------------------------------------------------------------------------

#[test]
fn slew_greater_than_zero_smooths_the_steps() {
    let stepped = {
        let p = WavetableParams::new();
        p.mod_sh.rate.set_value(20.0);
        p.mod_sh.slew.set_value(0.0);
        sh_trace_free(&p, 16, 16 * 900) // ~6 cycles at 2400-sample period
    };
    let smoothed = {
        let p = WavetableParams::new();
        p.mod_sh.rate.set_value(20.0);
        p.mod_sh.slew.set_value(0.5);
        sh_trace_free(&p, 16, 16 * 900)
    };

    let max_step = |trace: &[f32]| -> f32 {
        trace
            .windows(2)
            .fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()))
    };
    let stepped_max = max_step(&stepped);
    let smoothed_max = max_step(&smoothed);

    // A stepped random target can flip across the whole ±1 range in one
    // chunk; a slewed one can only creep a small fraction of the way there.
    assert!(
        stepped_max > 0.3,
        "expected at least one large stepped jump, got {stepped_max}"
    );
    assert!(
        smoothed_max < 0.05,
        "slew = 0.5 should smooth away single-chunk jumps, got {smoothed_max}"
    );
    assert!(
        smoothed_max < stepped_max * 0.2,
        "slewed max step ({smoothed_max}) should be well below the stepped one ({stepped_max})"
    );

    // And it is genuinely still moving (not stuck), just gradually.
    let moved = smoothed.windows(2).filter(|w| w[0] != w[1]).count();
    assert!(moved > smoothed.len() / 4, "the slewed value barely moved at all");
}

// ---------------------------------------------------------------------------
// Alternate: flips ±1 per trigger, not on legato
// ---------------------------------------------------------------------------

fn alt_params() -> WavetableParams {
    let p = WavetableParams::new();
    quick_release(&p);
    route_to_pitch(&p, ModSource::Alternate);
    p
}

#[test]
fn alternate_flips_plus_minus_one_on_every_trigger() {
    let p = alt_params();
    let mut e = engine();

    // First trigger of this engine: `mod_alternate` starts at -1.0 and
    // flips before the first `trigger()`, landing on +1.0.
    render(&mut e, &p, &[on(60)], 64);
    let c1 = osc1_offset_cents(&e, 60);
    assert!((c1 - 1200.0).abs() < EPS_CENTS, "{c1}");
    render(&mut e, &p, &[off(60)], 64);
    drain(&mut e, &p);

    render(&mut e, &p, &[on(64)], 64);
    let c2 = osc1_offset_cents(&e, 64);
    assert!((c2 + 1200.0).abs() < EPS_CENTS, "second trigger did not flip: {c2}");
    render(&mut e, &p, &[off(64)], 64);
    drain(&mut e, &p);

    render(&mut e, &p, &[on(67)], 64);
    let c3 = osc1_offset_cents(&e, 67);
    assert!((c3 - 1200.0).abs() < EPS_CENTS, "third trigger did not flip back: {c3}");
}

#[test]
fn alternate_does_not_flip_on_mono_legato() {
    let p = alt_params();
    p.max_voices.set_value(1);
    let mut e = engine();

    render(&mut e, &p, &[on(60)], 64);
    let c1 = osc1_offset_cents(&e, 60);

    // A second key pressed while the first is still held: mono legato, the
    // same voice takes the new note over without a fresh `trigger()`.
    render(&mut e, &p, &[on(64)], 64);
    let c2 = osc1_offset_cents(&e, 64);
    assert_eq!(
        c1.to_bits(),
        c2.to_bits(),
        "legato take-over flipped Alternate: {c1} -> {c2}"
    );

    // Releasing back to the first held key is also legato, not a trigger.
    render(&mut e, &p, &[off(64)], 64);
    let c3 = osc1_offset_cents(&e, 60);
    assert_eq!(
        c1.to_bits(),
        c3.to_bits(),
        "legato return flipped Alternate: {c1} -> {c3}"
    );

    // A real new trigger, after the voice is fully released, does flip.
    render(&mut e, &p, &[off(60)], 64);
    drain(&mut e, &p);
    render(&mut e, &p, &[on(60)], 64);
    let c4 = osc1_offset_cents(&e, 60);
    assert_ne!(
        c1.to_bits(),
        c4.to_bits(),
        "a genuine retrigger after full release should flip Alternate"
    );
}
