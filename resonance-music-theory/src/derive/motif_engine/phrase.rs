// Phrase-level planning and rendering: divide the chord progression into
// phrases, pick a contour for each, choose a Transform sequence across
// phrases, and realize a single phrase into MIDI notes.

use crate::rng::XorShift;
use crate::scale::Scale;
use crate::voicing::nearest_midi_to;

use super::super::melody::ContourPreference;
use super::super::motif_bass::chord_tones_in_register;
use super::super::{GeneratedNote, TimedChord};
use super::build::transform_motif;
use super::harmony::{align_to_harmony, nearest_in_set};
use super::types::{Contour, MotifNote, PhrasePlan, Transform};

/// Pre-compute the per-phrase Transform sequence for a motif plan. Uses a
/// fresh RNG seeded only from `motif.seed` so two callers with the same
/// `MotifParams` always agree on the sequence — this is what makes
/// `BassMotifPhrase::MirrorMelody` lock to the melody.
pub(in crate::derive) fn plan_motif_transforms(
    num_phrases: usize,
    motif_len: usize,
    complexity: f32,
    seed: u64,
) -> Vec<Transform> {
    let mut rng = XorShift::new(seed.wrapping_add(0xA1B2C3D4E5F60718));
    (0..num_phrases)
        .map(|i| pick_transform(motif_len, i, complexity, &mut rng))
        .collect()
}

/// Pick a contour for a phrase from the preference or RNG.
fn pick_contour(pref: ContourPreference, is_consequent: bool, rng: &mut XorShift) -> Contour {
    match pref {
        ContourPreference::Arch => Contour::Arch,
        ContourPreference::Descending => Contour::Descending,
        ContourPreference::Ascending => Contour::Ascending,
        ContourPreference::Wave => Contour::Wave,
        ContourPreference::Auto => {
            // Research-weighted: arch 29%, desc 27%, asc 22%, wave 22%.
            // Consequent phrases bias toward descending (resolution).
            let roll = rng.next_f32();
            if is_consequent {
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

/// Divide chords into phrases and assign contours.
pub(in crate::derive) fn plan_phrases(
    chords: &[TimedChord],
    contour_pref: ContourPreference,
    phrase_len: u8,
    rng: &mut XorShift,
) -> Vec<PhrasePlan> {
    let plen = (phrase_len as usize).max(1);
    let mut plans = Vec::new();
    let mut i = 0;
    let mut phrase_index = 0;

    while i < chords.len() {
        let end = (i + plen).min(chords.len());
        let is_consequent = phrase_index % 2 == 1;
        let contour = pick_contour(contour_pref, is_consequent, rng);
        plans.push(PhrasePlan {
            chord_range: (i, end),
            contour,
            is_consequent,
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
            let note_ticks = (chord_ticks * mn.duration_ratio as u64 / total_ratio).max(1);
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

    // Consequent phrases resolve: snap the last note to the chord
    // *root*. The previous "lowest chord tone in register" shortcut
    // only equals the root when the register floor doesn't cut into
    // the close voicing — otherwise it resolved to a third or fifth.
    if phrase.is_consequent && !out.is_empty() {
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
