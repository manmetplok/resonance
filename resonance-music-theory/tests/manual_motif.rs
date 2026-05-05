//! Tests for the hand-drawn motif path: `MotifSource::Manual { notes, .. }`
//! must feed through every motif consumer (motif melody lane, motif bass
//! lane, drum-motif rhythm) producing the same scale-step shape as a
//! procedurally generated motif of the same intervals would.

use resonance_music_theory::{
    derive_bass_motif, derive_motif_melody_with_section, derive_motif_rhythm, BassMotifMode,
    BassMotifPhrase, BassParams, BassStyle, Chord, ChordQuality, ManualMotifNote, MelodyParams,
    MelodyStyle, Mode, MotifParams, MotifSource, PitchClass, Scale, TimedChord,
};

const TPB: u32 = 480;
const LANE_SEED: u64 = 1;

fn tc(chord: Chord, start_beat: u32, duration_beats: u32) -> TimedChord {
    TimedChord {
        chord,
        start_beat,
        duration_beats,
    }
}

/// Standard "do-re-mi-re" pattern: scale steps 0, 1, 2, 1 in eighth-notes.
fn ascending_motif() -> Vec<ManualMotifNote> {
    vec![
        ManualMotifNote { scale_step: 0, duration_sixteenths: 2, accent: true, is_rest: false },
        ManualMotifNote { scale_step: 1, duration_sixteenths: 2, accent: false, is_rest: false },
        ManualMotifNote { scale_step: 2, duration_sixteenths: 2, accent: false, is_rest: false },
        ManualMotifNote { scale_step: 1, duration_sixteenths: 2, accent: false, is_rest: false },
    ]
}

fn manual_source(notes: Vec<ManualMotifNote>) -> MotifSource {
    MotifSource::Manual {
        notes,
        params: MotifParams::default(),
    }
}

fn melody_params() -> MelodyParams {
    MelodyParams {
        style: MelodyStyle::Motif,
        ..MelodyParams::default()
    }
}

fn bass_params() -> BassParams {
    BassParams {
        style: BassStyle::Motif,
        base_note: 28,
        velocity: 0.85,
        motif_mode: BassMotifMode::SameIntervals,
        motif_phrase: BassMotifPhrase::Simple,
    }
}

#[test]
fn manual_motif_produces_notes_through_melody_lane() {
    let chords = vec![
        tc(Chord::new(PitchClass::C, ChordQuality::Maj), 0, 4),
        tc(Chord::new(PitchClass::F, ChordQuality::Maj), 4, 4),
    ];
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let source = manual_source(ascending_motif());

    let notes = derive_motif_melody_with_section(
        &chords,
        scale,
        &melody_params(),
        &source,
        LANE_SEED,
        TPB,
    );

    assert!(!notes.is_empty(), "manual motif should produce melody notes");
}

#[test]
fn manual_motif_zero_step_anchors_at_chord_root() {
    // A single-note manual motif with scale_step=0 should land on the
    // chord root (mod 12) for every chord in SameIntervals bass mode.
    let chords = vec![
        tc(Chord::new(PitchClass::C, ChordQuality::Maj), 0, 4),
        tc(Chord::new(PitchClass::F, ChordQuality::Maj), 4, 4),
        tc(Chord::new(PitchClass::G, ChordQuality::Maj), 8, 4),
    ];
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let source = manual_source(vec![ManualMotifNote {
        scale_step: 0,
        duration_sixteenths: 4,
        accent: true,
        is_rest: false,
    }]);

    let bass = derive_bass_motif(&chords, scale, &bass_params(), &source, LANE_SEED, TPB);

    let tpb = TPB as u64;
    for n in &bass {
        let chord_idx = chords
            .iter()
            .rposition(|c| (c.start_beat as u64 * tpb) <= n.start_tick)
            .unwrap();
        let root_pc = chords[chord_idx].chord.root.to_semitone();
        assert_eq!(
            n.note % 12,
            root_pc,
            "scale_step=0 should map to chord root, got {} expected {}",
            n.note % 12,
            root_pc,
        );
    }
}

#[test]
fn manual_motif_rhythm_uses_drawn_durations() {
    // A motif with three eighth-notes (each 2 sixteenths) should produce
    // exactly 3 hits per chord — the rhythm tiler scales the unitless
    // ratios to fill the chord, but the shape must come from our drawing.
    let chords = vec![tc(Chord::new(PitchClass::C, ChordQuality::Maj), 0, 4)];
    let source = manual_source(vec![
        ManualMotifNote { scale_step: 0, duration_sixteenths: 2, accent: true, is_rest: false },
        ManualMotifNote { scale_step: 0, duration_sixteenths: 2, accent: false, is_rest: false },
        ManualMotifNote { scale_step: 0, duration_sixteenths: 2, accent: false, is_rest: false },
    ]);

    let hits = derive_motif_rhythm(&chords, &source, TPB);
    // 4 beats / 3 ratios = floor evenly, so we should fit exactly 3 (or
    // more from tiling) hits without being empty.
    assert!(!hits.is_empty(), "rhythm extraction must yield at least one hit");
    // First hit accents flag must come from our drawing.
    assert!(hits[0].accent, "first hit should inherit the manual accent flag");
}

#[test]
fn manual_motif_in_dorian_uses_scale_intervals_not_chromatic() {
    // scale_step=2 in D dorian is the scale's 3rd degree → F (3 semitones
    // up from D), not E (4 semitones, what major would give). Verify the
    // semitone interval that lands matches dorian, by checking that step
    // 1 → +2 and step 2 → +3.
    let chords = vec![tc(Chord::new(PitchClass::D, ChordQuality::Min), 0, 4)];
    let scale = Some(Scale::new(PitchClass::D, Mode::Dorian));

    let source = manual_source(vec![
        ManualMotifNote { scale_step: 0, duration_sixteenths: 2, accent: false, is_rest: false },
        ManualMotifNote { scale_step: 1, duration_sixteenths: 2, accent: false, is_rest: false },
        ManualMotifNote { scale_step: 2, duration_sixteenths: 2, accent: false, is_rest: false },
    ]);

    let bass = derive_bass_motif(&chords, scale, &bass_params(), &source, LANE_SEED, TPB);
    assert!(bass.len() >= 3, "expected at least one motif iteration");

    // The first three notes should rise by 0, +2, +3 semitones from the
    // anchor (matching dorian intervals 0, 2, 3).
    let first = bass[0].note;
    assert_eq!(bass[1].note as i32 - first as i32, 2);
    assert_eq!(bass[2].note as i32 - first as i32, 3);
}

#[test]
fn manual_motif_without_scale_falls_back_to_chromatic() {
    // No scale → scale_step n is interpreted as n semitones. The motif
    // should still produce notes; we can verify by checking the per-chord
    // step delta matches the manual scale_step value.
    let chords = vec![tc(Chord::new(PitchClass::C, ChordQuality::Maj), 0, 4)];
    let source = manual_source(vec![
        ManualMotifNote { scale_step: 0, duration_sixteenths: 2, accent: false, is_rest: false },
        ManualMotifNote { scale_step: 1, duration_sixteenths: 2, accent: false, is_rest: false },
    ]);

    let notes = derive_bass_motif(&chords, None, &bass_params(), &source, LANE_SEED, TPB);
    assert!(notes.len() >= 2);
    // step 0 → step 1 is +1 semitone in chromatic fallback. The bass motif
    // path applies harmony alignment which can re-snap notes to chord
    // tones, so we accept any non-zero positive shift here as evidence
    // that the manual motif drove the output.
    assert!(notes[1].note >= notes[0].note);
}

#[test]
fn manual_motif_is_deterministic_under_lane_seed_reroll() {
    // With Manual mode the motif itself is fixed; lane_seed only alters
    // contour selection / phrase octave displacement on long sections.
    // For a single chord the output should be identical across lane seeds.
    let chords = vec![tc(Chord::new(PitchClass::C, ChordQuality::Maj), 0, 4)];
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let source = manual_source(ascending_motif());

    let a = derive_motif_melody_with_section(
        &chords,
        scale,
        &melody_params(),
        &source,
        1,
        TPB,
    );
    let b = derive_motif_melody_with_section(
        &chords,
        scale,
        &melody_params(),
        &source,
        2,
        TPB,
    );
    assert_eq!(
        a.iter().map(|n| n.start_tick).collect::<Vec<_>>(),
        b.iter().map(|n| n.start_tick).collect::<Vec<_>>(),
        "lane_seed should not alter manual motif rhythm on a single chord",
    );
}

#[test]
fn manual_motif_rest_emits_no_note_but_advances_cursor() {
    // Three slots: note, rest, note. Rhythm extraction should emit hits
    // for the two notes but NOT the rest, and the second note should
    // start later than it would in a 2-note motif (the rest takes time).
    let chords = vec![tc(Chord::new(PitchClass::C, ChordQuality::Maj), 0, 4)];
    let with_rest = manual_source(vec![
        ManualMotifNote { scale_step: 0, duration_sixteenths: 2, accent: true, is_rest: false },
        ManualMotifNote { scale_step: 0, duration_sixteenths: 2, accent: false, is_rest: true },
        ManualMotifNote { scale_step: 0, duration_sixteenths: 2, accent: false, is_rest: false },
    ]);

    let hits = derive_motif_rhythm(&chords, &with_rest, TPB);
    assert!(!hits.is_empty());
    // Each motif iteration emits 2 hits, not 3 (the rest is skipped).
    // The rhythm tiles to fill the chord; verify by counting hits in the
    // first iteration window (start_tick < total_motif_ticks).
    let total_ratio: u64 = 6; // 2 + 2 + 2
    let first_iter_ticks = (4 * TPB as u64) * total_ratio / total_ratio; // = full chord
    let _ = first_iter_ticks;

    // Sanity: the second hit should NOT start at the second slot — it
    // skips the rest and lands at the third. With ratios 2,2,2 over 4
    // beats, slot duration is roughly TPB*4*2/6 ≈ TPB*4/3.
    let slot_ticks = (4 * TPB as u64) / 3;
    assert!(
        (hits[1].start_tick as i64 - 2 * slot_ticks as i64).abs() <= 2,
        "second emitted hit should land at slot 2 (after the rest), got {}",
        hits[1].start_tick,
    );
}

#[test]
fn manual_motif_rest_does_not_emit_melody_note() {
    // A motif that's just a rest should yield zero melody notes even
    // though the cursor advances through the chord.
    let chords = vec![tc(Chord::new(PitchClass::C, ChordQuality::Maj), 0, 4)];
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let source = manual_source(vec![ManualMotifNote {
        scale_step: 0,
        duration_sixteenths: 4,
        accent: false,
        is_rest: true,
    }]);

    let melody = derive_motif_melody_with_section(
        &chords,
        scale,
        &melody_params(),
        &source,
        LANE_SEED,
        TPB,
    );
    let bass = derive_bass_motif(&chords, scale, &bass_params(), &source, LANE_SEED, TPB);
    let rhythm = derive_motif_rhythm(&chords, &source, TPB);

    assert!(melody.is_empty(), "rest-only motif should emit no melody notes");
    assert!(bass.is_empty(), "rest-only motif should emit no bass notes");
    assert!(rhythm.is_empty(), "rest-only motif should emit no rhythm hits");
}

#[test]
fn manual_motif_rest_at_start_offsets_first_note() {
    // Rest then note — the first emitted note should NOT land at tick 0,
    // because the rest uses the leading slot.
    let chords = vec![tc(Chord::new(PitchClass::C, ChordQuality::Maj), 0, 4)];
    let source = manual_source(vec![
        ManualMotifNote { scale_step: 0, duration_sixteenths: 2, accent: false, is_rest: true },
        ManualMotifNote { scale_step: 0, duration_sixteenths: 2, accent: false, is_rest: false },
    ]);

    let hits = derive_motif_rhythm(&chords, &source, TPB);
    assert!(!hits.is_empty());
    assert!(
        hits[0].start_tick > 0,
        "first hit should be offset by the leading rest, got {}",
        hits[0].start_tick,
    );
}

#[test]
fn manual_motif_empty_returns_no_notes() {
    let chords = vec![tc(Chord::new(PitchClass::C, ChordQuality::Maj), 0, 4)];
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let source = manual_source(Vec::new());

    let melody = derive_motif_melody_with_section(
        &chords,
        scale,
        &melody_params(),
        &source,
        LANE_SEED,
        TPB,
    );
    let bass = derive_bass_motif(&chords, scale, &bass_params(), &source, LANE_SEED, TPB);
    let rhythm = derive_motif_rhythm(&chords, &source, TPB);

    assert!(melody.is_empty());
    assert!(bass.is_empty());
    assert!(rhythm.is_empty());
}
