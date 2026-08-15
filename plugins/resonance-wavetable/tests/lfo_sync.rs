//! Tempo-synced LFOs (ba todo #1324).
//!
//! Before this, `_tempo` was ignored everywhere in the DSP and the "Sync"
//! segment of the LFO control meant retrigger. These tests pin the LFO's
//! period against a known tempo and division, and pin that the lock survives
//! a tempo change and a transport locate.

use resonance_plugin::param::Param;
use resonance_plugin::{EventIterator, NoteEvent, TempoInfo};
use resonance_wavetable::dsp::engine::SynthEngine;
use resonance_wavetable::dsp::lfo::{
    beats_per_bar, sync_phase, sync_rate_hz, LfoMode, SyncDivision, TransportPlan, FALLBACK_BPM,
};
use resonance_wavetable::params::WavetableParams;

const SR: f32 = 48_000.0;

fn tempo(bpm: f32, playing: bool, song_pos_beats: f64) -> TempoInfo {
    TempoInfo {
        bpm,
        time_sig_num: 4,
        time_sig_den: 4,
        playing,
        song_pos_beats,
    }
}

// ---------------------------------------------------------------------------
// The arithmetic
// ---------------------------------------------------------------------------

#[test]
fn division_labels_cover_every_discriminant() {
    let params = WavetableParams::new();
    assert_eq!(params.lfo1.division.min_plain(), 0.0);
    assert_eq!(
        params.lfo1.division.max_plain(),
        (SyncDivision::LABELS.len() - 1) as f64
    );
    for (i, label) in SyncDivision::LABELS.iter().enumerate() {
        let variant = SyncDivision::from_int(i as i32);
        assert_eq!(variant as usize, i, "SyncDivision::from_int({i}) round-trip");
        assert_eq!(&variant.label(), label);
    }
    // The default is one cycle per beat, the least surprising choice.
    assert_eq!(
        params.lfo1.division.default_plain(),
        SyncDivision::Quarter as i32 as f64
    );
}

#[test]
fn a_cycle_is_the_named_musical_length() {
    let bpb = 4.0; // 4/4
    assert_eq!(SyncDivision::Quarter.beats(bpb), 1.0);
    assert_eq!(SyncDivision::Eighth.beats(bpb), 0.5);
    assert_eq!(SyncDivision::Sixteenth.beats(bpb), 0.25);
    assert_eq!(SyncDivision::Half.beats(bpb), 2.0);
    assert_eq!(SyncDivision::OneBar.beats(bpb), 4.0);
    assert_eq!(SyncDivision::FourBars.beats(bpb), 16.0);
    // Dotted is 1.5x, triplet is 2/3.
    assert_eq!(SyncDivision::QuarterDotted.beats(bpb), 1.5);
    assert!((SyncDivision::QuarterTriplet.beats(bpb) - 2.0 / 3.0).abs() < 1e-6);
    assert_eq!(SyncDivision::EighthDotted.beats(bpb), 0.75);
}

#[test]
fn bar_divisions_follow_the_time_signature() {
    assert_eq!(beats_per_bar(4, 4), 4.0);
    assert_eq!(beats_per_bar(3, 4), 3.0);
    assert_eq!(beats_per_bar(6, 8), 3.0);
    assert_eq!(beats_per_bar(7, 8), 3.5);
    // A malformed signature falls back to common time rather than dividing
    // by zero.
    assert_eq!(beats_per_bar(0, 0), 4.0);

    assert_eq!(SyncDivision::OneBar.beats(beats_per_bar(3, 4)), 3.0);
    // A note-value division does not scale with the meter — an eighth note
    // is an eighth note in 7/8 too.
    assert_eq!(SyncDivision::Eighth.beats(beats_per_bar(7, 8)), 0.5);
}

#[test]
fn sync_rate_matches_tempo_and_division() {
    // 120 BPM: one beat is 0.5 s, so a 1/4 cycle is 2 Hz.
    assert!((sync_rate_hz(120.0, SyncDivision::Quarter.beats(4.0)) - 2.0).abs() < 1e-6);
    assert!((sync_rate_hz(120.0, SyncDivision::Eighth.beats(4.0)) - 4.0).abs() < 1e-6);
    assert!((sync_rate_hz(120.0, SyncDivision::OneBar.beats(4.0)) - 0.5).abs() < 1e-6);
    // 90 BPM: one beat is 2/3 s, so a 1/4 cycle is 1.5 Hz.
    assert!((sync_rate_hz(90.0, SyncDivision::Quarter.beats(4.0)) - 1.5).abs() < 1e-6);
    // Degenerate inputs produce a stopped LFO, not a NaN or an infinity.
    assert_eq!(sync_rate_hz(0.0, 1.0), 0.0);
    assert_eq!(sync_rate_hz(f32::NAN, 1.0), 0.0);
    assert_eq!(sync_rate_hz(120.0, 0.0), 0.0);
}

#[test]
fn sync_phase_is_the_position_within_the_cycle() {
    let quarter = SyncDivision::Quarter.beats(4.0);
    assert_eq!(sync_phase(0.0, quarter), 0.0);
    assert_eq!(sync_phase(0.5, quarter), 0.5);
    assert_eq!(sync_phase(4.0, quarter), 0.0);
    assert_eq!(sync_phase(4.25, quarter), 0.25);

    let bar = SyncDivision::OneBar.beats(4.0);
    assert_eq!(sync_phase(2.0, bar), 0.5);
    assert_eq!(sync_phase(8.0, bar), 0.0);
}

#[test]
fn a_missing_transport_falls_back_rather_than_stopping() {
    let t = TransportPlan::resolve(None);
    assert_eq!(t.bpm, FALLBACK_BPM);
    assert_eq!(t.beats_per_bar, 4.0);
    assert_eq!(t.song_pos_beats, None);
    // Still a real rate, so a synced LFO keeps moving offline.
    assert!(
        t.lfo_rate_hz(LfoMode::Sync, SyncDivision::Quarter, 7.0) > 0.0,
        "a synced LFO froze with no host transport"
    );
    // ...and there is nothing to anchor to, so it free-runs.
    assert_eq!(t.lfo_anchor_phase(LfoMode::Sync, SyncDivision::Quarter), None);
}

#[test]
fn only_sync_mode_reads_the_transport() {
    let t = TransportPlan::resolve(Some(tempo(120.0, true, 1.0)));
    for mode in [LfoMode::Free, LfoMode::Retrig] {
        assert_eq!(t.lfo_rate_hz(mode, SyncDivision::Quarter, 7.0), 7.0);
        assert_eq!(t.lfo_anchor_phase(mode, SyncDivision::Quarter), None);
    }
    assert!((t.lfo_rate_hz(LfoMode::Sync, SyncDivision::Quarter, 7.0) - 2.0).abs() < 1e-6);
    assert_eq!(
        t.lfo_anchor_phase(LfoMode::Sync, SyncDivision::Quarter),
        Some(0.0)
    );
}

// ---------------------------------------------------------------------------
// End to end through the engine
// ---------------------------------------------------------------------------

fn sync_params(division: SyncDivision) -> WavetableParams {
    let p = WavetableParams::new();
    let (sync, retrigger) = LfoMode::Sync.to_params();
    p.lfo1.sync.set_value(sync);
    p.lfo1.retrigger.set_value(retrigger);
    p.lfo1.division.set_plain(division as i32 as f64);
    // A rate the sync must override, so a passing test cannot be a
    // coincidence.
    p.lfo1.rate.set_value(7.0);
    p
}

/// Render `frames` samples of a held note and return LFO 1's phase after it.
fn render(engine: &mut SynthEngine, p: &WavetableParams, frames: usize, t: Option<TempoInfo>) -> f32 {
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    let events = [NoteEvent::NoteOn {
        note: 60,
        velocity: 1.0,
        timing: 0,
    }];
    let mut iter = EventIterator::new(&events);
    engine.render_block(&mut left, &mut right, frames, p, &mut iter, t);
    engine.global_lfo1.phase
}

#[test]
fn lfo_period_matches_the_tempo_and_division() {
    // 120 BPM at 1/4 is 2 Hz, so 1024 samples at 48 kHz advance the phase by
    // 1024 * 2 / 48000.
    let p = sync_params(SyncDivision::Quarter);
    let mut engine = SynthEngine::new();
    engine.initialize(SR);
    let phase = render(&mut engine, &p, 1024, Some(tempo(120.0, false, 0.0)));
    let expected = 1024.0 * 2.0 / SR;
    assert!(
        (phase - expected).abs() < 1e-4,
        "phase {phase} != expected {expected}"
    );

    // Halve the division, double the frequency.
    let p = sync_params(SyncDivision::Eighth);
    let mut engine = SynthEngine::new();
    engine.initialize(SR);
    let phase = render(&mut engine, &p, 1024, Some(tempo(120.0, false, 0.0)));
    let expected = 1024.0 * 4.0 / SR;
    assert!(
        (phase - expected).abs() < 1e-4,
        "phase {phase} != expected {expected}"
    );
}

#[test]
fn a_free_lfo_ignores_the_host_tempo() {
    let p = WavetableParams::new();
    let (sync, retrigger) = LfoMode::Free.to_params();
    p.lfo1.sync.set_value(sync);
    p.lfo1.retrigger.set_value(retrigger);
    p.lfo1.rate.set_value(7.0);

    let mut engine = SynthEngine::new();
    engine.initialize(SR);
    let phase = render(&mut engine, &p, 1024, Some(tempo(120.0, true, 3.0)));
    let expected = 1024.0 * 7.0 / SR;
    assert!(
        (phase - expected).abs() < 1e-4,
        "free LFO followed the transport: {phase} != {expected}"
    );
}

#[test]
fn a_synced_lfo_anchors_on_the_song_position() {
    // Half way through a beat, at 1/4, is half way through the cycle.
    let p = sync_params(SyncDivision::Quarter);
    let mut engine = SynthEngine::new();
    engine.initialize(SR);
    let phase = render(&mut engine, &p, 64, Some(tempo(120.0, true, 2.5)));
    let expected = 0.5 + 64.0 * 2.0 / SR;
    assert!(
        (phase - expected).abs() < 1e-4,
        "phase {phase} != expected {expected}"
    );
}

#[test]
fn sync_survives_a_locate() {
    // Play a block from beat 0, then jump the transport to beat 8. At 1/4
    // that is a whole number of cycles, so the LFO must be back at the top
    // of its cycle rather than wherever it had integrated to.
    let p = sync_params(SyncDivision::Quarter);
    let mut engine = SynthEngine::new();
    engine.initialize(SR);
    render(&mut engine, &p, 4096, Some(tempo(120.0, true, 0.0)));
    let phase = render(&mut engine, &p, 1, Some(tempo(120.0, true, 8.0)));
    let expected = 2.0 / SR;
    assert!(
        (phase - expected).abs() < 1e-4,
        "LFO did not re-lock after a locate: {phase} != {expected}"
    );
}

#[test]
fn sync_survives_a_tempo_change() {
    // Same musical position, different tempo: the phase is a function of the
    // timeline, so it is unchanged — only the rate the block advances at
    // moves.
    let p = sync_params(SyncDivision::Quarter);

    let mut slow = SynthEngine::new();
    slow.initialize(SR);
    let slow_phase = render(&mut slow, &p, 1, Some(tempo(90.0, true, 6.25)));

    let mut fast = SynthEngine::new();
    fast.initialize(SR);
    let fast_phase = render(&mut fast, &p, 1, Some(tempo(160.0, true, 6.25)));

    assert!(
        (slow_phase - 0.25).abs() < 1e-3 && (fast_phase - 0.25).abs() < 1e-3,
        "tempo change moved the musical position: {slow_phase} / {fast_phase}"
    );
}

#[test]
fn sync_ignores_note_on_retrigger() {
    // `Sync` writes retrigger=false, but a saved patch could hold both; the
    // DSP must still treat it as synced rather than resetting per note.
    let p = sync_params(SyncDivision::Quarter);
    p.lfo1.retrigger.set_value(true);

    let mut engine = SynthEngine::new();
    engine.initialize(SR);
    let phase = render(&mut engine, &p, 64, Some(tempo(120.0, true, 2.5)));
    let expected = 0.5 + 64.0 * 2.0 / SR;
    assert!(
        (phase - expected).abs() < 1e-4,
        "a note-on broke the transport lock: {phase} != {expected}"
    );
}
