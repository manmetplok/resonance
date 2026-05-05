use serde::{Deserialize, Serialize};

use crate::rng::XorShift;
use crate::scale::Scale;

use super::motif_bass::chord_tones_in_register;
use super::motif_engine::derive_motif_melody;
use super::{GeneratedNote, TimedChord};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MelodyStyle {
    ArpUp,
    ArpDown,
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
        match self {
            MelodyStyle::ArpUp => "Arp up",
            MelodyStyle::ArpDown => "Arp down",
            MelodyStyle::ArpUpDown => "Arp up/down",
            MelodyStyle::Motif => "Motif",
        }
    }
}

impl std::fmt::Display for MelodyStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Preferred melodic contour shape for motif-based generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContourPreference {
    /// RNG picks per-phrase, weighted by research distributions.
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
        match self {
            ContourPreference::Auto => "Auto",
            ContourPreference::Arch => "Arch",
            ContourPreference::Descending => "Descending",
            ContourPreference::Ascending => "Ascending",
            ContourPreference::Wave => "Wave",
        }
    }
}

impl Default for ContourPreference {
    fn default() -> Self {
        Self::Auto
    }
}

impl std::fmt::Display for ContourPreference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
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
        }
    }
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
