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
use super::types::{Contour, MotifNote, PhraseGrammarRole, PhrasePlan, Transform};

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
///   - `Continuation` / `ContinuationCadence`: fragmentation of the
///     idea's *head* motive (`Fragment` keeps the leading notes); the
///     realizer doubles the tiling rate for these roles.
///   - `Antecedent`: identity for the section opener, then the full
///     complexity-weighted repertoire.
///   - `Consequent`: reuses its antecedent's transform verbatim — the
///     period's defining "same opening, different ending"; the ending
///     swap (weak→strong) lives in the cadence goals.
pub(in crate::derive) fn plan_motif_transforms(
    num_phrases: usize,
    motif_len: usize,
    complexity: f32,
    seed: u64,
) -> Vec<Transform> {
    let roles = phrase_grammar_roles(num_phrases, seed);
    let mut rng = XorShift::new(seed.wrapping_add(0xA1B2C3D4E5F60718));
    let head_len = 2.max(motif_len / 2);
    let mut last_antecedent = Transform::Identity;
    roles
        .iter()
        .enumerate()
        .map(|(i, role)| match role {
            PhraseGrammarRole::BasicIdea => Transform::Identity,
            PhraseGrammarRole::VariedRepeat => pick_varied_repeat(&mut rng),
            PhraseGrammarRole::Continuation | PhraseGrammarRole::ContinuationCadence => {
                Transform::Fragment(head_len)
            }
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
/// exact repeat or a small transposition — never an operation that
/// obscures the idea (inversion, retrograde, fragmentation).
fn pick_varied_repeat(rng: &mut XorShift) -> Transform {
    let roll = rng.next_f32();
    let amount = 1 + rng.next_range(3) as i8;
    if roll < 0.34 {
        Transform::Identity
    } else if roll < 0.67 {
        Transform::TransposeUp(amount)
    } else {
        Transform::TransposeDown(amount)
    }
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
    let mut plans = Vec::with_capacity(num_phrases);
    let mut i = 0;
    let mut phrase_index = 0;

    while i < chords.len() {
        let end = (i + plen).min(chords.len());
        let role = roles[phrase_index];
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
        // Moderate: add inversion and fragmentation
        if roll < 0.20 {
            Transform::Identity
        } else if roll < 0.40 {
            Transform::TransposeUp(transpose_amount)
        } else if roll < 0.60 {
            Transform::TransposeDown(transpose_amount)
        } else if roll < 0.75 {
            Transform::Invert
        } else {
            let frag_len = 2.max(motif_len / 2);
            Transform::Fragment(frag_len)
        }
    } else {
        // Complex: full repertoire
        if roll < 0.10 {
            Transform::Identity
        } else if roll < 0.25 {
            Transform::TransposeUp(transpose_amount)
        } else if roll < 0.40 {
            Transform::TransposeDown(transpose_amount)
        } else if roll < 0.55 {
            Transform::Invert
        } else if roll < 0.65 {
            Transform::Retrograde
        } else if roll < 0.75 {
            Transform::Augment
        } else if roll < 0.85 {
            Transform::Diminish
        } else {
            let frag_len = 2.max(motif_len / 2);
            Transform::Fragment(frag_len)
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
    let density: u64 = if phrase.role.is_continuation() {
        2 * (motif.len().max(1) as u64).div_ceil(transformed.len() as u64)
    } else {
        1
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
        let c_offset = contour_offset(phrase.contour, phrase_position, register_span);

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
