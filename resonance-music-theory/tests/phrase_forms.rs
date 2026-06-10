//! Phrase grammar: sentence / period / srdc planning (Open Music
//! Theory v2, phrase archetypes). Replaces the old independent
//! per-phrase transform draws:
//!
//!   - *Sentence* (4-phrase group): basic idea + varied repeat
//!     (presentation, no cadence) → continuation phrases that fragment
//!     the idea's head motive at doubled surface-rhythm density, with
//!     the group's one strong cadence on the final phrase.
//!   - *Period*: the consequent reuses the antecedent's opening
//!     transform and swaps the ending weak→strong (cadence goals).
//!   - *Pop srdc* (vocal sections): lines group in fours as aaba/aabc —
//!     the restatement re-sings the statement's contour, the departure
//!     contrasts, the conclusion closes (and restates in aaba groups).
//!
//! The grammar plan is exposed through `phrase_grammar_roles` so these
//! tests can select sentence/period seeds; everything else is asserted
//! through the public generator entry points.

use resonance_music_theory::{
    count_syllables, derive_motif_melody_with_section, derive_vocal, generate_lyrics,
    phrase_grammar_roles, Chord, ChordQuality, ContourPreference, GeneratedNote, MelodyParams,
    MelodyStyle, Mode, MotifParams, MotifSource, PhraseGrammarRole, PitchClass, Scale, TimedChord,
    VocalParams,
};

const TPB: u32 = 480;

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

fn melody_params(phrase_len: u8, contour: ContourPreference) -> MelodyParams {
    MelodyParams {
        style: MelodyStyle::Motif,
        phrase_len,
        rest_density: 0.0,
        complexity: 0.6,
        leap_chance: 0.3,
        contour,
        ..MelodyParams::default()
    }
}

fn generate_melody(
    chords: &[TimedChord],
    phrase_len: u8,
    contour: ContourPreference,
    seed: u64,
) -> Vec<GeneratedNote> {
    let scale = Some(Scale::new(PitchClass::C, Mode::Major));
    let params = melody_params(phrase_len, contour);
    let source = MotifSource::Generated(MotifParams {
        seed,
        complexity: 0.6,
        motif_len: 0,
        leap_chance: 0.3,
    });
    derive_motif_melody_with_section(chords, scale, &params, &source, seed, TPB)
}

// ---------------------------------------------------------------------------
// Grammar plan structure
// ---------------------------------------------------------------------------

#[test]
fn grammar_roles_form_sentences_or_periods_in_groups_of_four() {
    use PhraseGrammarRole::*;
    let sentence = [BasicIdea, VariedRepeat, Continuation, ContinuationCadence];
    let period = [Antecedent, Consequent, Antecedent, Consequent];
    let mut sentences = 0usize;
    let mut periods = 0usize;
    for seed in 0..200u64 {
        for n in 1..=11usize {
            let roles = phrase_grammar_roles(n, seed);
            assert_eq!(roles.len(), n, "role count for n={n}");
            // Full leading groups of four are one of the two forms.
            let mut i = 0;
            while n - i >= 4 {
                let group = &roles[i..i + 4];
                assert!(
                    group == sentence || group == period,
                    "unexpected 4-group {group:?} (n={n}, seed={seed})"
                );
                if group == sentence {
                    sentences += 1;
                } else {
                    periods += 1;
                }
                i += 4;
            }
            // Trailing remainder: a pair is a period; a lone phrase
            // stays open when it is the whole section, closes after
            // earlier groups (a trailing triple is a period plus a
            // closing reprise).
            match n - i {
                0 => {}
                1 => {
                    let expected = if i == 0 { Antecedent } else { Consequent };
                    assert_eq!(roles[i], expected, "trailing single (n={n})");
                }
                2 => assert_eq!(&roles[i..], &[Antecedent, Consequent], "trailing pair"),
                3 => assert_eq!(
                    &roles[i..],
                    &[Antecedent, Consequent, Consequent],
                    "trailing triple"
                ),
                _ => unreachable!(),
            }
        }
        // Deterministic per seed.
        assert_eq!(phrase_grammar_roles(8, seed), phrase_grammar_roles(8, seed));
    }
    // Both forms actually occur.
    assert!(sentences > 100, "sentences underrepresented ({sentences})");
    assert!(periods > 100, "periods underrepresented ({periods})");
}

// ---------------------------------------------------------------------------
// Sentence: fragmentation + denser surface rhythm in the continuation
// ---------------------------------------------------------------------------

/// 8 chords / phrase_len 2 = 4 phrases: presentation = chords 0..4
/// (beats 0..16), continuation = chords 4..8 (beats 16..32).
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

#[test]
fn sentence_continuation_doubles_the_surface_rhythm() {
    let chords = sentence_chords();
    let split_tick = 16 * TPB as u64;
    let mut sentence_seeds = 0usize;
    let mut presentation_notes = 0usize;
    let mut continuation_notes = 0usize;
    for seed in 0..200u64 {
        let roles = phrase_grammar_roles(4, seed);
        if roles[0] != PhraseGrammarRole::BasicIdea {
            continue; // period group — covered by the period tests
        }
        sentence_seeds += 1;
        let notes = generate_melody(&chords, 2, ContourPreference::Auto, seed);
        presentation_notes += notes.iter().filter(|n| n.start_tick < split_tick).count();
        continuation_notes += notes.iter().filter(|n| n.start_tick >= split_tick).count();
    }
    assert!(sentence_seeds >= 60, "too few sentence seeds ({sentence_seeds})");
    // The continuation tiles the fragmented head motive at twice the
    // presentation's rate, so over the same number of beats it carries
    // roughly twice the notes. Assert a clear aggregate margin.
    assert!(
        continuation_notes as f32 >= presentation_notes as f32 * 1.5,
        "continuation not denser: {continuation_notes} vs {presentation_notes} presentation notes"
    );
}

#[test]
fn sentence_carries_its_cadence_on_the_final_phrase() {
    // Final chord is the C tonic: the cadential continuation targets
    // PAC (with the ~10% deceptive swap), so the section's final note
    // lands on a formula degree — mostly the tonic pitch class.
    let chords = sentence_chords();
    let mut total = 0usize;
    let mut tonic = 0usize;
    let mut formula = 0usize;
    for seed in 0..200u64 {
        let roles = phrase_grammar_roles(4, seed);
        if roles[3] != PhraseGrammarRole::ContinuationCadence {
            continue;
        }
        let notes = generate_melody(&chords, 2, ContourPreference::Auto, seed);
        let Some(last) = notes.last() else { continue };
        total += 1;
        let pc = last.note % 12;
        if pc == 0 {
            tonic += 1;
        }
        // Goal-chain finals over the tonic chord: PAC 1 (C), deceptive
        // 6 (A), and the IAC fallbacks 3/5 (E/G) taken when no full
        // two-note PAC realization validates on the dense continuation.
        if [0u8, 4, 7, 9].contains(&pc) {
            formula += 1;
        }
    }
    assert!(total >= 60, "too few sentence seeds ({total})");
    assert!(
        tonic as f32 >= total as f32 * 0.55,
        "only {tonic}/{total} sentences closed on the tonic"
    );
    assert!(
        formula as f32 >= total as f32 * 0.90,
        "only {formula}/{total} sentences ended on a goal-cadence degree"
    );
}

// ---------------------------------------------------------------------------
// Period: the consequent reuses the antecedent's opening
// ---------------------------------------------------------------------------

#[test]
fn period_consequent_reuses_the_antecedent_opening() {
    // Two phrases over identical chord pairs (C F | C F) with a pinned
    // contour: the consequent reuses the antecedent's transform, so
    // its realization of the first chord matches the antecedent's
    // opening — same pitch classes in the same order (octave
    // displacement is lane-local variation), same relative onsets.
    // Only the *endings* differ (weak HC/IAC vs strong PAC).
    let chords = vec![
        tc(maj(PitchClass::C), 0, 4),
        tc(maj(PitchClass::F), 4, 4),
        tc(maj(PitchClass::C), 8, 4),
        tc(maj(PitchClass::F), 12, 4),
    ];
    let mut total = 0usize;
    let mut matching = 0usize;
    for seed in 0..200u64 {
        let notes = generate_melody(&chords, 2, ContourPreference::Arch, seed);
        let opening: Vec<(u64, u8)> = notes
            .iter()
            .filter(|n| n.start_tick < 4 * TPB as u64)
            .map(|n| (n.start_tick, n.note % 12))
            .collect();
        let reprise: Vec<(u64, u8)> = notes
            .iter()
            .filter(|n| (8 * TPB as u64..12 * TPB as u64).contains(&n.start_tick))
            .map(|n| (n.start_tick - 8 * TPB as u64, n.note % 12))
            .collect();
        if opening.is_empty() {
            continue;
        }
        total += 1;
        if opening == reprise {
            matching += 1;
        }
    }
    assert!(total >= 190, "too few periods generated ({total})");
    assert!(
        matching as f32 >= total as f32 * 0.85,
        "only {matching}/{total} consequents reused the antecedent's opening"
    );
}

#[test]
fn phrase_grammar_keeps_generation_deterministic() {
    let chords = sentence_chords();
    let a = generate_melody(&chords, 2, ContourPreference::Auto, 77);
    let b = generate_melody(&chords, 2, ContourPreference::Auto, 77);
    assert_eq!(a, b);
}

// ---------------------------------------------------------------------------
// Vocal srdc: aaba / aabc section layout
// ---------------------------------------------------------------------------

fn c_major_chords() -> Vec<TimedChord> {
    (0..4).map(|i| tc(maj(PitchClass::C), i * 4, 4)).collect()
}

/// Per-line note slices, recovered the same way the generator and the
/// SVS pipeline do.
fn line_slices<'a>(notes: &'a [GeneratedNote], params: &VocalParams) -> Vec<&'a [GeneratedNote]> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    for line in &params.draft {
        let n = (count_syllables(&line.text) as usize).min(notes.len().saturating_sub(cursor));
        if n == 0 {
            continue;
        }
        out.push(&notes[cursor..cursor + n]);
        cursor += n;
    }
    out
}

/// Mean absolute difference between two lines' contours (semitone
/// offsets from each line's first note, index-scaled across differing
/// syllable counts). The final two syllables are excluded — the
/// cadence-formula pass rewrites them per the line's own goal.
fn contour_diff(a: &[GeneratedNote], b: &[GeneratedNote]) -> Option<f32> {
    if a.len() < 3 || b.len() < 4 {
        return None;
    }
    let a0 = a[0].note as i16;
    let b0 = b[0].note as i16;
    let mut sum = 0.0f32;
    let mut count = 0usize;
    for s in 1..b.len() - 2 {
        let idx = s * a.len() / b.len();
        let a_off = a[idx].note as i16 - a0;
        let b_off = b[s].note as i16 - b0;
        sum += (b_off - a_off).abs() as f32;
        count += 1;
    }
    (count > 0).then(|| sum / count as f32)
}

#[test]
fn srdc_restatement_echoes_the_statement_and_departure_contrasts() {
    let chords = c_major_chords();
    let mut restate_sum = 0.0f32;
    let mut depart_sum = 0.0f32;
    let mut groups = 0usize;
    for seed in 0..120u64 {
        let mut p = VocalParams::default();
        p.draft = generate_lyrics(&p, seed.wrapping_add(5));
        let notes = derive_vocal(&chords, &p, TPB, seed);
        let slices = line_slices(&notes, &p);
        if slices.len() < 4 {
            continue;
        }
        let (Some(restate), Some(depart)) = (
            contour_diff(slices[0], slices[1]),
            contour_diff(slices[0], slices[2]),
        ) else {
            continue;
        };
        restate_sum += restate;
        depart_sum += depart;
        groups += 1;
    }
    assert!(groups >= 80, "too few 4-line groups ({groups})");
    let restate_avg = restate_sum / groups as f32;
    let depart_avg = depart_sum / groups as f32;
    // The restatement re-sings the statement's contour (small residual
    // from scale snapping + the climax/cadence passes); the departure
    // is independent walked material.
    assert!(
        restate_avg <= 1.5,
        "restatement strays from the statement contour (avg diff {restate_avg})"
    );
    assert!(
        restate_avg + 0.5 <= depart_avg,
        "departure does not contrast: restatement {restate_avg} vs departure {depart_avg}"
    );
}

#[test]
fn srdc_sections_split_between_aaba_and_aabc() {
    // The conclusion either restates the statement's contour (aaba) or
    // keeps its own material (aabc), ~50/50 per group, seeded. Assert
    // both shapes occur across seeds.
    let chords = c_major_chords();
    let mut restated = 0usize;
    let mut groups = 0usize;
    for seed in 0..120u64 {
        let mut p = VocalParams::default();
        p.draft = generate_lyrics(&p, seed.wrapping_add(5));
        let notes = derive_vocal(&chords, &p, TPB, seed);
        let slices = line_slices(&notes, &p);
        if slices.len() < 4 {
            continue;
        }
        let Some(diff) = contour_diff(slices[0], slices[3]) else {
            continue;
        };
        groups += 1;
        if diff <= 1.0 {
            restated += 1;
        }
    }
    assert!(groups >= 80, "too few 4-line groups ({groups})");
    let share = restated as f32 / groups as f32;
    assert!(
        (0.20..=0.85).contains(&share),
        "aaba share off target: {restated}/{groups}"
    );
}

#[test]
fn srdc_layout_is_deterministic() {
    let chords = c_major_chords();
    let mut p = VocalParams::default();
    p.draft = generate_lyrics(&p, 21);
    let a = derive_vocal(&chords, &p, TPB, 21);
    let b = derive_vocal(&chords, &p, TPB, 21);
    assert_eq!(a, b);
}
