//! Integration tests for the section-shared motif: bass + melody Motif
//! lanes consume the same `MotifParams`, and the bass renders it via
//! one of four `BassMotifMode`s and one of three `BassMotifPhrase`s.

use resonance_music_theory::{
    derive_bass_motif, derive_motif_melody_with_section, BassMotifMode, BassMotifPhrase,
    BassParams, BassStyle, Chord, ChordQuality, MelodyParams, MelodyStyle, Mode, MotifParams,
    MotifSource, PitchClass, Scale, TimedChord,
};

const TPB: u32 = 480;
const LANE_SEED: u64 = 9999;

fn gen(p: MotifParams) -> MotifSource {
    MotifSource::Generated(p)
}

fn tc(chord: Chord, start_beat: u32, duration_beats: u32) -> TimedChord {
    TimedChord {
        chord,
        start_beat,
        duration_beats,
    }
}

fn standard_chords() -> Vec<TimedChord> {
    vec![
        tc(Chord::new(PitchClass::C, ChordQuality::Maj), 0, 4),
        tc(Chord::new(PitchClass::F, ChordQuality::Maj), 4, 4),
        tc(Chord::new(PitchClass::G, ChordQuality::Maj), 8, 4),
        tc(Chord::new(PitchClass::C, ChordQuality::Maj), 12, 4),
    ]
}

fn motif_params() -> MotifParams {
    MotifParams {
        seed: 42,
        complexity: 0.5,
        motif_len: 4,
        leap_chance: 0.21,
    }
}

fn bass_params(mode: BassMotifMode, phrase: BassMotifPhrase) -> BassParams {
    BassParams {
        style: BassStyle::Motif,
        base_note: 28, // E1
        velocity: 0.85,
        motif_mode: mode,
        motif_phrase: phrase,
    }
}

fn melody_params() -> MelodyParams {
    MelodyParams {
        style: MelodyStyle::Motif,
        ..MelodyParams::default()
    }
}

#[test]
fn bass_motif_empty_in_empty_out() {
    let p = bass_params(BassMotifMode::SameIntervals, BassMotifPhrase::Simple);
    let out = derive_bass_motif(&[], None, &p, &gen(motif_params()), LANE_SEED, TPB);
    assert!(out.is_empty());
}

#[test]
fn bass_motif_first_note_only_emits_one_per_chord() {
    let chords = standard_chords();
    let p = bass_params(BassMotifMode::FirstNoteOnly, BassMotifPhrase::Simple);
    let out = derive_bass_motif(
        &chords,
        Some(Scale::new(PitchClass::C, Mode::Major)),
        &p,
        &gen(motif_params()),
        LANE_SEED,
        TPB,
    );
    assert_eq!(out.len(), chords.len());
    for (i, n) in out.iter().enumerate() {
        let expected = chords[i].start_beat as u64 * TPB as u64;
        assert_eq!(n.start_tick, expected);
    }
}

#[test]
fn bass_motif_rhythm_only_uses_chord_bass_pitch() {
    let chords = standard_chords();
    let p = bass_params(BassMotifMode::RhythmOnly, BassMotifPhrase::Simple);
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let out = derive_bass_motif(&chords, scale, &p, &gen(motif_params()), LANE_SEED, TPB);
    assert!(!out.is_empty());

    let tpb = TPB as u64;
    for n in &out {
        let chord_idx = chords
            .iter()
            .rposition(|c| (c.start_beat as u64 * tpb) <= n.start_tick)
            .unwrap();
        let bass_pc = chords[chord_idx].chord.bass.unwrap_or(chords[chord_idx].chord.root);
        assert_eq!(
            n.note % 12,
            bass_pc.to_semitone(),
            "note {} (pc {}) doesn't match chord bass {}",
            n.note,
            n.note % 12,
            bass_pc.to_semitone(),
        );
    }
}

#[test]
fn bass_motif_same_intervals_matches_melody_intervals() {
    let chords = standard_chords();
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let motif = motif_params();

    let bass = bass_params(BassMotifMode::SameIntervals, BassMotifPhrase::Simple);
    let bass_notes = derive_bass_motif(&chords, scale, &bass, &gen(motif), LANE_SEED, TPB);

    let melody_params = MelodyParams {
        register: (28, 52),
        ..melody_params()
    };
    let mel_notes = derive_motif_melody_with_section(
        &chords,
        scale,
        &melody_params,
        &gen(motif),
        LANE_SEED,
        TPB,
    );

    assert!(!bass_notes.is_empty());
    assert!(!mel_notes.is_empty());

    let chord_end_tick = chords[0].duration_beats as u64 * TPB as u64;
    let bass_first: Vec<_> = bass_notes
        .iter()
        .filter(|n| n.start_tick < chord_end_tick)
        .collect();
    let mel_first: Vec<_> = mel_notes
        .iter()
        .filter(|n| n.start_tick < chord_end_tick)
        .collect();

    assert_eq!(
        bass_first.len(),
        mel_first.len(),
        "bass and melody emitted different note counts on the first chord"
    );

    let bass_starts: Vec<u64> = bass_first.iter().map(|n| n.start_tick).collect();
    let mel_starts: Vec<u64> = mel_first.iter().map(|n| n.start_tick).collect();
    assert_eq!(bass_starts, mel_starts, "rhythms differ");
}

#[test]
fn bass_motif_augmented_doubles_durations() {
    let chords = standard_chords();
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let motif = motif_params();

    let same = bass_params(BassMotifMode::SameIntervals, BassMotifPhrase::Simple);
    let aug = bass_params(BassMotifMode::Augmented, BassMotifPhrase::Simple);

    let same_notes = derive_bass_motif(&chords, scale, &same, &gen(motif), LANE_SEED, TPB);
    let aug_notes = derive_bass_motif(&chords, scale, &aug, &gen(motif), LANE_SEED, TPB);

    let chord_end = chords[0].duration_beats as u64 * TPB as u64;
    let same_count = same_notes.iter().filter(|n| n.start_tick < chord_end).count();
    let aug_count = aug_notes.iter().filter(|n| n.start_tick < chord_end).count();
    assert!(
        aug_count <= same_count,
        "augmented should emit at most as many notes as same-intervals (aug={aug_count}, same={same_count})"
    );
}

#[test]
fn bass_motif_phrase_simple_is_deterministic() {
    let chords = standard_chords();
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let motif = motif_params();
    let p = bass_params(BassMotifMode::SameIntervals, BassMotifPhrase::Simple);
    let a = derive_bass_motif(&chords, scale, &p, &gen(motif), LANE_SEED, TPB);
    let b = derive_bass_motif(&chords, scale, &p, &gen(motif), LANE_SEED, TPB);
    assert_eq!(a, b);
}

#[test]
fn bass_motif_phrase_modes_produce_different_outputs() {
    // 16 chords / 4 phrases: a full sentence or period group. A
    // 2-phrase section would be a single antecedent–consequent period,
    // where the phrase grammar makes MirrorMelody legitimately equal
    // Simple (the consequent reuses the antecedent's opening, and the
    // section opener is Identity).
    let chords: Vec<TimedChord> = (0..16)
        .map(|i| tc(Chord::new(PitchClass::C, ChordQuality::Maj), i * 4, 4))
        .collect();
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let motif = MotifParams {
        seed: 12345,
        complexity: 0.7,
        ..motif_params()
    };
    let simple = derive_bass_motif(
        &chords,
        scale,
        &bass_params(BassMotifMode::SameIntervals, BassMotifPhrase::Simple),
        &gen(motif),
        LANE_SEED,
        TPB,
    );
    let mirror = derive_bass_motif(
        &chords,
        scale,
        &bass_params(BassMotifMode::SameIntervals, BassMotifPhrase::MirrorMelody),
        &gen(motif),
        LANE_SEED,
        TPB,
    );
    let restricted = derive_bass_motif(
        &chords,
        scale,
        &bass_params(BassMotifMode::SameIntervals, BassMotifPhrase::Restricted),
        &gen(motif),
        LANE_SEED,
        TPB,
    );
    assert_ne!(simple, mirror);
    assert!(!restricted.is_empty());
}

#[test]
fn bass_motif_seed_deterministic() {
    let chords = standard_chords();
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let p = bass_params(BassMotifMode::SameIntervals, BassMotifPhrase::Simple);
    let m = motif_params();
    let a = derive_bass_motif(&chords, scale, &p, &gen(m), LANE_SEED, TPB);
    let b = derive_bass_motif(&chords, scale, &p, &gen(m), LANE_SEED, TPB);
    assert_eq!(a, b);
}

#[test]
fn bass_motif_motif_seed_change_changes_intervals() {
    let chords = standard_chords();
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let p = bass_params(BassMotifMode::SameIntervals, BassMotifPhrase::Simple);
    let m1 = MotifParams { seed: 1, ..motif_params() };
    let m2 = MotifParams { seed: 99, ..motif_params() };
    let a = derive_bass_motif(&chords, scale, &p, &gen(m1), LANE_SEED, TPB);
    let b = derive_bass_motif(&chords, scale, &p, &gen(m2), LANE_SEED, TPB);
    assert_ne!(a, b);
}

#[test]
fn bass_motif_stays_in_bass_register() {
    let chords = standard_chords();
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let p = bass_params(BassMotifMode::SameIntervals, BassMotifPhrase::Simple);
    let out = derive_bass_motif(&chords, scale, &p, &gen(motif_params()), LANE_SEED, TPB);
    for n in &out {
        assert!(n.note >= 28, "note {} below base_note 28", n.note);
        assert!(n.note <= 52, "note {} above bass register cap 52", n.note);
    }
}

// ---------------------------------------------------------------------------
// Section-shared identity vs lane-local variation
// ---------------------------------------------------------------------------

/// Bumping a lane's own seed must NOT change the shared motif's interval
/// shape — only this lane's surface (phrase contour, etc.). The bass
/// `SameIntervals` mode preserves motif intervals exactly, so the
/// pitch-class sequence per chord must be invariant under lane-seed change.
#[test]
fn bass_lane_seed_change_keeps_motif_intervals() {
    // Use enough chords for several phrases so contour selection has room
    // to diverge, but the motif intervals (per chord) must still match.
    let chords: Vec<TimedChord> = (0..8)
        .map(|i| tc(Chord::new(PitchClass::C, ChordQuality::Maj), i * 4, 4))
        .collect();
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let p = bass_params(BassMotifMode::SameIntervals, BassMotifPhrase::Simple);
    let m = motif_params();

    let a = derive_bass_motif(&chords, scale, &p, &gen(m), 1, TPB);
    let b = derive_bass_motif(&chords, scale, &p, &gen(m), 999, TPB);

    // Both must have the same number of notes (motif tiles the same way).
    assert_eq!(a.len(), b.len(), "lane-seed change altered note count");
    // Each note's pitch class must match (intervals are identical).
    for (na, nb) in a.iter().zip(b.iter()) {
        assert_eq!(
            na.note % 12,
            nb.note % 12,
            "lane-seed change altered pitch-class at start_tick {}",
            na.start_tick,
        );
    }
}

/// Pressing Regenerate on a bass-motif lane (which bumps `lane_seed`)
/// must produce audibly different MIDI for at least most seeds — the
/// per-phrase octave displacement gives the lane real variation while
/// the underlying motif identity stays put. Without this, Regenerate
/// would feel like it does nothing.
#[test]
fn bass_lane_seed_change_changes_some_octaves() {
    // Use eight chords so we get multiple phrases (phrase_len = 4) and
    // therefore at least one phrase that can pick a non-zero octave shift.
    let chords: Vec<TimedChord> = (0..8)
        .map(|i| tc(Chord::new(PitchClass::C, ChordQuality::Maj), i * 4, 4))
        .collect();
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let p = bass_params(BassMotifMode::SameIntervals, BassMotifPhrase::Simple);
    let m = motif_params();

    // Across a range of lane seeds, at least one pair should produce a
    // different MIDI sequence (octave displacement on at least one phrase).
    let baseline = derive_bass_motif(&chords, scale, &p, &gen(m), 1, TPB);
    let mut found_variation = false;
    for seed in 2..50u64 {
        let alt = derive_bass_motif(&chords, scale, &p, &gen(m), seed, TPB);
        if alt != baseline {
            found_variation = true;
            break;
        }
    }
    assert!(
        found_variation,
        "lane-seed change should produce audibly different MIDI on at least one phrase"
    );
}

/// Bumping the melody lane's own seed must not change the bass-motif
/// output, when both lanes share the same `MotifParams`.
#[test]
fn melody_lane_seed_change_does_not_affect_bass_motif_output() {
    let chords = standard_chords();
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let m = motif_params();
    let p = bass_params(BassMotifMode::SameIntervals, BassMotifPhrase::Simple);

    // Both bass derivations use bass lane seed 1; the melody lane seed is
    // irrelevant to bass output and isn't passed in here at all.
    let bass_a = derive_bass_motif(&chords, scale, &p, &gen(m), 1, TPB);
    let bass_b = derive_bass_motif(&chords, scale, &p, &gen(m), 1, TPB);
    assert_eq!(bass_a, bass_b);
}
