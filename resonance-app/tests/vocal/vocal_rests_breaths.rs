//! Rests and breaths in the SVS segment (vocal-mcp-control-and-quality.md
//! §8, slice 12).
//!
//! - A gap of at least `rest_threshold_beats` between a note's end and the
//!   next onset is a rest (`SP`, gated to silence, and a render-unit
//!   boundary); a shorter one is sung legato.
//! - A standalone `AP` in the lyrics is a breath (`AP`) at the end of the
//!   gap before the next note, up to 0.35 s; with no gap, up to 30 % (at
//!   most 0.25 s) of the previous note's tail.
//!
//! Every scenario pins its tempo, notes and params; none renders audio.

use resonance_app::compose::expression::ExpressionCurves;
use resonance_app::compose::vocal_svs::{
    articulation_report_for, build_segment, resolve_clip, split_render_units,
    validate_for_voicebank, BankProfile,
};
use resonance_audio::types::{MidiNote, TICKS_PER_QUARTER_NOTE};
use resonance_music_theory::derive::LyricLine;
use resonance_music_theory::g2p::AssignedSyllable;
use resonance_music_theory::{VocalParams, VoicebankId};
use resonance_svs::ds::DsSegment;

const TPQ: u32 = TICKS_PER_QUARTER_NOTE as u32;
const Q: u64 = TICKS_PER_QUARTER_NOTE;
const BPM: f32 = 120.0;
/// Seconds per beat at [`BPM`].
const BEAT: f64 = 0.5;
const PAD: f64 = 0.3;
const EPS: f64 = 1e-6;

fn params(rest_threshold_beats: f32) -> VocalParams {
    VocalParams {
        voicebank: VoicebankId::new("lilia"),
        rest_threshold_beats,
        ..VocalParams::default()
    }
}

fn bank() -> BankProfile {
    BankProfile::not_installed("lilia")
}

fn assign(text: &str, notes: usize) -> Vec<AssignedSyllable> {
    let draft = vec![LyricLine {
        n: 1,
        rhyme: 'A',
        syllables: notes as u8,
        text: text.to_owned(),
        locked: false,
    }];
    let resolved = resolve_clip(&draft, &[], notes, &Default::default(), &[], &[]);
    validate_for_voicebank(&resolved, &bank()).expect("plain English resolves")
}

fn note(start_tick: u64, duration_ticks: u64) -> MidiNote {
    MidiNote {
        note: 64,
        velocity: 0.8,
        start_tick,
        duration_ticks,
    }
}

fn segment(notes: &[MidiNote], text: &str, rest_threshold_beats: f32) -> DsSegment {
    build_segment(
        notes,
        &params(rest_threshold_beats),
        &assign(text, notes.len()),
        &ExpressionCurves::default(),
        TPQ,
        BPM,
        &bank(),
    )
}

/// `(phoneme, start, duration)` of every entry.
fn timeline(seg: &DsSegment) -> Vec<(String, f64, f64)> {
    let mut t = 0.0;
    seg.ph_seq
        .iter()
        .zip(&seg.ph_dur)
        .map(|(p, d)| {
            let e = (p.clone(), t, *d);
            t += d;
            e
        })
        .collect()
}

/// The `SP`/`AP` entries strictly between the pads.
fn inner(seg: &DsSegment, kind: &str) -> Vec<(f64, f64)> {
    let tl = timeline(seg);
    tl[1..tl.len() - 1]
        .iter()
        .filter(|e| e.0 == kind)
        .map(|e| (e.1, e.2))
        .collect()
}

/// Seconds the notes really span, plus both pads.
fn span(notes: &[MidiNote]) -> f64 {
    let last = notes.last().unwrap();
    (last.start_tick + last.duration_ticks - notes[0].start_tick) as f64 * BEAT / Q as f64
        + 2.0 * PAD
}

fn total(seg: &DsSegment) -> f64 {
    seg.ph_dur.iter().sum()
}

/// Two quarter notes with an eighth rest between them (quarter, eighth
/// rest, quarter).
fn eighth_rest() -> Vec<MidiNote> {
    vec![note(0, Q), note(Q + Q / 2, Q)]
}

#[test]
fn an_eighth_rest_at_the_default_threshold_is_a_silent_gap() {
    let notes = eighth_rest();
    let seg = segment(&notes, "la la", VocalParams::DEFAULT_REST_THRESHOLD_BEATS);
    // Guard: the notes themselves are sung.
    assert!(seg.ph_seq.iter().any(|p| p == "aa"), "{:?}", seg.ph_seq);
    let rests = inner(&seg, "SP");
    assert_eq!(rests.len(), 1, "one rest between the notes: {:?}", timeline(&seg));
    let (start, dur) = rests[0];
    assert!((dur - 0.5 * BEAT).abs() < EPS, "the rest is the eighth: {dur}");
    assert!((start - (PAD + BEAT)).abs() < EPS, "it starts at the first note's end: {start}");
    assert!(inner(&seg, "AP").is_empty(), "a plain rest is not a breath");
    assert!((total(&seg) - span(&notes)).abs() < EPS);

    // Pads are rests too, never breaths.
    assert_eq!(seg.ph_seq.first().map(String::as_str), Some("SP"));
    assert_eq!(seg.ph_seq.last().map(String::as_str), Some("SP"));

    // The render-unit splitter agrees: two units.
    let units = split_render_units(
        &notes,
        &params(0.5),
        &assign("la la", 2),
        &ExpressionCurves::default(),
        TPQ,
        BPM,
        &bank(),
    );
    assert_eq!(units.len(), 2);
}

#[test]
fn raising_the_threshold_sings_the_same_gap_legato() {
    let notes = eighth_rest();
    let seg = segment(&notes, "la la", 1.0);
    assert!(inner(&seg, "SP").is_empty(), "{:?}", timeline(&seg));
    assert!(inner(&seg, "AP").is_empty());
    assert!((total(&seg) - span(&notes)).abs() < EPS);
    let units = split_render_units(
        &notes,
        &params(1.0),
        &assign("la la", 2),
        &ExpressionCurves::default(),
        TPQ,
        BPM,
        &bank(),
    );
    assert_eq!(units.len(), 1);
}

#[test]
fn lowering_the_threshold_turns_a_sixteenth_gap_into_a_rest() {
    // A sixteenth gap (0.25 beat): legato at the default, a rest at 0.25.
    let notes = vec![note(0, Q), note(Q + Q / 4, Q)];
    let legato = segment(&notes, "la la", 0.5);
    assert!(inner(&legato, "SP").is_empty(), "{:?}", timeline(&legato));
    let rest = segment(&notes, "la la", 0.25);
    let rests = inner(&rest, "SP");
    assert_eq!(rests.len(), 1, "{:?}", timeline(&rest));
    assert!((rests[0].1 - 0.25 * BEAT).abs() < EPS);

    // The articulation report follows the lane's threshold: legato, the
    // first note owns its slot; as a rest, only its written length.
    let syl = assign("la la", 2);
    let at = |beats| articulation_report_for(&notes, &syl, TPQ, BPM, &bank(), beats)[0].duration_sec;
    assert!((at(0.5) - 1.25 * BEAT).abs() < EPS);
    assert!((at(0.25) - BEAT).abs() < EPS);
}

#[test]
fn a_breath_fills_the_end_of_the_gap_before_its_note() {
    // A one-beat rest (0.5 s) between the notes.
    let notes = vec![note(0, Q), note(2 * Q, Q)];
    let seg = segment(&notes, "la AP la", 0.5);
    assert!(seg.ph_seq.iter().any(|p| p == "aa"), "{:?}", seg.ph_seq);
    let tl = timeline(&seg);
    let breaths = inner(&seg, "AP");
    assert_eq!(breaths.len(), 1, "{tl:?}");
    let (b_start, b_dur) = breaths[0];
    assert!((b_dur - 0.35).abs() < EPS, "a full 0.35 s breath: {b_dur}");
    // It ends where the second note's phonemes begin.
    assert!(
        (b_start + b_dur - (PAD + 2.0 * BEAT)).abs() < EPS,
        "ends at the next onset: {tl:?}"
    );
    // What is left of the gap stays a silent rest in front of it.
    let rests = inner(&seg, "SP");
    assert_eq!(rests.len(), 1, "{tl:?}");
    assert!((rests[0].1 - (BEAT - 0.35)).abs() < EPS);
    assert!((total(&seg) - span(&notes)).abs() < EPS, "the segment keeps its length");

    // The render units keep the rest and its breath together, so what is
    // rendered is this same layout (not a breath squeezed into the next
    // unit's leading pad).
    let units = split_render_units(
        &notes,
        &params(0.5),
        &assign("la AP la", 2),
        &ExpressionCurves::default(),
        TPQ,
        BPM,
        &bank(),
    );
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].segment.ph_seq, seg.ph_seq);
}

#[test]
fn a_breath_with_no_gap_takes_the_previous_notes_tail() {
    // Two legato half notes (1 s each): 30 % of the first is 0.3 s, so the
    // 0.25 s cap applies.
    let notes = vec![note(0, 2 * Q), note(2 * Q, 2 * Q)];
    let seg = segment(&notes, "la AP la", 0.5);
    let tl = timeline(&seg);
    assert!(inner(&seg, "SP").is_empty(), "legato: no rest: {tl:?}");
    let breaths = inner(&seg, "AP");
    assert_eq!(breaths.len(), 1, "{tl:?}");
    let (b_start, b_dur) = breaths[0];
    assert!((b_dur - 0.25).abs() < EPS, "capped at 0.25 s: {b_dur}");
    assert!((b_start + b_dur - (PAD + 2.0 * BEAT)).abs() < EPS, "{tl:?}");
    assert!((total(&seg) - span(&notes)).abs() < EPS);

    // Short legato quarter notes (0.5 s): 30 % of the first, 0.15 s.
    let notes = vec![note(0, Q), note(Q, Q)];
    let seg = segment(&notes, "la AP la", 0.5);
    let breaths = inner(&seg, "AP");
    assert_eq!(breaths.len(), 1, "{:?}", timeline(&seg));
    assert!((breaths[0].1 - 0.3 * BEAT).abs() < EPS, "{:?}", breaths);
    assert!((total(&seg) - span(&notes)).abs() < EPS);
}

#[test]
fn a_short_gap_lends_the_rest_of_the_breath_from_the_tail() {
    // A 0.5-beat rest (0.25 s) in front of the breath: all of it, plus
    // 0.1 s of the previous (0.5 s) note's tail.
    let notes = vec![note(0, Q), note(Q + Q / 2, Q)];
    let seg = segment(&notes, "la AP la", 0.5);
    let breaths = inner(&seg, "AP");
    assert_eq!(breaths.len(), 1, "{:?}", timeline(&seg));
    assert!((breaths[0].1 - 0.35).abs() < EPS, "{breaths:?}");
    assert!(inner(&seg, "SP").is_empty(), "the gap went to the breath");
    assert!((total(&seg) - span(&notes)).abs() < EPS);
}

#[test]
fn a_breath_on_the_first_note_takes_the_leading_pad() {
    let notes = vec![note(0, Q), note(Q, Q)];
    let seg = segment(&notes, "AP la la", 0.5);
    let tl = timeline(&seg);
    assert_eq!(tl[0].0, "SP");
    assert!((tl[0].2 - 0.05).abs() < EPS, "{tl:?}");
    assert_eq!(tl[1].0, "AP");
    assert!((tl[1].2 - 0.25).abs() < EPS, "{tl:?}");
    assert!((total(&seg) - span(&notes)).abs() < EPS);
}

#[test]
fn without_an_ap_token_nothing_breathes() {
    for notes in [vec![note(0, Q), note(2 * Q, Q)], vec![note(0, Q), note(Q, Q)]] {
        let seg = segment(&notes, "la la", 0.5);
        assert!(
            !seg.ph_seq.iter().any(|p| p == "AP"),
            "only written breaths are AP: {:?}",
            seg.ph_seq
        );
    }
}
