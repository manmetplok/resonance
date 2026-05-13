// ---------------------------------------------------------------------------
// Motif-based melody engine
// ---------------------------------------------------------------------------

use crate::chord::Chord;
use crate::rng::XorShift;
use crate::scale::Scale;

use super::bass::step_scale;
use super::melody::{ContourPreference, MelodyParams};
use super::motif_bass::chord_tones_in_register;
use super::motif_source::{manual_motif_to_motif_notes, MotifParams, MotifSource};
use super::{GeneratedNote, TimedChord};

/// A single note in a motif, stored as a relative interval from an anchor
/// pitch so that transposition and inversion are simple arithmetic.
#[derive(Debug, Clone, Copy)]
pub(super) struct MotifNote {
    /// Signed interval in semitones from the motif's anchor pitch.
    pub(super) interval: i8,
    /// Duration as a multiple of a base rhythmic unit.
    pub(super) duration_ratio: u8,
    /// Slight velocity emphasis on this note.
    pub(super) accent: bool,
    /// True if this entry is a rest — the per-chord cursor still advances
    /// by `duration_ratio` but no MIDI note is emitted.
    pub(super) silent: bool,
}

/// Transformation to apply to a motif when developing it across phrases.
#[derive(Debug, Clone, Copy)]
pub(super) enum Transform {
    Identity,
    TransposeUp(i8),
    TransposeDown(i8),
    Invert,
    Retrograde,
    Augment,
    Diminish,
    Fragment(usize),
}

/// Internal contour shape for a phrase.
#[derive(Debug, Clone, Copy)]
enum Contour {
    Arch,
    Descending,
    Ascending,
    Wave,
}

/// Plan for a single melodic phrase.
pub(super) struct PhrasePlan {
    pub(super) chord_range: (usize, usize),
    contour: Contour,
    is_consequent: bool,
}

/// Rhythm pattern library: each pattern is a list of duration ratios.
/// The ratios are scaled to fill the available time. Higher indices are
/// more rhythmically complex.
const RHYTHM_PATTERNS: &[&[u8]] = &[
    &[1, 1, 1, 1],       // steady
    &[2, 1, 1],           // long-short-short
    &[1, 1, 2],           // short-short-long
    &[1, 2, 1],           // short-long-short
    &[3, 1, 2, 2],        // dotted feel
    &[1, 1, 1, 1, 2],     // four eighths + quarter
    &[2, 1, 1, 2, 2],     // varied
    &[1, 1, 2, 1, 1],     // syncopated center
];

/// Build a motif: a short melodic cell of 2-6 notes with relative intervals
/// and a rhythmic pattern. Intervals are unbounded by lane register — each
/// lane clamps to its own register at render time, so two lanes built from
/// the same `MotifParams` and chord get identical interval shapes.
pub(super) fn build_motif(
    rng: &mut XorShift,
    chord: Chord,
    scale: Option<Scale>,
    motif: &MotifParams,
) -> Vec<MotifNote> {
    let len = if motif.motif_len > 0 {
        (motif.motif_len as usize).clamp(2, 6)
    } else {
        (2.0 + motif.complexity * 4.0).round() as usize
    };

    // Pick a rhythm pattern. Higher complexity biases toward later
    // (more complex) patterns.
    let max_pattern = (motif.complexity * (RHYTHM_PATTERNS.len() - 1) as f32).ceil() as usize;
    let pattern_idx = rng.next_range(max_pattern.max(1) + 1).min(RHYTHM_PATTERNS.len() - 1);
    let rhythm = RHYTHM_PATTERNS[pattern_idx];

    // Build interval contour.
    let chord_intervals = chord_tone_intervals(&chord);
    let has_scale = scale.is_some();
    let mut notes = Vec::with_capacity(len);
    let mut current_interval: i8 = 0;

    for i in 0..len {
        let duration_ratio = rhythm[i % rhythm.len()];
        let accent = i == 0 || duration_ratio >= 2;

        if i == 0 {
            notes.push(MotifNote {
                interval: 0,
                duration_ratio,
                accent,
                silent: false,
            });
            continue;
        }

        // Choose: step, leap, or repeat.
        let roll = rng.next_f32();
        let repeat_chance = 0.11;
        let step_chance = 1.0 - motif.leap_chance - repeat_chance;

        let new_interval = if roll < repeat_chance {
            current_interval
        } else if roll < repeat_chance + step_chance {
            let step_size = if rng.next_f32() < 0.6 { 1 } else { 2 };
            let dir: i8 = if rng.next_f32() < 0.5 { 1 } else { -1 };
            let candidate = current_interval + dir * step_size;
            if has_scale {
                candidate
            } else {
                snap_to_chord_interval(candidate, &chord_intervals)
            }
        } else {
            let leap_size = 3 + (rng.next_f32() * 4.0) as i8;
            let dir: i8 = if rng.next_f32() < 0.5 { 1 } else { -1 };
            let candidate = current_interval + dir * leap_size;
            if has_scale {
                candidate
            } else {
                snap_to_chord_interval(candidate, &chord_intervals)
            }
        };

        current_interval = new_interval.clamp(-10, 10);

        notes.push(MotifNote {
            interval: current_interval,
            duration_ratio,
            accent,
            silent: false,
        });
    }

    if let Some(last) = notes.last_mut() {
        last.interval = snap_to_chord_interval(last.interval, &chord_intervals);
    }

    notes
}

/// Pre-compute the per-phrase Transform sequence for a motif plan. Uses a
/// fresh RNG seeded only from `motif.seed` so two callers with the same
/// `MotifParams` always agree on the sequence — this is what makes
/// `BassMotifPhrase::MirrorMelody` lock to the melody.
pub(super) fn plan_motif_transforms(
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


/// Get the semitone intervals of a chord's pitch classes relative to
/// the root (e.g. major = [0, 4, 7]).
pub(super) fn chord_tone_intervals(chord: &Chord) -> Vec<i8> {
    let root = chord.root.to_semitone() as i8;
    chord
        .pitch_classes()
        .iter()
        .map(|pc| {
            let diff = pc.to_semitone() as i8 - root;
            if diff < 0 { diff + 12 } else { diff }
        })
        .collect()
}

/// Snap an interval to the nearest chord-tone interval (mod 12).
pub(super) fn snap_to_chord_interval(interval: i8, chord_intervals: &[i8]) -> i8 {
    if chord_intervals.is_empty() {
        return interval;
    }
    let norm = interval.rem_euclid(12);
    let octave = interval - norm;
    let mut best = chord_intervals[0];
    let mut best_dist = 12i8;
    for &ci in chord_intervals {
        let dist = ((norm - ci).abs()).min((norm - ci + 12).abs()).min((norm - ci - 12).abs());
        if dist < best_dist {
            best_dist = dist;
            best = ci;
        }
    }
    octave + best
}

/// Apply a transformation to a motif, returning a new motif.
pub(super) fn transform_motif(motif: &[MotifNote], transform: Transform) -> Vec<MotifNote> {
    match transform {
        Transform::Identity => motif.to_vec(),
        Transform::TransposeUp(n) => motif
            .iter()
            .map(|note| MotifNote {
                interval: note.interval + n,
                ..*note
            })
            .collect(),
        Transform::TransposeDown(n) => motif
            .iter()
            .map(|note| MotifNote {
                interval: note.interval - n,
                ..*note
            })
            .collect(),
        Transform::Invert => motif
            .iter()
            .map(|note| MotifNote {
                interval: -note.interval,
                ..*note
            })
            .collect(),
        Transform::Retrograde => {
            let mut reversed = motif.to_vec();
            reversed.reverse();
            reversed
        }
        Transform::Augment => motif
            .iter()
            .map(|note| MotifNote {
                duration_ratio: note.duration_ratio.saturating_mul(2).max(1),
                ..*note
            })
            .collect(),
        Transform::Diminish => motif
            .iter()
            .map(|note| MotifNote {
                duration_ratio: (note.duration_ratio / 2).max(1),
                ..*note
            })
            .collect(),
        Transform::Fragment(n) => motif[..n.min(motif.len())].to_vec(),
    }
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
pub(super) fn plan_phrases(
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

/// Align a MIDI note to the current harmony based on beat strength.
pub(super) fn align_to_harmony(
    raw_midi: u8,
    beat_position: u64,
    tpb: u64,
    chord: Chord,
    scale: Option<Scale>,
    register: (u8, u8),
) -> u8 {
    let chord_tones = chord_tones_in_register(chord, register);
    if chord_tones.is_empty() {
        return raw_midi.clamp(register.0, register.1);
    }

    // Strong beat: position is a multiple of 2 beats.
    let is_strong = tpb > 0 && beat_position.is_multiple_of(2 * tpb);

    if is_strong {
        // Must be a chord tone.
        if chord_tones.contains(&raw_midi) {
            return raw_midi;
        }
        return nearest_in_set(raw_midi, &chord_tones);
    }

    // Weak beat: allow scale tones.
    if let Some(scale) = scale {
        if scale.contains(raw_midi) {
            return raw_midi;
        }
        // Snap to nearest scale tone in register.
        let up = step_scale(&scale, raw_midi, 1);
        let down = step_scale(&scale, raw_midi, -1);
        let d_up = (up as i16 - raw_midi as i16).unsigned_abs() as u8;
        let d_down = (down as i16 - raw_midi as i16).unsigned_abs() as u8;
        let snapped = if d_up <= d_down { up } else { down };
        return snapped.clamp(register.0, register.1);
    }

    // No scale: snap to chord tone.
    nearest_in_set(raw_midi, &chord_tones)
}

/// Find the nearest value in a sorted set to the target.
pub(super) fn nearest_in_set(target: u8, set: &[u8]) -> u8 {
    let mut best = set[0];
    let mut best_dist = (target as i16 - best as i16).unsigned_abs();
    for &v in &set[1..] {
        let dist = (target as i16 - v as i16).unsigned_abs();
        if dist < best_dist {
            best = v;
            best_dist = dist;
        }
    }
    best
}

/// Post-processing: resolve large leaps (>5 semitones) with stepwise
/// fill notes in the opposite direction.
pub(super) fn apply_gap_fill(notes: &mut Vec<GeneratedNote>, scale: &Scale, register: (u8, u8)) {
    let mut i = 0;
    while i + 1 < notes.len() {
        let leap = notes[i + 1].note as i16 - notes[i].note as i16;
        if leap.unsigned_abs() > 5 {
            let fill_dir: i32 = if leap > 0 { -1 } else { 1 };
            // Check if next notes already resolve the leap.
            let already_filled = (i + 2 < notes.len()) && {
                let next_step = notes[i + 2].note as i16 - notes[i + 1].note as i16;
                (fill_dir > 0 && next_step > 0) || (fill_dir < 0 && next_step < 0)
            };
            if !already_filled {
                // Insert 1-2 fill notes by splitting the post-leap note's duration.
                let fill_count = if leap.unsigned_abs() > 7 { 2 } else { 1 };
                let post = &notes[i + 1];
                if post.duration_ticks > fill_count as u64 * 60 {
                    let fill_dur = post.duration_ticks / (fill_count as u64 + 1);
                    let mut fill_notes = Vec::new();
                    let mut cur = post.note;
                    let orig_start = post.start_tick;
                    for f in 0..fill_count {
                        cur = step_scale(scale, cur, fill_dir);
                        cur = cur.clamp(register.0, register.1);
                        fill_notes.push(GeneratedNote {
                            note: cur,
                            velocity: post.velocity * 0.9,
                            start_tick: orig_start + post.duration_ticks - (fill_count - f) as u64 * fill_dur,
                            duration_ticks: fill_dur,
                        });
                    }
                    // Shorten the post-leap note.
                    notes[i + 1].duration_ticks -= fill_count as u64 * fill_dur;
                    let insert_pos = i + 2;
                    for (j, note) in fill_notes.into_iter().enumerate() {
                        notes.insert(insert_pos + j, note);
                    }
                    i += 1 + fill_count; // skip past inserted notes
                    continue;
                }
            }
        }
        i += 1;
    }
}

/// Realize a single phrase from the motif and its transformation,
/// anchored to the chords and shaped by contour. The Transform is supplied
/// externally so that lanes which want to share transform plans (bass
/// `MirrorMelody` mode) can compute them up-front from a fresh RNG.
#[allow(clippy::too_many_arguments)]
fn realize_phrase(
    motif: &[MotifNote],
    transform: Transform,
    phrase: &PhrasePlan,
    chords: &[TimedChord],
    scale: Option<Scale>,
    register: (u8, u8),
    articulation: f32,
    velocity_base: f32,
    tpb: u64,
) -> Vec<GeneratedNote> {
    let transformed = transform_motif(motif, transform);
    if transformed.is_empty() {
        return Vec::new();
    }

    let phrase_chords = &chords[phrase.chord_range.0..phrase.chord_range.1];
    let register_span = register.1.saturating_sub(register.0);
    let register_mid = (register.0 as u16 + register.1 as u16) / 2;

    let mut out = Vec::new();
    let sounding_ratio = 1.0 - articulation * 0.55;
    let min_duration = (tpb / 8).max(1);

    for (ci, tc) in phrase_chords.iter().enumerate() {
        let chord_start = tc.start_beat as u64 * tpb;
        let chord_ticks = tc.duration_beats as u64 * tpb;
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
        let tones = chord_tones_in_register(tc.chord, register);
        if tones.is_empty() {
            continue;
        }
        let target = (register_mid as i16 + c_offset as i16).clamp(register.0 as i16, register.1 as i16) as u8;
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
                let raw_clamped = raw_midi.clamp(register.0, register.1);

                let beat_pos = tick_cursor - chord_start;
                let aligned =
                    align_to_harmony(raw_clamped, beat_pos, tpb, tc.chord, scale, register);

                let sounding =
                    ((actual_ticks as f64 * sounding_ratio as f64) as u64).max(min_duration);
                let vel = if mn.accent {
                    (velocity_base + 0.05).min(1.0)
                } else {
                    velocity_base
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

    // Consequent phrases resolve: snap the last note to the chord root.
    if phrase.is_consequent && !out.is_empty() {
        let last_chord = phrase_chords.last().unwrap();
        let root_tones = chord_tones_in_register(last_chord.chord, register);
        if let Some(root) = root_tones.first() {
            // Find the chord root (lowest chord tone = root in close position).
            let last = out.last_mut().unwrap();
            last.note = nearest_in_set(last.note, &[*root]);
        }
    }

    out
}

/// Top-level motif-based melody generator.
///
/// Back-compat shim: pulls motif knobs from `MelodyParams`. Direct callers
/// (and the inline tests) keep working unchanged. The app routes through
/// [`derive_motif_melody_with_section`] instead so the section's
/// `MotifSource` wins.
pub(super) fn derive_motif_melody(
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

    for (pi, phrase) in phrases.iter().enumerate() {
        let mut phrase_notes = realize_phrase(
            &motif,
            transforms[pi],
            phrase,
            chords,
            scale,
            params.register,
            params.articulation,
            params.velocity,
            tpb,
        );

        if let Some(scale) = scale {
            apply_gap_fill(&mut phrase_notes, &scale, params.register);
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
