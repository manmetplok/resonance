//! Composable transforms: curated pairs of primitive motif operations
//! (`MotifTransform::Composed(ComposedPair)`) applied in sequence —
//! fragment+transpose, invert+augment, retrograde+invert. The pairs
//! widen the *operator vocabulary* at high complexity rather than the
//! randomness: they are drawn only from `pick_transform`'s high tier
//! (complexity >= 0.7), at a conservative rate, from a small fixed set
//! of musically coherent combinations.
//!
//! Like the sequence suite, the plan API (`plan_motif_transforms`,
//! `MotifTransform`, `ComposedPair`) is public because these tests
//! assert plan-level properties that the downstream repair passes
//! (leap recovery, climax, cadence, embellishment) deliberately blur
//! in the rendered notes.

use resonance_music_theory::{
    derive_motif_melody_with_section, phrase_grammar_roles, plan_motif_transforms, Chord,
    ChordQuality, ComposedPair, ContourPreference, GeneratedNote, MelodyParams, MelodyStyle, Mode,
    MotifParams, MotifSource, MotifTransform, PhraseGrammarRole, PitchClass, Scale, TimedChord,
};

const TPB: u32 = 480;

/// Complexity used for the high-tier draws under test.
const HIGH: f32 = 0.9;
/// Pinned motif length: `MotifParams.motif_len = 4` makes the built
/// motif exactly 4 notes long, so direct `plan_motif_transforms`
/// calls in the filters match the pipeline's plan for the same seed.
const MOTIF_LEN: usize = 4;

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

/// 16 chords / phrase_len 2 = 8 phrases: two full form groups, so the
/// role plan contains departure-position antecedents (the only place
/// composed pairs are drawn).
fn section_chords() -> Vec<TimedChord> {
    let pattern = [
        maj(PitchClass::C),
        Chord::new(PitchClass::A, ChordQuality::Min),
        maj(PitchClass::F),
        maj(PitchClass::G),
        maj(PitchClass::C),
        Chord::new(PitchClass::A, ChordQuality::Min),
        maj(PitchClass::G),
        maj(PitchClass::C),
    ];
    (0..16u32)
        .map(|i| tc(pattern[(i % 8) as usize], i * 4, 4))
        .collect()
}

fn melody_params() -> MelodyParams {
    MelodyParams {
        style: MelodyStyle::Motif,
        phrase_len: 2,
        rest_density: 0.0,
        complexity: HIGH,
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
        complexity: HIGH,
        motif_len: MOTIF_LEN as u8,
        leap_chance: 0.3,
    });
    derive_motif_melody_with_section(chords, scale, &params, &source, seed, TPB)
}

/// Departure-position antecedent indices (any antecedent after the
/// section opener) for an 8-phrase role plan.
fn departure_indices(roles: &[PhraseGrammarRole]) -> Vec<usize> {
    roles
        .iter()
        .enumerate()
        .filter(|&(i, role)| i > 0 && *role == PhraseGrammarRole::Antecedent)
        .map(|(i, _)| i)
        .collect()
}

// ---------------------------------------------------------------------------
// Composition semantics: pairs decompose into their documented parts
// ---------------------------------------------------------------------------

#[test]
fn composed_pairs_decompose_into_their_documented_parts() {
    // Fragment+transpose: the realized cell is the fragment tiling
    // shifted by `semitones` — `transform_motif` applies `parts()` in
    // order, so the decomposition *is* the composition contract.
    assert_eq!(
        ComposedPair::FragmentTranspose {
            frag_len: 2,
            semitones: 3,
        }
        .parts(),
        (MotifTransform::Fragment(2), MotifTransform::TransposeUp(3))
    );
    // Negative semitones map to TransposeDown with the magnitude.
    assert_eq!(
        ComposedPair::FragmentTranspose {
            frag_len: 3,
            semitones: -4,
        }
        .parts(),
        (
            MotifTransform::Fragment(3),
            MotifTransform::TransposeDown(4)
        )
    );
    assert_eq!(
        ComposedPair::InvertAugment.parts(),
        (MotifTransform::Invert, MotifTransform::Augment)
    );
    assert_eq!(
        ComposedPair::RetrogradeInvert.parts(),
        (MotifTransform::Retrograde, MotifTransform::Invert)
    );
}

#[test]
fn composed_parts_are_always_primitive() {
    // `transform_motif` realizes a pair by recursing on its parts; the
    // recursion terminates because no part is itself composed.
    let pairs = [
        ComposedPair::FragmentTranspose {
            frag_len: 2,
            semitones: 5,
        },
        ComposedPair::InvertAugment,
        ComposedPair::RetrogradeInvert,
    ];
    for pair in pairs {
        let (first, second) = pair.parts();
        assert!(
            !matches!(first, MotifTransform::Composed(_)),
            "{pair:?}: first part is composed"
        );
        assert!(
            !matches!(second, MotifTransform::Composed(_)),
            "{pair:?}: second part is composed"
        );
    }
}

// ---------------------------------------------------------------------------
// Plan level: composed pairs gate on high complexity, conservatively
// ---------------------------------------------------------------------------

#[test]
fn composed_pairs_never_appear_below_high_complexity() {
    // The complexity knob is the user's simplicity control: the low
    // and moderate tiers keep the single-operation vocabulary.
    for complexity in [0.2f32, 0.5, 0.65] {
        for seed in 0..400u64 {
            let transforms = plan_motif_transforms(8, MOTIF_LEN, complexity, seed);
            assert!(
                !transforms
                    .iter()
                    .any(|t| matches!(t, MotifTransform::Composed(_))),
                "composed pair drawn at complexity {complexity} (seed {seed}): {transforms:?}"
            );
        }
    }
}

#[test]
fn high_complexity_departures_draw_composed_pairs_conservatively() {
    // ~10% of high-tier departure draws are composed pairs: enough to
    // widen the vocabulary, conservative enough that high-complexity
    // output is varied rather than chaotic. All three pair kinds occur,
    // and sequences keep their own slice (composition widens the
    // repertoire instead of replacing part of it).
    let mut departures = 0usize;
    let mut composed = 0usize;
    let mut sequences = 0usize;
    let mut frag_transpose = 0usize;
    let mut invert_augment = 0usize;
    let mut retrograde_invert = 0usize;
    for seed in 0..400u64 {
        let roles = phrase_grammar_roles(8, seed);
        let transforms = plan_motif_transforms(8, MOTIF_LEN, HIGH, seed);
        for i in departure_indices(&roles) {
            departures += 1;
            match transforms[i] {
                MotifTransform::Composed(pair) => {
                    composed += 1;
                    match pair {
                        ComposedPair::FragmentTranspose {
                            frag_len,
                            semitones,
                        } => {
                            frag_transpose += 1;
                            assert_eq!(frag_len, 2, "fragment is the 4-note motif's head");
                            assert!(
                                (1..=5).contains(&semitones.abs()),
                                "transposition out of the 1-5 semitone range: {semitones}"
                            );
                        }
                        ComposedPair::InvertAugment => invert_augment += 1,
                        ComposedPair::RetrogradeInvert => retrograde_invert += 1,
                    }
                }
                MotifTransform::Sequence { .. } => sequences += 1,
                _ => {}
            }
        }
    }
    assert!(departures >= 300, "too few departures ({departures})");
    let share = composed as f32 / departures as f32;
    assert!(
        (0.04..=0.20).contains(&share),
        "composed share off target: {composed}/{departures}"
    );
    assert!(
        frag_transpose > 0 && invert_augment > 0 && retrograde_invert > 0,
        "not all pair kinds drawn: \
         fragment+transpose {frag_transpose}, invert+augment {invert_augment}, \
         retrograde+invert {retrograde_invert}"
    );
    let seq_share = sequences as f32 / departures as f32;
    assert!(
        (0.05..=0.25).contains(&seq_share),
        "sequence share collapsed: {sequences}/{departures}"
    );
}

#[test]
fn continuations_never_draw_composed_pairs() {
    // Sentence continuations keep their two canonical devices —
    // fragmentation and melodic sequence; composed pairs belong to
    // departure phrases only.
    for seed in 0..400u64 {
        let roles = phrase_grammar_roles(8, seed);
        let transforms = plan_motif_transforms(8, MOTIF_LEN, HIGH, seed);
        for (i, role) in roles.iter().enumerate() {
            if matches!(
                role,
                PhraseGrammarRole::Continuation | PhraseGrammarRole::ContinuationCadence
            ) {
                assert!(
                    !matches!(transforms[i], MotifTransform::Composed(_)),
                    "continuation drew a composed pair (seed {seed}): {:?}",
                    transforms[i]
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Reuse and determinism
// ---------------------------------------------------------------------------

#[test]
fn consequents_reuse_composed_antecedent_transforms() {
    // The period's defining parallelism survives composition: a
    // consequent restates its antecedent's transform verbatim even
    // when that transform is a composed pair.
    let mut checked = 0usize;
    for seed in 0..400u64 {
        let roles = phrase_grammar_roles(8, seed);
        let transforms = plan_motif_transforms(8, MOTIF_LEN, HIGH, seed);
        for i in departure_indices(&roles) {
            if !matches!(transforms[i], MotifTransform::Composed(_)) {
                continue;
            }
            assert_eq!(
                roles[i + 1],
                PhraseGrammarRole::Consequent,
                "antecedent not followed by consequent (seed {seed})"
            );
            assert_eq!(
                transforms[i + 1],
                transforms[i],
                "consequent diverged from composed antecedent (seed {seed})"
            );
            checked += 1;
        }
    }
    assert!(checked >= 20, "too few composed periods checked ({checked})");
}

#[test]
fn composed_transform_plans_are_deterministic() {
    for seed in [0u64, 7, 77, 1234] {
        assert_eq!(
            plan_motif_transforms(8, MOTIF_LEN, HIGH, seed),
            plan_motif_transforms(8, MOTIF_LEN, HIGH, seed)
        );
    }
}

// ---------------------------------------------------------------------------
// Realization: composed phrases converge through the repair fixpoints
// ---------------------------------------------------------------------------

/// Seeds whose 8-phrase high-complexity plan contains at least one
/// composed pair (matching the pipeline's plan for the same seed).
fn composed_seeds() -> Vec<u64> {
    (0..800u64)
        .filter(|&seed| {
            plan_motif_transforms(8, MOTIF_LEN, HIGH, seed)
                .iter()
                .any(|t| matches!(t, MotifTransform::Composed(_)))
        })
        .collect()
}

#[test]
fn composed_output_stays_in_scale_and_register() {
    // The downstream validators (leap recovery, climax, cadence,
    // embellishment) repair whatever the transforms produce; composed
    // cells must converge through the same fixpoints — every rendered
    // note diatonic and inside the lane register, no empty output.
    let chords = section_chords();
    let scale = Scale::new(PitchClass::C, Mode::Major);
    let params = melody_params();
    let seeds = composed_seeds();
    assert!(seeds.len() >= 40, "too few composed seeds ({})", seeds.len());
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
fn composed_generation_is_deterministic() {
    let chords = section_chords();
    let seeds = composed_seeds();
    let seed = *seeds.first().expect("at least one composed seed");
    assert_eq!(generate_melody(&chords, seed), generate_melody(&chords, seed));
}
