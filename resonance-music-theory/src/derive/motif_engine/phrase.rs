// Phrase-level planning and rendering: divide the chord progression into
// phrases, pick a contour for each, choose a Transform sequence across
// phrases, and realize a single phrase into MIDI notes.

use crate::rng::XorShift;
use crate::scale::Scale;
use crate::voicing::nearest_midi_to;

use super::super::cadence::plan_cadence_goal;
use super::super::melody::ContourPreference;
use super::super::motif_bass::chord_tones_in_register;
use super::super::{GeneratedNote, TimedChord};
use super::build::transform_motif;
use super::harmony::{align_to_harmony, nearest_in_set};
use super::types::{
    ComposedPair, Contour, MotifNote, PhraseGrammarRole, PhrasePlan, SequenceKind, Transform,
};

/// Plan the grammatical roles of `num_phrases` phrases (Open Music
/// Theory v2 phrase archetypes). Phrases are grouped in fours from the
/// front; each full group becomes either a *sentence* (basic idea,
/// varied repeat, continuation, cadential continuation — one cadence
/// at the very end) or a *period chain* (antecedent/consequent pairs —
/// weak then strong endings), chosen 50/50 from `seed`. A trailing
/// pair is a period; a lone trailing phrase stays open (antecedent)
/// when it is the whole section and closes (consequent) when earlier
/// groups precede it.
///
/// Seeded only from `seed` (the section's motif seed) so every lane in
/// a section — and `plan_phrases` vs `plan_motif_transforms` — agrees
/// on the same form plan.
pub fn phrase_grammar_roles(num_phrases: usize, seed: u64) -> Vec<PhraseGrammarRole> {
    use PhraseGrammarRole::*;
    // Splitmix64-style scramble before seeding: the form choice is the
    // *first* RNG draw, and xorshift64's first output barely changes
    // across nearby seeds (a low-byte difference doesn't reach the top
    // bits in one round) — without mixing, sequential section seeds
    // would all pick the same form.
    let mut s = seed.wrapping_add(0x5E17_E14C_E0F0_2A7B);
    s = (s ^ (s >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    s = (s ^ (s >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    s ^= s >> 31;
    let mut rng = XorShift::new(s);
    let mut roles = Vec::with_capacity(num_phrases);
    while roles.len() < num_phrases {
        let remaining = num_phrases - roles.len();
        if remaining >= 4 {
            if rng.next_f32() < 0.5 {
                roles.extend([BasicIdea, VariedRepeat, Continuation, ContinuationCadence]);
            } else {
                roles.extend([Antecedent, Consequent, Antecedent, Consequent]);
            }
        } else if remaining >= 2 {
            roles.extend([Antecedent, Consequent]);
        } else if roles.is_empty() {
            roles.push(Antecedent);
        } else {
            roles.push(Consequent);
        }
    }
    roles
}

/// Index of the phrase designated to carry the *section's* single
/// climax (Open Music Theory v2: one climax per section, not one per
/// phrase). The natural carrier is the group's energy-building middle:
/// the continuation of a sentence, or the departure-position second
/// antecedent of a period chain — phrase 3 of 4 in both archetypes.
/// Concretely: the latest phrase whose role is `Continuation` or
/// `Antecedent` (closing phrases — consequents and cadential
/// continuations — resolve and never carry the peak). Sections with
/// several groups place the climax in the last group, late in the
/// section. Falls back to the second-to-last phrase for role lists
/// without an open phrase (which `phrase_grammar_roles` never emits).
pub fn section_climax_phrase(roles: &[PhraseGrammarRole]) -> usize {
    roles
        .iter()
        .rposition(|r| {
            matches!(
                r,
                PhraseGrammarRole::Continuation | PhraseGrammarRole::Antecedent
            )
        })
        .unwrap_or(roles.len().saturating_sub(2))
}

/// For each phrase, the index whose section-climax cap (and octave
/// displacement) it shares: consequents point at their antecedent —
/// the period's parallel structure must not be pulled apart by
/// independent demotion margins or octave rolls — and every other
/// phrase points at itself.
pub(super) fn section_cap_sources(roles: &[PhraseGrammarRole]) -> Vec<usize> {
    let mut last_antecedent: Option<usize> = None;
    roles
        .iter()
        .enumerate()
        .map(|(i, role)| match role {
            PhraseGrammarRole::Antecedent => {
                last_antecedent = Some(i);
                i
            }
            PhraseGrammarRole::Consequent => last_antecedent.unwrap_or(i),
            _ => i,
        })
        .collect()
}

/// Pre-compute the per-phrase Transform sequence for a motif plan from
/// the phrase-grammar roles. Uses a fresh RNG seeded only from
/// `motif.seed` so two callers with the same `MotifParams` always agree
/// on the sequence — this is what makes `BassMotifPhrase::MirrorMelody`
/// lock to the melody.
///
/// Per role (replacing the old independent per-phrase draws):
///   - `BasicIdea`: identity — the section states its idea plainly.
///   - `VariedRepeat`: exact repeat or a small (≤3 st) transposition,
///     keeping the idea recognizable through the presentation.
///   - `Continuation` / `ContinuationCadence`: development of the
///     idea's *head* motive — fragmentation (`Fragment`, tiled at a
///     doubled rate by the realizer) or a melodic *sequence* built on
///     the head (`Sequence`: model + 2–3 transposed copies — OMT's
///     other canonical continuation device). Drawn once on the
///     continuation; the cadential continuation reuses it so the
///     sentence's drive keeps one device.
///   - `Antecedent`: identity for the section opener, then the full
///     complexity-weighted repertoire (departures may also sequence,
///     and at high complexity occasionally draw a curated composed
///     pair — see `ComposedPair`).
///   - `Consequent`: reuses its antecedent's transform verbatim — the
///     period's defining "same opening, different ending"; the ending
///     swap (weak→strong) lives in the cadence goals.
pub fn plan_motif_transforms(
    num_phrases: usize,
    motif_len: usize,
    complexity: f32,
    seed: u64,
) -> Vec<Transform> {
    let roles = phrase_grammar_roles(num_phrases, seed);
    let mut rng = XorShift::new(seed.wrapping_add(0xA1B2C3D4E5F60718));
    let head_len = 2.max(motif_len / 2);
    let mut last_antecedent = Transform::Identity;
    let mut last_continuation = Transform::Fragment(head_len);
    roles
        .iter()
        .enumerate()
        .map(|(i, role)| match role {
            PhraseGrammarRole::BasicIdea => Transform::Identity,
            PhraseGrammarRole::VariedRepeat => pick_varied_repeat(&mut rng),
            PhraseGrammarRole::Continuation => {
                let t = pick_continuation(head_len, complexity, &mut rng);
                last_continuation = t;
                t
            }
            PhraseGrammarRole::ContinuationCadence => last_continuation,
            PhraseGrammarRole::Antecedent => {
                let t = if i == 0 {
                    Transform::Identity
                } else {
                    pick_transform(motif_len, i, complexity, &mut rng)
                };
                last_antecedent = t;
                t
            }
            PhraseGrammarRole::Consequent => last_antecedent,
        })
        .collect()
}

/// Variation for the sentence presentation's repeat of the basic idea:
/// exact repeat, a small transposition, or a straight syncopation of
/// the same pitches — never an operation that obscures the idea
/// (inversion, retrograde, fragmentation). The syncopated repeat keeps
/// the interval shape intact and only displaces the surface rhythm,
/// the canonical pop way to vary a restated idea.
fn pick_varied_repeat(rng: &mut XorShift) -> Transform {
    let roll = rng.next_f32();
    let amount = 1 + rng.next_range(3) as i8;
    if roll < 0.25 {
        Transform::Identity
    } else if roll < 0.50 {
        Transform::Syncopate
    } else if roll < 0.75 {
        Transform::TransposeUp(amount)
    } else {
        Transform::TransposeDown(amount)
    }
}

/// Probability that a sentence continuation develops its head motive
/// as a melodic sequence instead of plain fragmentation.
const CONTINUATION_SEQUENCE_CHANCE: f32 = 0.40;

/// Complexity floor below which no phrase draws a sequence — the
/// complexity knob is the user's simplicity control, and the simple
/// tier keeps the original fragment-only continuations (mirrors the
/// departure repertoire's tiering in `pick_transform`).
const SEQUENCE_MIN_COMPLEXITY: f32 = 0.3;

/// Development device for a sentence continuation: fragmentation of
/// the idea's head motive (the existing default), or a melodic
/// sequence built on that same head — the model restated as 2–3
/// transposed copies (OMT v2: continuations fragment *and* sequence
/// their basic idea on the way to the cadence).
fn pick_continuation(head_len: usize, complexity: f32, rng: &mut XorShift) -> Transform {
    if complexity >= SEQUENCE_MIN_COMPLEXITY && rng.next_f32() < CONTINUATION_SEQUENCE_CHANCE {
        pick_sequence(head_len, rng)
    } else {
        Transform::Fragment(head_len)
    }
}

/// Draw a melodic sequence on a `model_len`-note model: the kind is
/// weighted toward the rising 5–6 pattern (continuations build), and
/// the copy count is the OMT-typical 2–3.
fn pick_sequence(model_len: usize, rng: &mut XorShift) -> Transform {
    let roll = rng.next_f32();
    let kind = if roll < 0.40 {
        SequenceKind::Ascending56
    } else if roll < 0.70 {
        SequenceKind::DescendingFifths
    } else {
        SequenceKind::DescendingThirds
    };
    let copies = 2 + rng.next_range(2) as u8;
    Transform::Sequence {
        kind,
        copies,
        model_len,
    }
}

/// Draw a composed transform pair for a high-complexity departure
/// phrase. The vocabulary is the small curated `ComposedPair` set —
/// composition widens the *operators*, not the randomness — weighted
/// toward fragment+transpose (the most idiomatic two-step development:
/// the head motive restated at a flat offset). The transposition
/// distance reuses the repertoire's 1–5 semitone range; direction is a
/// coin. Inversion-based pairs split the remainder.
fn pick_composed(motif_len: usize, rng: &mut XorShift) -> Transform {
    let roll = rng.next_f32();
    let pair = if roll < 0.40 {
        let frag_len = 2.max(motif_len / 2);
        let amount = 1 + rng.next_range(5) as i8;
        let semitones = if rng.next_f32() < 0.5 { amount } else { -amount };
        ComposedPair::FragmentTranspose {
            frag_len,
            semitones,
        }
    } else if roll < 0.70 {
        ComposedPair::InvertAugment
    } else {
        ComposedPair::RetrogradeInvert
    };
    Transform::Composed(pair)
}

/// Pick a contour for a phrase from the preference or RNG.
fn pick_contour(pref: ContourPreference, closes: bool, rng: &mut XorShift) -> Contour {
    match pref {
        ContourPreference::Arch => Contour::Arch,
        ContourPreference::Descending => Contour::Descending,
        ContourPreference::Ascending => Contour::Ascending,
        ContourPreference::Wave => Contour::Wave,
        ContourPreference::Auto => {
            // Research-weighted: arch 29%, desc 27%, asc 22%, wave 22%.
            // Closing phrases bias toward descending (resolution).
            let roll = rng.next_f32();
            if closes {
                if roll < 0.40 {
                    Contour::Descending
                } else if roll < 0.75 {
                    Contour::Arch
                } else {
                    Contour::Ascending
                }
            } else if roll < 0.29 {
                Contour::Arch
            } else if roll < 0.56 {
                Contour::Descending
            } else if roll < 0.78 {
                Contour::Ascending
            } else {
                Contour::Wave
            }
        }
    }
}

/// Divide chords into phrases and assign contours, grammar roles, and
/// cadence goals. `grammar_seed` must be the section's motif seed so
/// the role plan here matches the one `plan_motif_transforms` derives
/// for the same phrase count — contours and goal draws still come from
/// the lane-local `rng`.
pub(in crate::derive) fn plan_phrases(
    chords: &[TimedChord],
    contour_pref: ContourPreference,
    phrase_len: u8,
    grammar_seed: u64,
    rng: &mut XorShift,
) -> Vec<PhrasePlan> {
    let plen = (phrase_len as usize).max(1);
    let num_phrases = chords.len().div_ceil(plen);
    let roles = phrase_grammar_roles(num_phrases, grammar_seed);
    // Section climax plan: the carrier phrase (and the consequent that
    // restates it) draws its contour at full amplitude; secondary
    // phrases get a reduced swing so their peaks settle below the
    // carrier's before the post-realization section pass even runs.
    let carrier = section_climax_phrase(&roles);
    let cap_sources = section_cap_sources(&roles);
    let mut plans = Vec::with_capacity(num_phrases);
    let mut i = 0;
    let mut phrase_index = 0;

    while i < chords.len() {
        let end = (i + plen).min(chords.len());
        let role = roles[phrase_index];
        let peak_scale = if cap_sources[phrase_index] == carrier {
            1.0
        } else {
            0.72
        };
        let contour = pick_contour(contour_pref, role.closes(), rng);
        // Sentence presentation + mid-continuation phrases prolong
        // without cadencing; the group's one real cadence sits on the
        // closing phrase. Period phrases keep the weak/strong pairing.
        let cadence = match role {
            PhraseGrammarRole::BasicIdea
            | PhraseGrammarRole::VariedRepeat
            | PhraseGrammarRole::Continuation => None,
            PhraseGrammarRole::Antecedent => Some(plan_cadence_goal(false, rng)),
            PhraseGrammarRole::Consequent | PhraseGrammarRole::ContinuationCadence => {
                Some(plan_cadence_goal(true, rng))
            }
        };
        plans.push(PhrasePlan {
            chord_range: (i, end),
            contour,
            role,
            cadence,
            peak_scale,
        });
        i = end;
        phrase_index += 1;
    }
    plans
}

/// Pick a transformation based on complexity and phrase position.
fn pick_transform(
    motif_len: usize,
    phrase_idx: usize,
    complexity: f32,
    rng: &mut XorShift,
) -> Transform {
    if phrase_idx == 0 {
        return Transform::Identity;
    }

    // Low complexity: mainly identity and transpose.
    // High complexity: full repertoire.
    let roll = rng.next_f32();
    let transpose_amount = 1 + rng.next_range(5) as i8;

    if complexity < 0.3 {
        // Simple: 40% identity, 30% transpose up, 30% transpose down
        if roll < 0.40 {
            Transform::Identity
        } else if roll < 0.70 {
            Transform::TransposeUp(transpose_amount)
        } else {
            Transform::TransposeDown(transpose_amount)
        }
    } else if complexity < 0.7 {
        // Moderate: add inversion, syncopation, fragmentation, and
        // melodic sequences (departure phrases restating the head
        // motive at successive transpositions).
        if roll < 0.16 {
            Transform::Identity
        } else if roll < 0.32 {
            Transform::TransposeUp(transpose_amount)
        } else if roll < 0.48 {
            Transform::TransposeDown(transpose_amount)
        } else if roll < 0.61 {
            Transform::Invert
        } else if roll < 0.75 {
            Transform::Syncopate
        } else if roll < 0.88 {
            let frag_len = 2.max(motif_len / 2);
            Transform::Fragment(frag_len)
        } else {
            pick_sequence(2.max(motif_len / 2), rng)
        }
    } else {
        // Complex: full repertoire, including a conservative slice of
        // composed pairs (two primitive operations in sequence — the
        // only tier where they appear; lower tiers keep the simpler
        // single-operation vocabulary).
        if roll < 0.07 {
            Transform::Identity
        } else if roll < 0.18 {
            Transform::TransposeUp(transpose_amount)
        } else if roll < 0.29 {
            Transform::TransposeDown(transpose_amount)
        } else if roll < 0.39 {
            Transform::Invert
        } else if roll < 0.47 {
            Transform::Retrograde
        } else if roll < 0.54 {
            Transform::Augment
        } else if roll < 0.61 {
            Transform::Diminish
        } else if roll < 0.70 {
            Transform::Syncopate
        } else if roll < 0.80 {
            let frag_len = 2.max(motif_len / 2);
            Transform::Fragment(frag_len)
        } else if roll < 0.90 {
            pick_sequence(2.max(motif_len / 2), rng)
        } else {
            pick_composed(motif_len, rng)
        }
    }
}

/// Compute a contour-based anchor offset in semitones for a given
/// position within a phrase.
fn contour_offset(contour: Contour, position: f32, register_span: u8) -> i8 {
    let half_span = (register_span / 4) as f32;
    let offset = match contour {
        Contour::Arch => {
            // Parabola peaking at position 0.5.
            let x = position - 0.5;
            half_span * (1.0 - 4.0 * x * x)
        }
        Contour::Descending => half_span * (1.0 - position),
        Contour::Ascending => half_span * position,
        Contour::Wave => {
            // One full sine cycle.
            (half_span * 0.7) * (position * std::f32::consts::TAU).sin()
        }
    };
    offset as i8
}

/// Section-wide inputs the per-phrase realizer needs. Held together so
/// the realizer signature stays small and so the caller can build the
/// context once and reuse it across every phrase in the section.
pub(super) struct PhraseRenderCtx<'a> {
    pub(super) chords: &'a [TimedChord],
    pub(super) scale: Option<Scale>,
    pub(super) register: (u8, u8),
    pub(super) articulation: f32,
    pub(super) velocity_base: f32,
    pub(super) tpb: u64,
}

/// Realize a single phrase from the motif and its transformation,
/// anchored to the chords and shaped by contour. The Transform is supplied
/// externally so that lanes which want to share transform plans (bass
/// `MirrorMelody` mode) can compute them up-front from a fresh RNG.
pub(super) fn realize_phrase(
    motif: &[MotifNote],
    transform: Transform,
    phrase: &PhrasePlan,
    ctx: &PhraseRenderCtx<'_>,
) -> Vec<GeneratedNote> {
    let transformed = transform_motif(motif, transform);
    if transformed.is_empty() {
        return Vec::new();
    }

    let phrase_chords = &ctx.chords[phrase.chord_range.0..phrase.chord_range.1];
    let register_span = ctx.register.1.saturating_sub(ctx.register.0);
    let register_mid = (ctx.register.0 as u16 + ctx.register.1 as u16) / 2;

    let mut out = Vec::new();
    let sounding_ratio = 1.0 - ctx.articulation * 0.55;
    let min_duration = (ctx.tpb / 8).max(1);
    // Sentence continuations carry twice the surface-rhythm density of
    // the presentation: the fragmented head would otherwise stretch to
    // fill the chord (tiling normalizes durations), so the multiplier
    // first compensates the fragment's shrinkage (motif/transformed
    // ratio) and then doubles the rate — per chord the continuation
    // sounds ~2x the notes of the basic idea. The harmonic rhythm
    // itself is owned by the chord progression; the melody-side
    // planner only accelerates the surface.
    //
    // Fragmentation outside a continuation accelerates too (rhythmic
    // acceleration: note values halve when an idea fragments): the
    // compensation factor alone keeps the fragment's notes at their
    // original surface values instead of letting the tiling stretch
    // them to fill the chord — half the duration each note would get
    // without it.
    let shrink_compensation =
        (motif.len().max(1) as u64).div_ceil(transformed.len() as u64);
    let density: u64 = match transform {
        // A sequence cell is already statement-dense: the model plus
        // its transposed copies span the chord once, accelerating the
        // surface by the statement count. An extra tiling multiplier
        // would wash the statements into sub-statement fragments and
        // bury the sequence's transposition shape.
        Transform::Sequence { .. } => 1,
        _ if phrase.role.is_continuation() => 2 * shrink_compensation,
        // A transposed fragment is still a fragment: the same rhythmic
        // acceleration applies (the compensation factor keeps the
        // fragment's notes at their original surface values).
        Transform::Fragment(_)
        | Transform::Composed(ComposedPair::FragmentTranspose { .. }) => shrink_compensation,
        _ => 1,
    };

    for (ci, tc) in phrase_chords.iter().enumerate() {
        let chord_start = tc.start_beat as u64 * ctx.tpb;
        let chord_ticks = tc.duration_beats as u64 * ctx.tpb;
        if chord_ticks == 0 {
            continue;
        }

        // Position within phrase for contour shaping (0.0 to 1.0).
        let phrase_position = if phrase_chords.len() > 1 {
            ci as f32 / (phrase_chords.len() - 1) as f32
        } else {
            0.5
        };
        // Section climax coordination: secondary phrases trace their
        // contour at reduced amplitude (see `PhrasePlan::peak_scale`).
        let c_offset = (contour_offset(phrase.contour, phrase_position, register_span) as f32
            * phrase.peak_scale)
            .round() as i8;

        // Choose anchor: a chord tone near the contour target.
        let tones = chord_tones_in_register(tc.chord, ctx.register);
        if tones.is_empty() {
            continue;
        }
        let target = (register_mid as i16 + c_offset as i16)
            .clamp(ctx.register.0 as i16, ctx.register.1 as i16) as u8;
        let anchor = nearest_in_set(target, &tones);

        // Scale the motif's duration ratios to fill this chord's time.
        let total_ratio: u64 = transformed.iter().map(|n| n.duration_ratio as u64).sum();
        if total_ratio == 0 {
            continue;
        }

        // Tile the motif to fill the chord duration. If the motif is
        // shorter than the chord, repeat it; if longer, truncate.
        let mut tick_cursor = chord_start;
        let chord_end = chord_start + chord_ticks;
        let mut motif_idx = 0;

        while tick_cursor < chord_end {
            let mn = &transformed[motif_idx % transformed.len()];
            let note_ticks =
                (chord_ticks * mn.duration_ratio as u64 / (total_ratio * density)).max(1);
            let remaining = chord_end - tick_cursor;
            let actual_ticks = note_ticks.min(remaining);

            if actual_ticks < min_duration {
                break;
            }

            if !mn.silent {
                let raw_midi = (anchor as i16 + mn.interval as i16).clamp(0, 127) as u8;
                let raw_clamped = raw_midi.clamp(ctx.register.0, ctx.register.1);

                let beat_pos = tick_cursor - chord_start;
                let aligned = align_to_harmony(
                    raw_clamped,
                    beat_pos,
                    ctx.tpb,
                    tc.chord,
                    ctx.scale,
                    ctx.register,
                );

                let sounding =
                    ((actual_ticks as f64 * sounding_ratio as f64) as u64).max(min_duration);
                let vel = if mn.accent {
                    (ctx.velocity_base + 0.05).min(1.0)
                } else {
                    ctx.velocity_base
                };

                out.push(GeneratedNote {
                    note: aligned,
                    velocity: vel,
                    start_tick: tick_cursor,
                    duration_ticks: sounding,
                });
            }

            tick_cursor += actual_ticks;
            motif_idx += 1;
        }
    }

    // Closing phrases (period consequents and sentence cadential
    // continuations) resolve: snap the last note to the chord *root*.
    // The previous "lowest chord tone in register" shortcut only
    // equals the root when the register floor doesn't cut into the
    // close voicing — otherwise it resolved to a third or fifth.
    // With a scale present this is only the baseline: the goal-cadence
    // overlay (`cadence::apply_cadence_formula`) usually retargets the
    // final two notes to a proper formula afterwards, and falls back
    // to this snap when no formula candidate validates. Without a
    // scale (no degree vocabulary) the snap is the final word.
    if phrase.role.closes() && !out.is_empty() {
        let last_chord = phrase_chords.last().unwrap();
        let last = out.last_mut().unwrap();
        let mut root = nearest_midi_to(last_chord.chord.root, last.note);
        // Pull into register by octaves, preserving the pitch class.
        while root < ctx.register.0 {
            root = root.saturating_add(12);
        }
        while root > ctx.register.1 && root >= 12 {
            root -= 12;
        }
        // Registers narrower than an octave may hold no root at all;
        // clamp as a last resort.
        last.note = root.clamp(ctx.register.0, ctx.register.1);
    }

    out
}
