// ---------------------------------------------------------------------------
// Bass motif renderer
// ---------------------------------------------------------------------------

use crate::chord::Chord;
use crate::rng::XorShift;
use crate::scale::Scale;
use crate::voicing::nearest_midi_above;

use super::bass::{BassMotifMode, BassMotifPhrase, BassParams};
use super::melody::ContourPreference;
use super::motif_engine::{
    align_to_harmony, build_motif, plan_motif_transforms, plan_phrases, transform_motif,
    MotifNote, Transform,
};
use super::motif_source::{manual_motif_to_motif_notes, MotifSource};
use super::{GeneratedNote, TimedChord};

/// Section-aware motif-based bass generator. Builds the shared motif from
/// `motif_source` + the first chord (or takes it verbatim when manual),
/// then renders it across the chord progression according to
/// `bass.motif_mode` (what part of the motif to use) and
/// `bass.motif_phrase` (how to develop it across phrases).
///
/// In `Generated` mode, `motif.seed` drives both the motif identity and
/// the Transform plan so a melody Motif lane in the same section produces
/// matching interval shapes (and, in `MirrorMelody` mode, matching
/// transforms). In `Manual` mode, the cell is taken verbatim and only the
/// Transform plan is seeded.
///
/// `lane_seed` drives only this lane's phrase-contour selection —
/// pressing Regenerate on the bass lane bumps `lane_seed` so the bass
/// surface varies while the shared motif stays put.
pub fn derive_bass_motif(
    chords: &[TimedChord],
    scale: Option<Scale>,
    bass: &BassParams,
    motif_source: &MotifSource,
    lane_seed: u64,
    ticks_per_beat: u32,
) -> Vec<GeneratedNote> {
    if chords.is_empty() {
        return Vec::new();
    }
    let tpb = ticks_per_beat as u64;

    let motif_params = motif_source.params();
    let motif = match motif_source {
        MotifSource::Generated(p) => {
            let mut motif_rng = XorShift::new(p.seed);
            build_motif(&mut motif_rng, chords[0].chord, scale, p)
        }
        MotifSource::Manual { notes, .. } => manual_motif_to_motif_notes(notes, scale),
    };
    if motif.is_empty() {
        return Vec::new();
    }

    let mut lane_rng = XorShift::new(lane_seed);
    let phrases = plan_phrases(
        chords,
        ContourPreference::Auto,
        4,
        motif_params.seed,
        &mut lane_rng,
    );

    // Restricted mode picks transforms from the lane RNG so pressing
    // Regenerate produces a fresh per-phrase plan without disturbing the
    // section's shared motif. Simple stays Identity, MirrorMelody stays
    // locked to the melody.
    let transforms: Vec<Transform> = match bass.motif_phrase {
        BassMotifPhrase::Simple => vec![Transform::Identity; phrases.len()],
        BassMotifPhrase::MirrorMelody => plan_motif_transforms(
            phrases.len(),
            motif.len(),
            motif_params.complexity,
            motif_params.seed,
        ),
        BassMotifPhrase::Restricted => {
            let mut restricted_rng = XorShift::new(lane_seed.wrapping_add(0xB1A2_5E55_C0FF_EE01));
            (0..phrases.len())
                .map(|i| {
                    if i == 0 || restricted_rng.next_f32() < 0.5 {
                        Transform::Identity
                    } else {
                        Transform::Augment
                    }
                })
                .collect()
        }
    };

    // Per-phrase octave displacement keeps the bass motivically identical
    // (same intervals and rhythm) while giving each Regenerate press an
    // audible shift — phrases occasionally jump up an octave or drop down,
    // staying inside the bass register at render time. This is the main
    // source of lane-local variation for Simple and MirrorMelody modes,
    // which otherwise have no per-lane randomness.
    let phrase_octave_offsets: Vec<i8> = (0..phrases.len())
        .map(|i| {
            // First phrase always at the anchor octave so the section
            // opens on the expected pitch.
            if i == 0 {
                return 0;
            }
            let roll = lane_rng.next_f32();
            if roll < 0.55 {
                0
            } else if roll < 0.85 {
                12
            } else {
                -12
            }
        })
        .collect();

    let mut out = Vec::new();
    for (pi, phrase) in phrases.iter().enumerate() {
        let transformed = transform_motif(&motif, transforms[pi]);
        if transformed.is_empty() {
            continue;
        }
        let octave_shift = phrase_octave_offsets[pi];
        let phrase_chords = &chords[phrase.chord_range.0..phrase.chord_range.1];

        for tc in phrase_chords {
            let chord_start = tc.start_beat as u64 * tpb;
            let chord_ticks = tc.duration_beats as u64 * tpb;
            if chord_ticks == 0 {
                continue;
            }
            let bass_pc = tc.chord.bass.unwrap_or(tc.chord.root);
            let base_anchor = nearest_midi_above(bass_pc, bass.base_note);
            let anchor = shifted_anchor(base_anchor, octave_shift, bass);

            render_bass_motif_chord(
                &transformed,
                tc,
                anchor,
                bass,
                scale,
                chord_start,
                chord_ticks,
                tpb,
                &mut out,
            );
        }
    }

    out
}

/// Apply an octave shift to a bass anchor while keeping it inside the
/// configured bass window (`base_note ..= base_note + 24`). If the shift
/// would take the anchor outside that window, fall back to the unshifted
/// anchor — the variation is musical, not a license to leave the register.
fn shifted_anchor(base_anchor: u8, octave_shift: i8, bass: &BassParams) -> u8 {
    if octave_shift == 0 {
        return base_anchor;
    }
    let lo = bass.base_note;
    let hi = bass.base_note.saturating_add(24);
    let candidate = (base_anchor as i16 + octave_shift as i16).clamp(0, 127) as u8;
    if candidate >= lo && candidate <= hi {
        candidate
    } else {
        base_anchor
    }
}

#[allow(clippy::too_many_arguments)]
fn render_bass_motif_chord(
    motif: &[MotifNote],
    tc: &TimedChord,
    anchor: u8,
    bass: &BassParams,
    scale: Option<Scale>,
    chord_start: u64,
    chord_ticks: u64,
    tpb: u64,
    out: &mut Vec<GeneratedNote>,
) {
    let min_duration = (tpb / 8).max(1);
    let bass_register = (bass.base_note, bass.base_note.saturating_add(24));

    match bass.motif_mode {
        BassMotifMode::FirstNoteOnly => {
            // Find the first non-rest entry to use as the chord's bass note.
            // If the motif starts with a rest, fall through to the next
            // sounding entry; if the entire motif is rests, emit nothing.
            let Some(first) = motif.iter().find(|n| !n.silent) else {
                return;
            };
            let vel = if first.accent {
                (bass.velocity + 0.05).min(1.0)
            } else {
                bass.velocity
            };
            out.push(GeneratedNote {
                note: anchor,
                velocity: vel,
                start_tick: chord_start,
                duration_ticks: chord_ticks,
            });
        }
        BassMotifMode::RhythmOnly => {
            let total_ratio: u64 = motif.iter().map(|n| n.duration_ratio as u64).sum();
            if total_ratio == 0 {
                return;
            }
            let mut tick_cursor = chord_start;
            let chord_end = chord_start + chord_ticks;
            let mut idx = 0;
            while tick_cursor < chord_end {
                let mn = &motif[idx % motif.len()];
                let note_ticks = (chord_ticks * mn.duration_ratio as u64 / total_ratio).max(1);
                let remaining = chord_end - tick_cursor;
                let actual = note_ticks.min(remaining);
                if actual < min_duration {
                    break;
                }
                if !mn.silent {
                    let vel = if mn.accent {
                        (bass.velocity + 0.05).min(1.0)
                    } else {
                        bass.velocity
                    };
                    out.push(GeneratedNote {
                        note: anchor,
                        velocity: vel,
                        start_tick: tick_cursor,
                        duration_ticks: actual,
                    });
                }
                tick_cursor += actual;
                idx += 1;
            }
        }
        BassMotifMode::SameIntervals | BassMotifMode::Augmented => {
            let augment = bass.motif_mode == BassMotifMode::Augmented;
            let total_ratio: u64 = motif
                .iter()
                .map(|n| n.duration_ratio as u64 * if augment { 2 } else { 1 })
                .sum();
            if total_ratio == 0 {
                return;
            }
            let mut tick_cursor = chord_start;
            let chord_end = chord_start + chord_ticks;
            let mut idx = 0;
            while tick_cursor < chord_end {
                let mn = &motif[idx % motif.len()];
                let dr = mn.duration_ratio as u64 * if augment { 2 } else { 1 };
                let note_ticks = (chord_ticks * dr / total_ratio).max(1);
                let remaining = chord_end - tick_cursor;
                let actual = note_ticks.min(remaining);
                if actual < min_duration {
                    break;
                }
                if !mn.silent {
                    let raw = (anchor as i16 + mn.interval as i16).clamp(0, 127) as u8;
                    let clamped = raw.clamp(bass_register.0, bass_register.1);
                    let beat_pos = tick_cursor - chord_start;
                    let aligned =
                        align_to_harmony(clamped, beat_pos, tpb, tc.chord, scale, bass_register);
                    let vel = if mn.accent {
                        (bass.velocity + 0.05).min(1.0)
                    } else {
                        bass.velocity
                    };
                    out.push(GeneratedNote {
                        note: aligned,
                        velocity: vel,
                        start_tick: tick_cursor,
                        duration_ticks: actual,
                    });
                }
                tick_cursor += actual;
                idx += 1;
            }
        }
    }
}

/// Every MIDI note inside `register` whose pitch class appears in
/// `chord`, sorted ascending and deduplicated.
pub(super) fn chord_tones_in_register(chord: Chord, register: (u8, u8)) -> Vec<u8> {
    // Pitch-class bitmap: one O(|pcs|) pass to build, then the register
    // scan is O(1) per note instead of a linear `contains` probe.
    let mut is_chord_tone = [false; 12];
    for pc in chord.pitch_classes() {
        is_chord_tone[pc.to_semitone() as usize] = true;
    }
    let (lo, hi) = register;
    // `lo..=hi` is ascending with unique values, so the result is
    // already sorted and deduplicated.
    (lo..=hi).filter(|midi| is_chord_tone[(midi % 12) as usize]).collect()
}
