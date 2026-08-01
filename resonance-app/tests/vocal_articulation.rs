//! Intelligibility guards for the SVS path: does each note get enough
//! time to articulate the phonemes assigned to it, and does the
//! `song.vocal` pre-flight say so?
//!
//! The failure these pin down is silent. A word with no syllable breaks
//! is ONE lyric token, so its whole phoneme run lands on a single note;
//! at any singable tempo that is a few tens of milliseconds per phoneme
//! and the word is heard as a smear. Nothing errors — the render
//! succeeds and simply cannot be understood. So we assert both halves of
//! the fix: the duration allocator protects each phone's audibility
//! floor when it can, and [`articulation_report`] flags the note when it
//! cannot.

use resonance_app::compose::vocal_svs::{
    articulation_report, build_segment, comfortable_pitch_range, floor_duration_sec,
    min_articulation_sec, resolve_clip_pronunciation, validate_for_voicebank,
};
use resonance_audio::types::{MidiNote, TICKS_PER_QUARTER_NOTE};
use resonance_music_theory::derive::LyricLine;
use resonance_music_theory::g2p::{self, AssignedSyllable};
use resonance_music_theory::{VocalParams, VocalVoicebank};

const TPQ: u32 = TICKS_PER_QUARTER_NOTE as u32;
/// The tempo the song under investigation was written at.
const BPM: f32 = 140.0;

fn draft(text: &str) -> Vec<LyricLine> {
    vec![LyricLine {
        n: 1,
        rhyme: 'A',
        syllables: 1,
        text: text.to_string(),
        locked: false,
    }]
}

/// `count` back-to-back notes of `beats` each, all at `pitch`.
fn notes(count: usize, beats: f64, pitch: u8) -> Vec<MidiNote> {
    let dur = (beats * TICKS_PER_QUARTER_NOTE as f64) as u64;
    (0..count)
        .map(|i| MidiNote {
            note: pitch,
            velocity: 0.8,
            start_tick: i as u64 * dur,
            duration_ticks: dur,
        })
        .collect()
}

/// Resolve `text` onto `n` notes the way the render path does.
fn assign(text: &str, n: usize) -> Vec<AssignedSyllable> {
    let resolved =
        resolve_clip_pronunciation(&draft(text), &[], n, &Default::default(), &[], &[]);
    validate_for_voicebank(&resolved, VocalVoicebank::Lilia).expect("plain English resolves")
}

fn params() -> VocalParams {
    VocalParams {
        voicebank: VocalVoicebank::Lilia,
        ..VocalParams::default()
    }
}

// ---------------------------------------------------------------------------
// The report
// ---------------------------------------------------------------------------

#[test]
fn crammed_word_on_one_short_note_is_flagged() {
    // `resolution` with no syllable breaks: nine phonemes on one
    // half-beat note (214 ms at 140 BPM). Their floors alone need well
    // over that, so the note is reported as unsingable-as-written.
    let assigned = assign("resolution", 1);
    assert_eq!(assigned[0].phonemes.len(), 9, "{:?}", assigned[0].phonemes);

    let report = articulation_report(
        &notes(1, 0.5, 64),
        &assigned,
        TPQ,
        BPM,
        VocalVoicebank::Lilia,
    );
    assert_eq!(report.len(), 1);
    assert_eq!(report[0].phonemes.len(), 9);
    assert!(
        report[0].too_short,
        "9 phonemes in {:.0} ms should be flagged (needs {:.0} ms)",
        report[0].duration_sec * 1000.0,
        report[0].min_duration_sec * 1000.0
    );
    assert!(report[0].min_duration_sec > report[0].duration_sec);
}

#[test]
fn the_same_word_split_across_notes_is_not_flagged() {
    // The fix, end to end: broken into `re·so·lu·tion` the word occupies
    // four notes and every one of them clears its floor at the same
    // tempo and note length.
    let text = g2p::auto_syllabify_text("resolution");
    let assigned = assign(&text, 4);
    assert_eq!(assigned.len(), 4);
    assert!(
        assigned.iter().all(|a| !a.phonemes.is_empty()),
        "a note was left with nothing to sing: {assigned:?}"
    );

    let report = articulation_report(
        &notes(4, 0.5, 64),
        &assigned,
        TPQ,
        BPM,
        VocalVoicebank::Lilia,
    );
    assert_eq!(report.len(), 4);
    for n in &report {
        assert!(
            !n.too_short,
            "syllable {:?} ({:?}) flagged: {:.0} ms available, {:.0} ms needed",
            n.label,
            n.phonemes,
            n.duration_sec * 1000.0,
            n.min_duration_sec * 1000.0
        );
    }
}

#[test]
fn out_of_range_notes_are_flagged_against_the_voicebank() {
    let (lo, hi) = comfortable_pitch_range(VocalVoicebank::Lilia);
    assert!(lo < hi);
    let assigned = assign("la la la", 3);
    let mut ns = notes(3, 1.0, lo + 6);
    ns[1].note = lo - 12;
    ns[2].note = hi + 12;

    let report = articulation_report(&ns, &assigned, TPQ, BPM, VocalVoicebank::Lilia);
    assert!(!report[0].out_of_range, "mid-range note flagged");
    assert!(report[1].out_of_range, "an octave below the range");
    assert!(report[2].out_of_range, "an octave above the range");
}

#[test]
fn report_durations_follow_the_note_grid() {
    // A half-beat note at 140 BPM is 214 ms; the report must agree with
    // the arithmetic a caller would do by hand, or its `too_short`
    // verdict is unactionable.
    let assigned = assign("go", 1);
    let report = articulation_report(
        &notes(1, 0.5, 64),
        &assigned,
        TPQ,
        BPM,
        VocalVoicebank::Lilia,
    );
    let expected = 60.0 / BPM as f64 * 0.5;
    assert!(
        (report[0].duration_sec - expected).abs() < 1e-6,
        "expected {expected} s, got {}",
        report[0].duration_sec
    );
}

// ---------------------------------------------------------------------------
// The allocator
// ---------------------------------------------------------------------------

/// Every lexical phoneme entry in a segment, paired with its duration
/// (the `AP`/`SP` pads dropped).
fn lexical_durations(seq: &[String], dur: &[f64]) -> Vec<(String, f64)> {
    seq.iter()
        .zip(dur)
        .filter(|(p, _)| !matches!(p.as_str(), "AP" | "SP" | "cl"))
        .map(|(p, d)| (p.clone(), *d))
        .collect()
}

#[test]
fn a_roomy_note_gives_every_phoneme_its_floor() {
    // `still` = s t ih l. On a note with time to spare, no phone may be
    // pushed under its audibility floor — the old allocator handed every
    // consonant the same slice and let the cap starve them all.
    let assigned = assign("still", 1);
    let segment = build_segment(&notes(1, 2.0, 64), &params(), &assigned, TPQ, BPM);
    for (ph, d) in lexical_durations(&segment.ph_seq, &segment.ph_dur) {
        let floor = floor_duration_sec(&ph);
        assert!(
            d >= floor - 1e-9,
            "{ph} got {:.1} ms, under its {:.1} ms floor",
            d * 1000.0,
            floor * 1000.0
        );
    }
}

#[test]
fn fricatives_get_more_time_than_stops() {
    // The point of the per-class table: an `s` needs materially longer
    // than a `t` before it is identifiable. A flat per-consonant
    // duration cannot express that.
    let assigned = assign("stop", 1);
    let segment = build_segment(&notes(1, 2.0, 64), &params(), &assigned, TPQ, BPM);
    let lex = lexical_durations(&segment.ph_seq, &segment.ph_dur);
    let s = lex.iter().find(|(p, _)| p == "s").expect("`s` in {lex:?}").1;
    let t = lex.iter().find(|(p, _)| p == "t").expect("`t` in {lex:?}").1;
    assert!(s > t, "fricative {s} should outlast stop {t}");
}

#[test]
fn phoneme_durations_fill_the_note_exactly() {
    // The f0 curve, the render-unit layout and the timeline offset are
    // all derived from cumulative `ph_dur`, so the allocator must neither
    // lose nor invent time — including in the over-full case, where it
    // scales the floors down rather than overrunning.
    for beats in [0.125, 0.25, 0.5, 1.0, 2.0] {
        let assigned = assign("resolution", 1);
        let ns = notes(1, beats, 64);
        let segment = build_segment(&ns, &params(), &assigned, TPQ, BPM);
        let total: f64 = segment.ph_dur.iter().sum();
        // Leading + trailing 0.3 s pads around the note's own slot.
        let slot = (60.0 / BPM as f64 * beats).max(0.05);
        let expected = 0.3 + slot + 0.3;
        assert!(
            (total - expected).abs() < 1e-6,
            "{beats} beats: ph_dur sums to {total}, expected {expected}"
        );
    }
}

#[test]
fn onset_lead_in_moves_the_boundary_without_changing_the_total() {
    // A syllable's onset consonants are pulled back across the previous
    // note's boundary so its vowel lands on the beat. That is a transfer
    // between two adjacent slots: the segment's total length — and hence
    // where the audio sits on the timeline — must not move.
    let text = g2p::auto_syllabify_text("resolution");
    let assigned = assign(&text, 4);
    let ns = notes(4, 1.0, 64);
    let segment = build_segment(&ns, &params(), &assigned, TPQ, BPM);
    let total: f64 = segment.ph_dur.iter().sum();
    let expected = 0.3 + 4.0 * (60.0 / BPM as f64) + 0.3;
    assert!(
        (total - expected).abs() < 1e-6,
        "lead-in changed the segment length: {total} vs {expected}"
    );
}

#[test]
fn min_articulation_grows_with_the_phoneme_count() {
    // The metric the report compares against: more phonemes on a note
    // always means more time needed. (Sanity — a client reasons about
    // `phoneme_count` directly.)
    let short = min_articulation_sec(&["r", "eh"]);
    let long = min_articulation_sec(&["r", "eh", "z", "ax", "l", "uw", "sh", "ax", "n"]);
    assert!(long > short * 2.0, "{long} vs {short}");
    assert_eq!(min_articulation_sec(&[]), 0.0);
}
