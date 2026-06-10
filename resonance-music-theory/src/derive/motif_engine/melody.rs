// Top-level motif-based melody generators. Wire together motif
// construction, phrase planning, and harmony alignment.

use crate::chord::Chord;
use crate::rng::XorShift;
use crate::scale::Scale;

use super::super::climax::{enforce_single_climax, ClimaxHarmony};
use super::super::melody::MelodyParams;
use super::super::motif_source::{manual_motif_to_motif_notes, MotifParams, MotifSource};
use super::super::{GeneratedNote, TimedChord};
use super::build::build_motif;
use super::harmony::apply_leap_recovery;
use super::phrase::{plan_motif_transforms, plan_phrases, realize_phrase, PhraseRenderCtx};

/// Extract the motif's signed semitone intervals (relative to its
/// anchor pitch), skipping rests. Used by lanes that don't render the
/// motif themselves but want to trace its melodic shape — e.g. the
/// vocal generator's "use section motif" mode.
///
/// `Generated` motifs are built with `build_motif` using the same RNG
/// flow as the melody renderer so the returned intervals match what
/// the motif lanes produce. `Manual` motifs are read directly from the
/// user-drawn cells via the existing scale-step mapping.
pub fn motif_intervals(
    source: &MotifSource,
    anchor_chord: Chord,
    scale: Option<Scale>,
) -> Vec<i8> {
    let notes = match source {
        MotifSource::Generated(p) => {
            let mut rng = XorShift::new(p.seed);
            build_motif(&mut rng, anchor_chord, scale, p)
        }
        MotifSource::Manual { notes, .. } => manual_motif_to_motif_notes(notes, scale),
    };
    notes
        .iter()
        .filter(|n| !n.silent)
        .map(|n| n.interval)
        .collect()
}

/// Top-level motif-based melody generator.
///
/// Back-compat shim: pulls motif knobs from `MelodyParams`. Direct callers
/// (and the inline tests) keep working unchanged. The app routes through
/// [`derive_motif_melody_with_section`] instead so the section's
/// `MotifSource` wins.
pub(in crate::derive) fn derive_motif_melody(
    chords: &[TimedChord],
    scale: Option<Scale>,
    params: &MelodyParams,
    ticks_per_beat: u32,
    seed: u64,
) -> Vec<GeneratedNote> {
    let source = MotifSource::Generated(MotifParams {
        seed,
        complexity: params.complexity,
        motif_len: params.motif_len,
        leap_chance: params.leap_chance,
    });
    derive_motif_melody_with_section(chords, scale, params, &source, seed, ticks_per_beat)
}

/// Section-aware motif-based melody generator.
///
/// In `MotifSource::Generated` mode, `motif.seed` drives the shared motif
/// (intervals + rhythm + accents) and the per-phrase Transform sequence —
/// both shared across all Motif lanes in a section. In `Manual` mode, the
/// motif cell is taken verbatim from the user-drawn notes and the seed
/// only drives the per-phrase Transform sequence so the motif still
/// develops across phrases.
///
/// `lane_seed` drives lane-local randomness only: phrase contour selection
/// (when `params.contour == Auto`) and rest-density hole placement.
/// Pressing Regenerate on a single lane should bump `lane_seed` so the
/// motif identity stays put while the lane gets a fresh surface variation.
pub fn derive_motif_melody_with_section(
    chords: &[TimedChord],
    scale: Option<Scale>,
    params: &MelodyParams,
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
    let phrases = plan_phrases(chords, params.contour, params.phrase_len, &mut lane_rng);
    let transforms = plan_motif_transforms(
        phrases.len(),
        motif.len(),
        motif_params.complexity,
        motif_params.seed,
    );

    // Per-phrase octave displacement keeps the motif identity intact
    // (same intervals + rhythm) while giving each Regenerate press an
    // audible shift. Without this, lane_seed only nudges contour and
    // rest-density randomization — invisible when the user pinned a
    // specific ContourPreference and rest_density sits at its default 0.
    let phrase_octave_offsets: Vec<i8> = (0..phrases.len())
        .map(|i| {
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

    let mut all_notes = Vec::new();
    let rest_gap = (tpb as f64 * (0.5 + params.rest_density as f64)) as u64;

    let render_ctx = PhraseRenderCtx {
        chords,
        scale,
        register: params.register,
        articulation: params.articulation,
        velocity_base: params.velocity,
        tpb,
    };

    for (pi, phrase) in phrases.iter().enumerate() {
        let mut phrase_notes = realize_phrase(&motif, transforms[pi], phrase, &render_ctx);

        if let Some(scale) = scale {
            apply_leap_recovery(&mut phrase_notes, &scale, params.register);
            // Single-climax rule: one highest note per phrase, in its
            // second half, never the final note. Climax demotion and
            // the leap grammar alternate to a fixpoint: demotion only
            // lowers pitches and the grammar's repairs never lift a
            // pitch back up to the phrase maximum, so the maximum is
            // non-increasing and the loop settles; the cap is
            // belt-and-braces.
            let harmony = ClimaxHarmony {
                chords,
                tpb,
                register: params.register,
            };
            for _ in 0..32 {
                if !enforce_single_climax(
                    &mut phrase_notes,
                    Some(scale),
                    params.register,
                    Some(&harmony),
                    true,
                ) {
                    break;
                }
                apply_leap_recovery(&mut phrase_notes, &scale, params.register);
            }
        }

        if pi > 0 && rest_gap > 0 {
            if let Some(last) = all_notes.last_mut() {
                let last_note: &mut GeneratedNote = last;
                if last_note.duration_ticks > rest_gap {
                    last_note.duration_ticks -= rest_gap;
                }
            }
        }

        let octave_shift = phrase_octave_offsets[pi];
        if octave_shift != 0 {
            for n in phrase_notes.iter_mut() {
                let candidate = (n.note as i16 + octave_shift as i16).clamp(0, 127) as u8;
                if candidate >= params.register.0 && candidate <= params.register.1 {
                    n.note = candidate;
                }
            }
        }

        all_notes.extend(phrase_notes);
    }

    if params.rest_density > 0.0 {
        let mut filtered = Vec::with_capacity(all_notes.len());
        for note in all_notes {
            if lane_rng.next_f32() >= params.rest_density {
                filtered.push(note);
            }
        }
        all_notes = filtered;
    }

    all_notes
}
