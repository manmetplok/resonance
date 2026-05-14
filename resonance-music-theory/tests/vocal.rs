//! Tests for the vocal lyric + melody generators.

use resonance_music_theory::{
    derive_vocal, generate_lyrics, Chord, ChordQuality, LyricLine, PitchClass, TimedChord,
    VocalContour, VocalMood, VocalParams, VocalPov, VocalRhymeScheme, VocalSinger, VocalStyle,
    VocalTimbre, VoiceType,
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

    let styles: std::collections::HashSet<_> =
        VocalStyle::ALL.iter().map(|s| s.as_str()).collect();
    assert_eq!(styles.len(), VocalStyle::ALL.len());
}

// ===========================================================================
// VocalStyle behavior — every style produces in-range, deterministic, one-
// note-per-syllable output.
// ===========================================================================

fn syllable_count(text: &str) -> u32 {
    let dots = text.matches('\u{00B7}').count() as u32;
    let words = text.split_whitespace().count() as u32;
    (dots + words).max(1)
}

#[test]
fn every_vocal_style_produces_one_note_per_syllable() {
    let mut p = VocalParams::default();
    p.draft = generate_lyrics(&p, 0xABCD);
    let total_syl: u32 = p.draft.iter().map(|l| syllable_count(&l.text)).sum();

    for style in VocalStyle::ALL.iter().copied() {
        let mut q = p.clone();
        q.style = style;
        let notes = derive_vocal(&b_minor_chords(), &q, 480, 0xABCD);
        assert_eq!(
            notes.len(),
            total_syl as usize,
            "{style} produced wrong note count: {} vs {} syllables",
            notes.len(),
            total_syl,
        );
    }
}

#[test]
fn every_vocal_style_stays_in_range() {
    let mut p = VocalParams::default();
    p.draft = generate_lyrics(&p, 0x9999);
    let (lo, hi) = p.range;

    for style in VocalStyle::ALL.iter().copied() {
        let mut q = p.clone();
        q.style = style;
        let notes = derive_vocal(&b_minor_chords(), &q, 480, 0x9999);
        for n in &notes {
            assert!(
                n.note >= lo && n.note <= hi,
                "{style} produced note {} outside [{}, {}]",
                n.note,
                lo,
                hi,
            );
        }
    }
}

#[test]
fn no_vocal_style_produces_overlapping_notes() {
    // Each input syllable maps to exactly one note in the SVS
    // pipeline, so notes must not overlap — overlapping notes mean
    // two syllables claim the same time window and the second's
    // pitch fights the first's tail in the rendered audio. Was a
    // real bug after phrase_start_offset was added (negative pickup
    // shifted line N+1 to start before line N's terminal sustain
    // ended).
    let mut p = VocalParams::default();
    p.draft = generate_lyrics(&p, 0xBA15ED);
    for style in VocalStyle::ALL.iter().copied() {
        let mut q = p.clone();
        q.style = style;
        // Multiple seeds to exercise the phrase_start_offset RNG path.
        for seed in [0xC0FFEE_u64, 0xDEADBEEF, 0xFA11_FA11, 1, 2, 3, 999] {
            let notes = derive_vocal(&b_minor_chords(), &q, 480, seed);
            for w in notes.windows(2) {
                let prev_end = w[0].start_tick + w[0].duration_ticks;
                assert!(
                    prev_end <= w[1].start_tick,
                    "{style} seed={seed:#x}: note at {} ends at {} but next note starts at {}",
                    w[0].start_tick,
                    prev_end,
                    w[1].start_tick,
                );
            }
        }
    }
}

#[test]
fn every_vocal_style_is_deterministic() {
    let mut p = VocalParams::default();
    p.draft = generate_lyrics(&p, 17);

    for style in VocalStyle::ALL.iter().copied() {
        let mut q = p.clone();
        q.style = style;
        let a = derive_vocal(&b_minor_chords(), &q, 480, 17);
        let b = derive_vocal(&b_minor_chords(), &q, 480, 17);
        assert_eq!(a.len(), b.len(), "{style} length differs across runs");
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.note, y.note, "{style} pitch differs");
            assert_eq!(x.start_tick, y.start_tick, "{style} timing differs");
        }
    }
}

#[test]
fn vocal_styles_produce_distinct_pitch_sequences() {
    // Different styles should pick different notes most of the time —
    // this catches regressions where a style accidentally falls back to
    // PopBallad's behavior.
    let mut p = VocalParams::default();
    p.draft = generate_lyrics(&p, 0x7777);

    let mut runs: Vec<(VocalStyle, Vec<u8>)> = Vec::new();
    for style in VocalStyle::ALL.iter().copied() {
        let mut q = p.clone();
        q.style = style;
        let notes = derive_vocal(&b_minor_chords(), &q, 480, 0x7777);
        runs.push((style, notes.iter().map(|n| n.note).collect()));
    }
    // Compare every pair — at least 80% of pairs should differ. Some
    // accidental similarity is OK (e.g. Hymnal and Conversational both
    // hover around the speaking pitch) but most pairs should diverge.
    let mut differ = 0usize;
    let mut total = 0usize;
    for i in 0..runs.len() {
        for j in (i + 1)..runs.len() {
            total += 1;
            if runs[i].1 != runs[j].1 {
                differ += 1;
            }
        }
    }
    assert!(
        differ as f32 / total as f32 >= 0.8,
        "expected ≥80% of style pairs to produce distinct sequences, got {}/{}",
        differ,
        total,
    );
}

#[test]
fn chant_style_uses_a_narrow_pitch_band() {
    // Chant should anchor on a single speaking pitch with at most a
    // ~5-semitone spread.
    let mut p = VocalParams::default();
    p.style = VocalStyle::Chant;
    p.draft = generate_lyrics(&p, 33);
    let notes = derive_vocal(&b_minor_chords(), &p, 480, 33);
    assert!(!notes.is_empty());
    let min = notes.iter().map(|n| n.note).min().unwrap();
    let max = notes.iter().map(|n| n.note).max().unwrap();
    assert!(
        max as i16 - min as i16 <= 6,
        "Chant should sit in a ≤6-semitone band, got [{}, {}] (spread {})",
        min,
        max,
        max as i16 - min as i16,
    );
}

#[test]
fn hymnal_style_uses_only_small_intervals() {
    // Hymnal walks stepwise — adjacent intervals should not exceed a
    // major third (4 semitones). The cap_interval helper tightens this
    // further in practice, but a major third is a comfortable upper
    // bound that catches any regression where Hymnal accidentally leaps.
    let mut p = VocalParams::default();
    p.style = VocalStyle::Hymnal;
    p.draft = generate_lyrics(&p, 55);
    let notes = derive_vocal(&b_minor_chords(), &p, 480, 55);
    assert!(notes.len() >= 2);
    for w in notes.windows(2) {
        let interval = (w[1].note as i16 - w[0].note as i16).abs();
        assert!(
            interval <= 4,
            "Hymnal step exceeded a major third: {} -> {} (Δ {})",
            w[0].note,
            w[1].note,
            interval,
        );
    }
}

#[test]
fn anthemic_style_uses_a_wider_range_than_chant() {
    // Sanity check: Anthemic should cover more of the available range
    // than Chant on the same lyrics + chords + seed.
    let mut base = VocalParams::default();
    base.draft = generate_lyrics(&base, 81);

    let mut anth = base.clone();
    anth.style = VocalStyle::Anthemic;
    let n_anth = derive_vocal(&b_minor_chords(), &anth, 480, 81);
    let spread_anth = n_anth.iter().map(|n| n.note).max().unwrap()
        - n_anth.iter().map(|n| n.note).min().unwrap();

    let mut chant = base.clone();
    chant.style = VocalStyle::Chant;
    let n_chant = derive_vocal(&b_minor_chords(), &chant, 480, 81);
    let spread_chant = n_chant.iter().map(|n| n.note).max().unwrap()
        - n_chant.iter().map(|n| n.note).min().unwrap();

    assert!(
        spread_anth > spread_chant,
        "Anthemic spread ({}) should exceed Chant spread ({})",
        spread_anth,
        spread_chant,
    );
}

#[test]
fn vocal_style_round_trips_through_serde() {
    let mut p = VocalParams::default();
    p.style = VocalStyle::Folk;
    let json = serde_json::to_string(&p).expect("serialize");
    let back: VocalParams = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.style, VocalStyle::Folk);
}

#[test]
fn vocal_singer_defaults_match_voice_type() {
    // The Default impl wires `singer = voice.default_singer()`. Spot-
    // check a couple of voice types so a regression in the mapping
    // (e.g. swapping Soprano and Bass) trips the test.
    let mut p = VocalParams::default();
    assert_eq!(p.voice, VoiceType::Alto);
    assert_eq!(p.singer, VocalSinger::Disco);

    p.voice = VoiceType::Soprano;
    let p2 = VocalParams {
        voice: VoiceType::Soprano,
        singer: VoiceType::Soprano.default_singer(),
        ..VocalParams::default()
    };
    assert_eq!(p2.singer, VocalSinger::Glam);
}

#[test]
fn vocal_singer_speaker_ids_are_unique_and_tiger_prefixed() {
    let ids: std::collections::HashSet<&str> =
        VocalSinger::ALL.iter().map(|s| s.speaker_id()).collect();
    assert_eq!(ids.len(), VocalSinger::ALL.len(), "duplicate speaker_id");
    for id in ids {
        assert!(
            id.starts_with("tiger_"),
            "speaker_id `{}` should start with tiger_",
            id
        );
    }
}

#[test]
fn vibrato_rate_and_singer_round_trip_through_serde() {
    let mut p = VocalParams::default();
    p.vibrato_rate = 6.5;
    p.singer = VocalSinger::Vinyl;
    let json = serde_json::to_string(&p).expect("serialize");
    let back: VocalParams = serde_json::from_str(&json).expect("deserialize");
    assert!((back.vibrato_rate - 6.5).abs() < 0.001);
    assert_eq!(back.singer, VocalSinger::Vinyl);
}

#[test]
fn vibrato_rate_and_singer_default_when_missing_from_json() {
    // Older saved projects don't carry vibrato_rate or singer; the
    // `#[serde(default = …)]` attributes should fill them in.
    let json = r#"{
        "theme": "test",
        "mood": "Yearning",
        "pov": "FirstSingular",
        "rhyme": "Abab",
        "lines": 4,
        "syllables_min": 7,
        "syllables_max": 11,
        "match_syllables_to_melody": true,
        "avoid_cliches": true,
        "draft": [],
        "voice": "Alto",
        "range": [55, 77],
        "contour": "Arch",
        "syllable_mode": "Syllabic",
        "chord_tone_anchor": 0.65,
        "leap_range": 0.15,
        "phrase_length_bars": 2,
        "breath": 0.45,
        "stay_in_scale": true,
        "avoid_clashes": true,
        "timbre": "Warm",
        "vibrato": 0.30,
        "articulation": 0.65,
        "consonant_emphasis": 0.40
    }"#;
    let p: VocalParams = serde_json::from_str(json).expect("deserialize legacy");
    // Vibrato rate falls back to the historic 5 Hz constant.
    assert!((p.vibrato_rate - 5.0).abs() < 0.001);
    // Singer falls back to Alto's default singer (Disco) — *not* the
    // file's own voice type, since we can't inspect the deserialized
    // voice from inside a `#[serde(default)]` callback.
    assert_eq!(p.singer, VocalSinger::Disco);
}

#[test]
fn vocal_style_defaults_to_pop_ballad_when_missing() {
    // Older saved projects don't have a `style` field; the
    // `#[serde(default)]` attribute should fill it in with PopBallad.
    let json = r#"{
        "theme": "test",
        "mood": "Yearning",
        "pov": "FirstSingular",
        "rhyme": "Abab",
        "lines": 4,
        "syllables_min": 7,
        "syllables_max": 11,
        "match_syllables_to_melody": true,
        "avoid_cliches": true,
        "draft": [],
        "voice": "Alto",
        "range": [55, 77],
        "contour": "Arch",
        "syllable_mode": "Syllabic",
        "chord_tone_anchor": 0.65,
        "leap_range": 0.15,
        "phrase_length_bars": 2,
        "breath": 0.45,
        "stay_in_scale": true,
        "avoid_clashes": true,
        "timbre": "Warm",
        "vibrato": 0.30,
        "articulation": 0.65,
        "consonant_emphasis": 0.40
    }"#;
    let p: VocalParams = serde_json::from_str(json).expect("deserialize legacy");
    assert_eq!(p.style, VocalStyle::PopBallad);
}
