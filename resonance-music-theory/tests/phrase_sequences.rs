//! Melodic sequences as a transform (Open Music Theory v2, sequences):
//! a *model* — the idea's head motive — restated as transposed copies
//! at a fixed interval per copy. Three patterns: descending fifths
//! (model, −5th, −5th…), descending thirds, and the rising ascending
//! 5–6 (a step up per copy). The transform tiles the model at
//! successive anchor-centered offsets; the harmony alignment pass
//! realizes the transposition diatonically.
//!
//! Sequences are drawn for sentence *continuations* (alongside
//! fragmentation) and for departure-position antecedents at moderate
//! and high complexity. The plan API (`plan_motif_transforms`,
//! `MotifTransform`, `SequenceKind`) is public for the same reason
//! `phrase_grammar_roles` is: these tests assert plan-level properties
//! that the downstream repair passes deliberately blur in the notes.

use resonance_music_theory::{
    derive_motif_melody_with_section, phrase_grammar_roles, plan_motif_transforms, Chord,
    ChordQuality, ContourPreference, GeneratedNote, MelodyParams, MelodyStyle, Mode, MotifParams,
    MotifSource, MotifTransform, PhraseGrammarRole, PitchClass, Scale, SequenceKind, TimedChord,
};

const TPB: u32 = 480;

const ALL_KINDS: [SequenceKind; 3] = [
    SequenceKind::DescendingFifths,
    SequenceKind::DescendingThirds,
    SequenceKind::Ascending56,
];

fn tc(chord: Chord, start_beat: u32, duration_beats: u32) -> TimedChord {
    TimedChord {
        chord,
        start_beat,
        duration_beats,
    }
}

fn maj(root: PitchClass) -> Chord {
    Chord::new(root, ChordQuality::Maj)
}

/// 8 chords / phrase_len 2 = 4 phrases: presentation = chords 0..4
/// (beats 0..16), continuation = chords 4..8 (beats 16..32). Same
/// shape as the phrase_forms suite.
fn sentence_chords() -> Vec<TimedChord> {
    vec![
        tc(maj(PitchClass::C), 0, 4),
        tc(Chord::new(PitchClass::A, ChordQuality::Min), 4, 4),
        tc(maj(PitchClass::F), 8, 4),
        tc(maj(PitchClass::G), 12, 4),
        tc(maj(PitchClass::C), 16, 4),
        tc(Chord::new(PitchClass::A, ChordQuality::Min), 20, 4),
        tc(maj(PitchClass::G), 24, 4),
        tc(maj(PitchClass::C), 28, 4),
    ]
}

fn melody_params() -> MelodyParams {
    MelodyParams {
        style: MelodyStyle::Motif,
        phrase_len: 2,
        rest_density: 0.0,
        complexity: 0.6,
        leap_chance: 0.3,
        contour: ContourPreference::Auto,
        ..MelodyParams::default()
    }
}

fn generate_melody(chords: &[TimedChord], seed: u64) -> Vec<GeneratedNote> {
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let params = melody_params();
    let source = MotifSource::Generated(MotifParams {
        seed,
        complexity: 0.6,
        motif_len: 0,
        leap_chance: 0.3,
    });
    derive_motif_melody_with_section(chords, scale, &params, &source, seed, TPB)
}

// ---------------------------------------------------------------------------
// Cell structure: offsets step by the kind's interval, centered
// ---------------------------------------------------------------------------

#[test]
fn sequence_offsets_step_by_the_kind_interval_and_center_on_the_anchor() {
    for kind in ALL_KINDS {
        let step = kind.step_semitones();
        assert_ne!(step, 0);
        for statements in 2..=4usize {
            let offsets = kind.offsets(statements);
            assert_eq!(offsets.len(), statements, "{kind:?}/{statements}");
            // Every copy sits exactly one kind-interval from the
            // previous statement.
            for w in offsets.windows(2) {
                assert_eq!(
                    i16::from(w[1]) - i16::from(w[0]),
                    i16::from(step),
                    "{kind:?}/{statements}: offsets {offsets:?}"
                );
            }
            // Centered on the anchor: the run's midpoint stays within
            // one step of zero, so register clamping can't flatten a
            // whole run that started at the anchor and only went down.
            let min = i16::from(*offsets.iter().min().unwrap());
            let max = i16::from(*offsets.iter().max().unwrap());
            assert!(
                (min + max).abs() <= i16::from(step.abs()),
                "{kind:?}/{statements}: offsets {offsets:?} not centered"
            );
        }
    }
}

#[test]
fn sequence_kinds_have_the_documented_directions() {
    assert!(SequenceKind::DescendingFifths.step_semitones() < 0);
    assert!(SequenceKind::DescendingThirds.step_semitones() < 0);
    assert!(SequenceKind::Ascending56.step_semitones() > 0);
    // Canonical sizes: a perfect fifth, a third, a step.
    assert_eq!(SequenceKind::DescendingFifths.step_semitones().abs(), 7);
    assert_eq!(SequenceKind::DescendingThirds.step_semitones().abs(), 3);
    assert!(SequenceKind::Ascending56.step_semitones() <= 2);
}

// ---------------------------------------------------------------------------
// Plan level: sequences appear in continuations and departures
// ---------------------------------------------------------------------------

#[test]
fn continuations_draw_sequences_at_the_expected_rate() {
    let mut sentences = 0usize;
    let mut sequences = 0usize;
    let mut kind_seen = [false; 3];
    for seed in 0..400u64 {
        let roles = phrase_grammar_roles(4, seed);
        if roles[2] != PhraseGrammarRole::Continuation {
            continue; // period group
        }
        sentences += 1;
        let transforms = plan_motif_transforms(4, 4, 0.6, seed);
        match transforms[2] {
            MotifTransform::Sequence {
                kind,
                copies,
                model_len,
            } => {
                sequences += 1;
                assert!(
                    (2..=3).contains(&copies),
                    "copies out of the 2–3 range: {copies}"
                );
                assert_eq!(model_len, 2, "model is the 4-note motif's head");
                let ki = ALL_KINDS.iter().position(|&k| k == kind).unwrap();
                kind_seen[ki] = true;
            }
            MotifTransform::Fragment(n) => assert_eq!(n, 2),
            other => panic!("unexpected continuation transform {other:?} (seed {seed})"),
        }
        // The cadential continuation reuses the continuation's device —
        // the sentence's drive keeps one development technique.
        assert_eq!(
            transforms[3], transforms[2],
            "cadential continuation diverged (seed {seed})"
        );
    }
    assert!(sentences >= 150, "too few sentence seeds ({sentences})");
    let share = sequences as f32 / sentences as f32;
    assert!(
        (0.28..=0.52).contains(&share),
        "sequence share off target: {sequences}/{sentences}"
    );
    assert!(
        kind_seen.iter().all(|&s| s),
        "not all sequence kinds drawn: {kind_seen:?}"
    );
}

#[test]
fn departures_draw_sequences_at_moderate_and_high_complexity() {
    // Departure-position antecedents (any antecedent after the section
    // opener) draw from the complexity-weighted repertoire, which now
    // includes sequences at moderate and high complexity.
    for complexity in [0.5f32, 0.8] {
        let mut departures = 0usize;
        let mut sequences = 0usize;
        for seed in 0..400u64 {
            let roles = phrase_grammar_roles(8, seed);
            let transforms = plan_motif_transforms(8, 4, complexity, seed);
            for (i, role) in roles.iter().enumerate() {
                if i == 0 || *role != PhraseGrammarRole::Antecedent {
                    continue;
                }
                departures += 1;
                if matches!(transforms[i], MotifTransform::Sequence { .. }) {
                    sequences += 1;
                }
            }
        }
        assert!(departures >= 300, "too few departures ({departures})");
        let share = sequences as f32 / departures as f32;
        assert!(
            (0.05..=0.25).contains(&share),
            "departure sequence share off target at complexity {complexity}: \
             {sequences}/{departures}"
        );
    }
}

#[test]
fn low_complexity_never_draws_sequences() {
    for seed in 0..400u64 {
        let transforms = plan_motif_transforms(8, 4, 0.2, seed);
        assert!(
            !transforms
                .iter()
                .any(|t| matches!(t, MotifTransform::Sequence { .. })),
            "sequence drawn at low complexity (seed {seed}): {transforms:?}"
        );
    }
}

#[test]
fn transform_plans_are_deterministic() {
    for seed in [0u64, 7, 77, 1234] {
        assert_eq!(
            plan_motif_transforms(8, 4, 0.6, seed),
            plan_motif_transforms(8, 4, 0.6, seed)
        );
    }
}

// ---------------------------------------------------------------------------
// Realization: transposed statements survive into the rendered notes
// ---------------------------------------------------------------------------

/// Seeds whose sentence continuation is a sequence of the given
/// direction (sign of the per-copy step).
fn sequence_seeds(direction: i8) -> Vec<u64> {
    (0..400u64)
        .filter(|&seed| {
            let roles = phrase_grammar_roles(4, seed);
            roles[2] == PhraseGrammarRole::Continuation
                && matches!(
                    plan_motif_transforms(4, 4, 0.6, seed)[2],
                    MotifTransform::Sequence { kind, .. }
                        if kind.step_semitones().signum() == direction
                )
        })
        .collect()
}

#[test]
fn sequence_continuations_trend_in_the_kind_direction() {
    // The continuation phrase covers chords 4..8 (beats 16..32). Within
    // its first chord, the realized cell sounds the model followed by
    // its transposed copies, so the pitch trend across the chord should
    // follow the kind's direction in a clear majority of seeds — the
    // downstream repair passes (leap recovery, climax demotion,
    // embellishment) nudge individual notes but not the statement-level
    // transposition shape.
    let chords = sentence_chords();
    let window = (16 * TPB as u64)..(20 * TPB as u64);
    for direction in [-1i8, 1] {
        let seeds = sequence_seeds(direction);
        assert!(
            seeds.len() >= 20,
            "too few sequence seeds for direction {direction} ({})",
            seeds.len()
        );
        let mut measured = 0usize;
        let mut trending = 0usize;
        for &seed in &seeds {
            let notes = generate_melody(&chords, seed);
            let in_chord: Vec<i32> = notes
                .iter()
                .filter(|n| window.contains(&n.start_tick))
                .map(|n| i32::from(n.note))
                .collect();
            if in_chord.len() < 4 {
                continue;
            }
            let half = in_chord.len() / 2;
            let first: i32 = in_chord[..half].iter().sum::<i32>() / half as i32;
            let second: i32 =
                in_chord[half..].iter().sum::<i32>() / (in_chord.len() - half) as i32;
            measured += 1;
            if (second - first).signum() == i32::from(direction) {
                trending += 1;
            }
        }
        assert!(measured >= 20, "too few measurable seeds ({measured})");
        assert!(
            trending as f32 >= measured as f32 * 0.55,
            "direction {direction}: only {trending}/{measured} continuations trend with the \
             sequence"
        );
    }
}

#[test]
fn sequence_output_stays_in_scale_and_register() {
    // Diatonic realization: every rendered note of a sequence
    // continuation is a scale member (the alignment pass is what makes
    // the chromatic per-copy steps diatonic), inside the lane register.
    let chords = sentence_chords();
    let scale = Scale::new(PitchClass::C, Mode::Major);
    let params = melody_params();
    let mut seeds = sequence_seeds(-1);
    seeds.extend(sequence_seeds(1));
    assert!(seeds.len() >= 40, "too few sequence seeds ({})", seeds.len());
    for &seed in &seeds {
        let notes = generate_melody(&chords, seed);
        assert!(!notes.is_empty(), "no notes for seed {seed}");
        for n in &notes {
            assert!(
                scale.contains(n.note),
                "non-scale note {} (seed {seed})",
                n.note
            );
            assert!(
                (params.register.0..=params.register.1).contains(&n.note),
                "note {} outside register (seed {seed})",
                n.note
            );
        }
    }
}

#[test]
fn sequence_generation_is_deterministic() {
    let chords = sentence_chords();
    let seeds = sequence_seeds(-1);
    let seed = *seeds.first().expect("at least one descending-sequence seed");
    assert_eq!(generate_melody(&chords, seed), generate_melody(&chords, seed));
}
