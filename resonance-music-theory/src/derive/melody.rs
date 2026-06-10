use serde::{Deserialize, Serialize};

use crate::rng::XorShift;
use crate::scale::Scale;

use super::motif_bass::chord_tones_in_register;
use super::motif_engine::derive_motif_melody;
use super::{GeneratedNote, TimedChord};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
    strum::Display, strum::IntoStaticStr, strum::EnumString,
)]
pub enum MelodyStyle {
    #[strum(serialize = "Arp up")]
    ArpUp,
    #[strum(serialize = "Arp down")]
    ArpDown,
    #[strum(serialize = "Arp up/down")]
    ArpUpDown,
    /// Motif-based melodic development with phrase structure,
    /// chord-tone targeting, rhythmic variation, and contour shaping.
    #[serde(alias = "ScaleWalk")]
    Motif,
}

impl MelodyStyle {
    pub const ALL: [MelodyStyle; 4] = [
        MelodyStyle::ArpUp,
        MelodyStyle::ArpDown,
        MelodyStyle::ArpUpDown,
        MelodyStyle::Motif,
    ];

    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// Preferred melodic contour shape for motif-based generation.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default,
    strum::Display, strum::IntoStaticStr, strum::EnumString,
)]
pub enum ContourPreference {
    /// RNG picks per-phrase, weighted by research distributions.
    #[default]
    Auto,
    /// Rise then fall (most common in folk/pop).
    Arch,
    /// Gradual descent.
    Descending,
    /// Gradual ascent.
    Ascending,
    /// Alternating peaks and valleys.
    Wave,
}

impl ContourPreference {
    pub const ALL: [ContourPreference; 5] = [
        ContourPreference::Auto,
        ContourPreference::Arch,
        ContourPreference::Descending,
        ContourPreference::Ascending,
        ContourPreference::Wave,
    ];

    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// Embellishing-tone vocabulary weighting for the motif engine's
/// decoration pass (Open Music Theory v2, embellishing tones). Each
/// named style weights the table differently; the pass itself always
/// honors the dissonance discipline (never leap both into and out of
/// a dissonance; strong-beat dissonances resolve down by step).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default,
    strum::Display, strum::IntoStaticStr, strum::EnumString,
)]
pub enum EmbellishmentStyle {
    /// Pick one of the named styles per section from the motif seed,
    /// so every lane in a section decorates with the same flavor.
    #[default]
    Auto,
    /// Passing and neighbor tones only — stepwise, consonant surface.
    Folk,
    /// Suspensions and appoggiaturas: expressive strong-beat
    /// dissonances resolving down by step.
    #[strum(serialize = "Pop ballad")]
    PopBallad,
    /// Anticipations and escape tones: forward-leaning, syncopated
    /// dissonance treatment.
    Jazz,
}

impl EmbellishmentStyle {
    pub const ALL: [EmbellishmentStyle; 4] = [
        EmbellishmentStyle::Auto,
        EmbellishmentStyle::Folk,
        EmbellishmentStyle::PopBallad,
        EmbellishmentStyle::Jazz,
    ];

    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MelodyParams {
    pub style: MelodyStyle,
    pub register: (u8, u8),
    /// Length of one melody note in ticks. 240 = 8ths at TPQN=480,
    /// 120 = 16ths, 480 = quarter notes. Used by arp styles only.
    pub note_value_ticks: u32,
    /// Probability in [0, 1] that any given slot is silent.
    pub rest_density: f32,
    pub velocity: f32,
    /// 0.0 = very simple/repetitive, 1.0 = maximum development.
    /// Controls transformation variety, motif length, harmonic tension.
    /// Only used by the Motif style.
    #[serde(default = "default_complexity")]
    pub complexity: f32,
    /// 0.0 = very legato, 1.0 = very staccato. Controls the ratio of
    /// sounding duration to rhythmic slot. Only used by the Motif style.
    #[serde(default = "default_articulation")]
    pub articulation: f32,
    /// Preferred melodic contour shape. Only used by the Motif style.
    #[serde(default)]
    pub contour: ContourPreference,
    /// Phrase length in chords (2, 4, or 8). Only used by the Motif style.
    #[serde(default = "default_phrase_len")]
    pub phrase_len: u8,
    /// Motif length override (0 = auto from complexity). Only used by
    /// the Motif style.
    #[serde(default)]
    pub motif_len: u8,
    /// Probability of a leap vs step when generating motif intervals.
    /// Only used by the Motif style.
    #[serde(default = "default_leap_chance")]
    pub leap_chance: f32,
    /// Embellishing-tone style for the decoration pass. Only used by
    /// the Motif style.
    #[serde(default)]
    pub embellishment: EmbellishmentStyle,
    /// When true the lane only sounds where the section's vocal lane is
    /// silent — a "fill" between vocal phrases (call-and-response). The
    /// vocal occupancy mask is collected by the call site and applied
    /// post-derivation via [`drop_overlapping_vocal`]; the generator
    /// itself is unaware of the vocal.
    #[serde(default)]
    pub fill_vocal_gaps: bool,
}

fn default_complexity() -> f32 {
    0.5
}
fn default_articulation() -> f32 {
    0.3
}
fn default_phrase_len() -> u8 {
    4
}
fn default_leap_chance() -> f32 {
    0.21
}

impl Default for MelodyParams {
    fn default() -> Self {
        Self {
            style: MelodyStyle::ArpUp,
            register: (67, 88), // G4..E6
            note_value_ticks: 240,
            rest_density: 0.0,
            velocity: 0.8,
            complexity: default_complexity(),
            articulation: default_articulation(),
            contour: ContourPreference::default(),
            phrase_len: default_phrase_len(),
            motif_len: 0,
            leap_chance: default_leap_chance(),
            embellishment: EmbellishmentStyle::default(),
            fill_vocal_gaps: false,
        }
    }
}

/// Generate a chord-tone arp that *fills the silences* between
/// `vocal_spans`. Walks each silence (the section span minus every
/// occupied interval) in `params.note_value_ticks` steps anchored
/// to the silence start, emitting a chord-tone note per step. Tones
/// come from the chord active at the slot's beat and are rotated by
/// `params.style` (ArpUp / ArpDown / ArpUpDown; Motif falls back to
/// ArpUp because the motif's own phrase rests would defeat the
/// "fill every gap" goal). Each note's tail is trimmed to leave a
/// `min_gap_ticks` margin before the next vocal onset / section end.
/// Slots that can't produce at least a 32nd-note's worth of sounding
/// time are skipped so the output doesn't include sub-perceptible
/// 1-tick stubs jammed up against the next syllable.
///
/// `vocal_spans` must be **phrase-level** intervals (one per lyric
/// line), not per-syllable notes — the silences between syllables
/// inside a single phrase are big enough for the arp to wedge a
/// stub into, which yields a jittery fill that fights the vocal
/// instead of complementing it. Build the spans with
/// [`crate::vocal_phrase_spans`] from the vocal lane's notes + draft.
///
/// Used to implement `MelodyParams::fill_vocal_gaps`. When
/// `vocal_spans` is empty the whole section gets filled.
pub fn derive_melody_fill_vocal(
    chords: &[TimedChord],
    params: &MelodyParams,
    vocal_spans: &[(u64, u64)],
    section_end_ticks: u64,
    ticks_per_beat: u32,
    min_gap_ticks: u64,
) -> Vec<GeneratedNote> {
    if chords.is_empty() || section_end_ticks == 0 {
        return Vec::new();
    }
    let tpb = ticks_per_beat as u64;
    let slot_ticks = params.note_value_ticks.max(1) as u64;

    // Sort + collapse phrase intervals so we can walk silences cleanly.
    // Multiple vocal lanes may overlap; phrase spans within a lane
    // typically don't, but we handle both with the same merge.
    let mut intervals: Vec<(u64, u64)> = vocal_spans.to_vec();
    intervals.sort_by_key(|i| i.0);
    let mut merged: Vec<(u64, u64)> = Vec::with_capacity(intervals.len());
    for (s, e) in intervals {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }

    // Build silence intervals: (gap_start, gap_end) for every region
    // not covered by any vocal note within the section.
    let mut silences: Vec<(u64, u64)> = Vec::with_capacity(merged.len() + 1);
    let mut cursor = 0u64;
    for (s, e) in &merged {
        if *s > cursor {
            silences.push((cursor, *s));
        }
        cursor = cursor.max(*e);
    }
    if cursor < section_end_ticks {
        silences.push((cursor, section_end_ticks));
    }

    // Min sounding duration: a 32nd note (= slot/8 for an 8th, slot/4
    // for a 16th). Below this we drop the slot — sub-perceptible.
    let min_sounding = (slot_ticks / 4).max(1);
    let mut out = Vec::new();
    let mut chord_idx = 0usize;

    for (gap_start, gap_end) in silences {
        let mut slot_start = gap_start;
        let mut slot_in_gap = 0usize;
        while slot_start + min_sounding <= gap_end {
            let onset = slot_start;
            slot_start += slot_ticks;

            // Find the active chord for this onset.
            let beat = (onset / tpb) as u32;
            while chord_idx + 1 < chords.len() && chords[chord_idx + 1].start_beat <= beat {
                chord_idx += 1;
            }
            // Walk back if a previous gap pushed chord_idx ahead of us.
            while chord_idx > 0 && chords[chord_idx].start_beat > beat {
                chord_idx -= 1;
            }
            let tc = &chords[chord_idx];
            let tones = chord_tones_in_register(tc.chord, params.register);
            if tones.is_empty() {
                slot_in_gap += 1;
                continue;
            }

            let note = match params.style {
                MelodyStyle::ArpDown => tones[tones.len() - 1 - (slot_in_gap % tones.len())],
                MelodyStyle::ArpUpDown => {
                    let n = tones.len();
                    if n < 2 {
                        tones[0]
                    } else {
                        let cycle = 2 * n - 2;
                        let idx = slot_in_gap % cycle;
                        if idx < n {
                            tones[idx]
                        } else {
                            tones[cycle - idx]
                        }
                    }
                }
                // ArpUp + Motif (fallback): walk tones bottom-up.
                _ => tones[slot_in_gap % tones.len()],
            };
            slot_in_gap += 1;

            // Cap the tail so we leave `min_gap_ticks` before the next
            // vocal entry (= gap_end) and at least `min_sounding` ticks
            // of audible note.
            let max_end = gap_end.saturating_sub(min_gap_ticks);
            let dur = slot_ticks.min(max_end.saturating_sub(onset));
            if dur < min_sounding {
                break;
            }

            out.push(GeneratedNote {
                note,
                velocity: params.velocity,
                start_tick: onset,
                duration_ticks: dur,
            });
        }
    }
    out
}

pub fn derive_melody(
    chords: &[TimedChord],
    scale: Option<Scale>,
    params: &MelodyParams,
    ticks_per_beat: u32,
    seed: u64,
) -> Vec<GeneratedNote> {
    if chords.is_empty() {
        return Vec::new();
    }

    if params.style == MelodyStyle::Motif {
        return derive_motif_melody(chords, scale, params, ticks_per_beat, seed);
    }

    let tpb = ticks_per_beat as u64;
    let slot_ticks = params.note_value_ticks.max(1) as u64;
    let mut out = Vec::new();
    let mut rng = XorShift::new(seed);

    for tc in chords {
        let chord_start = tc.start_beat as u64 * tpb;
        let chord_len = (tc.duration_beats as u64).max(1) * tpb;
        let tones = chord_tones_in_register(tc.chord, params.register);
        if tones.is_empty() {
            continue;
        }

        let slots = (chord_len / slot_ticks).max(1) as usize;
        for slot in 0..slots {
            let rest_roll = rng.next_f32();
            if params.rest_density > 0.0 && rest_roll < params.rest_density {
                continue;
            }

            let note = match params.style {
                MelodyStyle::ArpUp => tones[slot % tones.len()],
                MelodyStyle::ArpDown => tones[tones.len() - 1 - (slot % tones.len())],
                MelodyStyle::ArpUpDown => {
                    let n = tones.len();
                    if n < 2 {
                        tones[0]
                    } else {
                        let cycle = 2 * n - 2;
                        let idx = slot % cycle;
                        if idx < n {
                            tones[idx]
                        } else {
                            tones[cycle - idx]
                        }
                    }
                }
                MelodyStyle::Motif => unreachable!(),
            };

            out.push(GeneratedNote {
                note,
                velocity: params.velocity,
                start_tick: chord_start + slot as u64 * slot_ticks,
                duration_ticks: slot_ticks,
            });
        }
    }
    out
}
