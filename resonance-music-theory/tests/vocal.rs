//! Tests for the vocal lyric + melody generators.

use resonance_music_theory::{
    derive_vocal, generate_lyrics, Chord, ChordQuality, LyricLine, PitchClass, TimedChord,
    VocalContour, VocalMood, VocalParams, VocalPov, VocalRhymeScheme, VocalTimbre, VoiceType,
};

fn b_minor_chords() -> Vec<TimedChord> {
    vec![
        TimedChord {
            chord: Chord::new(PitchClass::B, ChordQuality::Min),
            start_beat: 0,
            duration_beats: 4,
        },
        TimedChord {
            chord: Chord::new(PitchClass::Fs, ChordQuality::Maj),
            start_beat: 4,
            duration_beats: 4,
        },
        TimedChord {
            chord: Chord::new(PitchClass::G, ChordQuality::Maj),
            start_beat: 8,
            duration_beats: 4,
        },
        TimedChord {
            chord: Chord::new(PitchClass::E, ChordQuality::Min),
            start_beat: 12,
            duration_beats: 4,
        },
    ]
}

#[test]
fn generate_lyrics_returns_requested_line_count() {
    let mut p = VocalParams::default();
    p.lines = 4;
    p.draft = Vec::new();
    let draft = generate_lyrics(&p, 0xC0FFEE);
    assert_eq!(draft.len(), 4, "should generate exactly `lines` lines");
}

#[test]
fn generate_lyrics_is_deterministic_for_same_seed() {
    let mut p = VocalParams::default();
    p.draft = Vec::new();
    let a = generate_lyrics(&p, 0xC0FFEE);
    let b = generate_lyrics(&p, 0xC0FFEE);
    assert_eq!(
        a.iter().map(|l| l.text.clone()).collect::<Vec<_>>(),
        b.iter().map(|l| l.text.clone()).collect::<Vec<_>>(),
        "same seed should yield identical lyric set"
    );
}

#[test]
fn generate_lyrics_varies_with_seed() {
    let mut p = VocalParams::default();
    p.draft = Vec::new();
    p.lines = 4;
    let a = generate_lyrics(&p, 0xAAAA_AAAA);
    let b = generate_lyrics(&p, 0xBBBB_BBBB);
    // Allow rare collisions but require *some* line to differ across
    // distant seeds.
    let any_diff = a.iter().zip(b.iter()).any(|(x, y)| x.text != y.text);
    assert!(any_diff, "different seeds should produce different lyrics");
}

#[test]
fn generate_lyrics_respects_abab_rhyme_pattern() {
    let mut p = VocalParams::default();
    p.rhyme = VocalRhymeScheme::Abab;
    p.lines = 4;
    p.draft = Vec::new();
    let draft = generate_lyrics(&p, 0xC0FFEE_BEEF);
    assert_eq!(draft.len(), 4);
    assert_eq!(draft[0].rhyme, 'A');
    assert_eq!(draft[1].rhyme, 'B');
    assert_eq!(draft[2].rhyme, 'A');
    assert_eq!(draft[3].rhyme, 'B');
}

#[test]
fn generate_lyrics_respects_aabb_rhyme_pattern() {
    let mut p = VocalParams::default();
    p.rhyme = VocalRhymeScheme::Aabb;
    p.lines = 4;
    p.draft = Vec::new();
    let draft = generate_lyrics(&p, 0xDEAD_BEEF);
    assert_eq!(
        draft.iter().map(|l| l.rhyme).collect::<Vec<_>>(),
        vec!['A', 'A', 'B', 'B']
    );
}

#[test]
fn generate_lyrics_preserves_locked_line() {
    let locked = LyricLine {
        n: 1,
        rhyme: 'A',
        syllables: 11,
        text: "Glass hou\u{00B7}ses don't break, they just re\u{00B7}mem\u{00B7}ber".into(),
        locked: true,
    };
    let mut p = VocalParams::default();
    p.draft = vec![locked.clone()];
    p.lines = 4;
    let draft = generate_lyrics(&p, 0xFA11_FA11);
    assert_eq!(draft[0].text, locked.text);
    assert!(draft[0].locked);
}

#[test]
fn generate_lyrics_filters_by_mood() {
    let mut p = VocalParams::default();
    p.mood = VocalMood::Joyful;
    p.draft = Vec::new();
    p.lines = 8;
    // Wide syllable range to keep the joyful pool intact.
    p.syllables_min = 3;
    p.syllables_max = 24;
    let draft = generate_lyrics(&p, 0xCAFE);
    // No line should equal one of the yearning-only corpus entries.
    let yearning_lines = [
        "I hold the days I can\u{00B7}not say",
        "Glass hou\u{00B7}ses don't break, they just re\u{00B7}mem\u{00B7}ber",
        "ev\u{00B7}ry stone we threw on the way",
    ];
    for line in &draft {
        assert!(
            !yearning_lines.contains(&line.text.as_str()),
            "joyful mood should not pick yearning-only lines, got {:?}",
            line.text
        );
    }
}

#[test]
fn generate_lyrics_respects_broad_syllable_range() {
    // With a generous window that comfortably covers every corpus line,
    // every emitted line should fall inside it.
    let mut p = VocalParams::default();
    p.draft = Vec::new();
    p.syllables_min = 7;
    p.syllables_max = 12;
    p.lines = 4;
    let draft = generate_lyrics(&p, 0x12345);
    for l in &draft {
        assert!(
            (7..=12).contains(&l.syllables),
            "syllable count {} out of [7, 12] for line {:?}",
            l.syllables,
            l.text
        );
    }
}

#[test]
fn generate_lyrics_falls_back_when_range_is_tight() {
    // Tight 9–10 range; the generator may have to widen for some
    // buckets, but every line should still pick from the rhyme bucket
    // it was assigned. We just check the generator doesn't panic and
    // returns the right number of lines.
    let mut p = VocalParams::default();
    p.draft = Vec::new();
    p.syllables_min = 9;
    p.syllables_max = 10;
    p.lines = 4;
    let draft = generate_lyrics(&p, 0x12345);
    assert_eq!(draft.len(), 4);
}

#[test]
fn derive_vocal_returns_one_note_per_syllable_for_syllabic_mode() {
    let mut p = VocalParams::default();
    let draft = generate_lyrics(&p, 0xABCD);
    // `derive_vocal` counts syllables mechanically (separators +
    // whitespace) rather than trusting the corpus's `syllables` field,
    // so the test must do the same — otherwise it'll diverge whenever
    // a corpus line's stored count drifts from its actual text.
    let mechanical = |text: &str| -> u32 {
        let dots = text.matches('\u{00B7}').count() as u32;
        let words = text.split_whitespace().count() as u32;
        (dots + words).max(1)
    };
    let total_syl: u32 = draft.iter().map(|l| mechanical(&l.text)).sum();
    p.draft = draft;
    let notes = derive_vocal(&b_minor_chords(), &p, 480, 0xABCD);
    assert_eq!(
        notes.len(),
        total_syl as usize,
        "expected one note per syllable in Syllabic mode"
    );
}

#[test]
fn derive_vocal_stays_in_range() {
    let mut p = VocalParams::default();
    p.draft = generate_lyrics(&p, 7);
    let (lo, hi) = p.range;
    let notes = derive_vocal(&b_minor_chords(), &p, 480, 7);
    assert!(!notes.is_empty(), "expected some notes");
    for n in &notes {
        assert!(
            n.note >= lo && n.note <= hi,
            "note {} fell outside range [{}, {}]",
            n.note,
            lo,
            hi
        );
    }
}

#[test]
fn derive_vocal_is_deterministic() {
    let mut p = VocalParams::default();
    p.draft = generate_lyrics(&p, 42);
    let a = derive_vocal(&b_minor_chords(), &p, 480, 42);
    let b = derive_vocal(&b_minor_chords(), &p, 480, 42);
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(b.iter()) {
        assert_eq!(x.note, y.note);
        assert_eq!(x.start_tick, y.start_tick);
        assert_eq!(x.duration_ticks, y.duration_ticks);
        assert!((x.velocity - y.velocity).abs() < 0.001);
    }
}

#[test]
fn derive_vocal_respects_voice_type_range() {
    let mut p = VocalParams::default();
    p.voice = VoiceType::Bass;
    p.range = VoiceType::Bass.default_range();
    p.draft = generate_lyrics(&p, 9);
    let notes = derive_vocal(&b_minor_chords(), &p, 480, 9);
    let (lo, hi) = p.range;
    for n in &notes {
        assert!(n.note >= lo && n.note <= hi, "bass voice violates range");
    }
    // Sanity — bass voice should not be silly-high.
    assert!(
        notes.iter().all(|n| n.note < 72),
        "bass voice produced notes ≥ C5"
    );
}

#[test]
fn derive_vocal_emits_nothing_for_empty_chords_or_draft() {
    let p = VocalParams::default();
    let empty_chords: Vec<TimedChord> = vec![];
    let notes = derive_vocal(&empty_chords, &p, 480, 0);
    assert!(notes.is_empty(), "empty chord list should produce no notes");

    let mut p2 = VocalParams::default();
    p2.draft = Vec::new();
    let notes2 = derive_vocal(&b_minor_chords(), &p2, 480, 0);
    assert!(
        notes2.is_empty(),
        "empty draft should produce no notes (no syllables to place)"
    );
}

#[test]
fn derive_vocal_velocity_is_within_bounds() {
    let mut p = VocalParams::default();
    p.draft = generate_lyrics(&p, 1);
    let notes = derive_vocal(&b_minor_chords(), &p, 480, 1);
    for n in &notes {
        assert!(
            (0.4..=1.0).contains(&n.velocity),
            "velocity {} out of [0.4, 1.0]",
            n.velocity
        );
    }
}

#[test]
fn vocal_params_round_trip_through_serde_json() {
    let p = VocalParams::default();
    let json = serde_json::to_string(&p).expect("serialize VocalParams");
    let back: VocalParams = serde_json::from_str(&json).expect("deserialize VocalParams");
    assert_eq!(back.lines, p.lines);
    assert_eq!(back.voice, p.voice);
    assert_eq!(back.timbre, p.timbre);
    assert_eq!(back.rhyme, p.rhyme);
    assert_eq!(back.range, p.range);
}

#[test]
fn vocal_enum_variants_have_unique_string_labels() {
    // Catch regressions where two `as_str` arms duplicate a label.
    let moods: std::collections::HashSet<_> =
        VocalMood::ALL.iter().map(|m| m.as_str()).collect();
    assert_eq!(moods.len(), VocalMood::ALL.len());

    let povs: std::collections::HashSet<_> =
        VocalPov::ALL.iter().map(|p| p.as_str()).collect();
    assert_eq!(povs.len(), VocalPov::ALL.len());

    let voices: std::collections::HashSet<_> =
        VoiceType::ALL.iter().map(|v| v.as_str()).collect();
    assert_eq!(voices.len(), VoiceType::ALL.len());

    let timbres: std::collections::HashSet<_> =
        VocalTimbre::ALL.iter().map(|t| t.as_str()).collect();
    assert_eq!(timbres.len(), VocalTimbre::ALL.len());

    let contours: std::collections::HashSet<_> =
        VocalContour::ALL.iter().map(|c| c.as_str()).collect();
    assert_eq!(contours.len(), VocalContour::ALL.len());
}
