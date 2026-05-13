use serde::{Deserialize, Serialize};

use crate::scale::Scale;
use crate::voicing::{nearest_midi_above, nearest_midi_to};

use super::{GeneratedNote, TimedChord};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BassStyle {
    /// One note per chord, held for the chord's full duration.
    RootHold,
    /// Root on every beat of the chord.
    RootPulse,
    /// Root / fifth alternating on each beat.
    RootFifth,
    /// Root / octave alternating on each beat.
    Octave,
    /// Scale-stepping walking bass that approaches the next chord's root.
    /// Falls back to `RootPulse` when no scale is provided.
    Walking,
    /// Motif-based bass that consumes the section-shared motif. The exact
    /// rendering is controlled by `BassParams::motif_mode` and the per-phrase
    /// development by `BassParams::motif_phrase`.
    Motif,
}

impl BassStyle {
    pub const ALL: [BassStyle; 6] = [
        BassStyle::RootHold,
        BassStyle::RootPulse,
        BassStyle::RootFifth,
        BassStyle::Octave,
        BassStyle::Walking,
        BassStyle::Motif,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            BassStyle::RootHold => "Root hold",
            BassStyle::RootPulse => "Root pulse",
            BassStyle::RootFifth => "Root + fifth",
            BassStyle::Octave => "Octave",
            BassStyle::Walking => "Walking",
            BassStyle::Motif => "Motif",
        }
    }
}

impl std::fmt::Display for BassStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How a `BassStyle::Motif` lane renders the section-shared motif.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[derive(Default)]
pub enum BassMotifMode {
    /// Same intervals + rhythm, anchored to the chord's bass note in the bass register.
    #[default]
    SameIntervals,
    /// Same intervals + rhythm but each note's duration ratio doubled — sits under the melody.
    Augmented,
    /// Use only the motif's rhythm + accents; pitches collapse to the chord's bass note.
    RhythmOnly,
    /// Take only the motif's first note per chord, on the chord's bass note.
    FirstNoteOnly,
}

impl BassMotifMode {
    pub const ALL: [BassMotifMode; 4] = [
        BassMotifMode::SameIntervals,
        BassMotifMode::Augmented,
        BassMotifMode::RhythmOnly,
        BassMotifMode::FirstNoteOnly,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            BassMotifMode::SameIntervals => "Same intervals",
            BassMotifMode::Augmented => "Augmented",
            BassMotifMode::RhythmOnly => "Rhythm only",
            BassMotifMode::FirstNoteOnly => "First note only",
        }
    }
}

impl std::fmt::Display for BassMotifMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}


/// How a `BassStyle::Motif` lane chooses per-phrase transforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[derive(Default)]
pub enum BassMotifPhrase {
    /// `Identity` for every phrase — predictable foundation.
    #[default]
    Simple,
    /// Same `Transform` per phrase as the melody picks (shared seed → in lockstep).
    MirrorMelody,
    /// Restricted set: `Identity` or `Augment` only.
    Restricted,
}

impl BassMotifPhrase {
    pub const ALL: [BassMotifPhrase; 3] = [
        BassMotifPhrase::Simple,
        BassMotifPhrase::MirrorMelody,
        BassMotifPhrase::Restricted,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            BassMotifPhrase::Simple => "Simple",
            BassMotifPhrase::MirrorMelody => "Mirror melody",
            BassMotifPhrase::Restricted => "Restricted",
        }
    }
}

impl std::fmt::Display for BassMotifPhrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BassParams {
    pub style: BassStyle,
    /// MIDI floor for the bass root. Default E1 (28).
    pub base_note: u8,
    pub velocity: f32,
    /// How the motif is rendered when `style == Motif`.
    #[serde(default)]
    pub motif_mode: BassMotifMode,
    /// How per-phrase development is handled when `style == Motif`.
    #[serde(default)]
    pub motif_phrase: BassMotifPhrase,
}

impl Default for BassParams {
    fn default() -> Self {
        Self {
            style: BassStyle::RootPulse,
            base_note: 28, // E1
            velocity: 0.85,
            motif_mode: BassMotifMode::SameIntervals,
            motif_phrase: BassMotifPhrase::Simple,
        }
    }
}

pub fn derive_bass(
    chords: &[TimedChord],
    scale: Option<Scale>,
    params: &BassParams,
    ticks_per_beat: u32,
) -> Vec<GeneratedNote> {
    if chords.is_empty() {
        return Vec::new();
    }
    let tpb = ticks_per_beat as u64;
    let mut out = Vec::new();

    for (i, tc) in chords.iter().enumerate() {
        let root_pc = tc.chord.bass.unwrap_or(tc.chord.root);
        let root_midi = nearest_midi_above(root_pc, params.base_note);
        let start_tick = tc.start_beat as u64 * tpb;
        let beats = tc.duration_beats.max(1);

        match params.style {
            BassStyle::RootHold => {
                out.push(GeneratedNote {
                    note: root_midi,
                    velocity: params.velocity,
                    start_tick,
                    duration_ticks: beats as u64 * tpb,
                });
            }
            BassStyle::RootPulse => {
                for b in 0..beats {
                    out.push(GeneratedNote {
                        note: root_midi,
                        velocity: params.velocity,
                        start_tick: start_tick + b as u64 * tpb,
                        duration_ticks: tpb,
                    });
                }
            }
            BassStyle::RootFifth => {
                let fifth_pc = root_pc.transpose(7);
                let fifth_midi = nearest_midi_above(fifth_pc, root_midi);
                for b in 0..beats {
                    let note = if b % 2 == 0 { root_midi } else { fifth_midi };
                    out.push(GeneratedNote {
                        note,
                        velocity: params.velocity,
                        start_tick: start_tick + b as u64 * tpb,
                        duration_ticks: tpb,
                    });
                }
            }
            BassStyle::Octave => {
                let up = root_midi.checked_add(12).filter(|&n| n <= 127);
                for b in 0..beats {
                    let note = match up {
                        Some(up) if b % 2 != 0 => up,
                        _ => root_midi,
                    };
                    out.push(GeneratedNote {
                        note,
                        velocity: params.velocity,
                        start_tick: start_tick + b as u64 * tpb,
                        duration_ticks: tpb,
                    });
                }
            }
            BassStyle::Walking => {
                let next_root_midi = match (chords.get(i + 1), scale) {
                    (Some(nc), _) => {
                        let next_pc = nc.chord.bass.unwrap_or(nc.chord.root);
                        nearest_midi_to(next_pc, root_midi)
                    }
                    (None, _) => root_midi,
                };
                let line = walking_line(scale, root_midi, next_root_midi, beats as usize);
                for (b, note) in line.into_iter().enumerate() {
                    out.push(GeneratedNote {
                        note,
                        velocity: params.velocity,
                        start_tick: start_tick + b as u64 * tpb,
                        duration_ticks: tpb,
                    });
                }
            }
            BassStyle::Motif => {
                // Motif requires section-level MotifParams; without them
                // (the legacy `derive_bass` entry point), fall back to
                // RootHold so the lane still produces audible notes.
                out.push(GeneratedNote {
                    note: root_midi,
                    velocity: params.velocity,
                    start_tick,
                    duration_ticks: beats as u64 * tpb,
                });
            }
        }
    }
    out
}

/// Stepwise line from `root` toward `next_root` through scale tones.
/// When no scale is available, falls back to repeating the root.
fn walking_line(scale: Option<Scale>, root: u8, next_root: u8, beats: usize) -> Vec<u8> {
    if beats == 0 {
        return Vec::new();
    }
    let Some(scale) = scale else {
        return vec![root; beats];
    };
    if beats == 1 {
        return vec![root];
    }

    // The last beat is an approach tone — one scale step away from the
    // next chord's root, on the side we're coming from.
    let approach_dir: i32 = if next_root >= root { -1 } else { 1 };
    let approach = step_scale(&scale, next_root, approach_dir);

    if beats == 2 {
        return vec![root, approach];
    }

    // Interior beats: step from root toward the approach tone. Direction
    // is chosen by whichever end of the span the approach tone sits on.
    let up = approach > root;
    let interior_count = beats - 2;
    let mut notes = Vec::with_capacity(beats);
    notes.push(root);
    let mut cur = root;
    for _ in 0..interior_count {
        cur = step_scale(&scale, cur, if up { 1 } else { -1 });
        notes.push(cur);
    }
    notes.push(approach);
    notes
}

/// Next MIDI note in `dir` direction whose pitch class belongs to
/// `scale`. Searches up to one octave; returns `from` if no scale tone
/// is found (shouldn't happen for well-formed scales).
pub(super) fn step_scale(scale: &Scale, from: u8, dir: i32) -> u8 {
    let mut n = from as i32 + dir;
    for _ in 0..12 {
        if !(0..=127).contains(&n) {
            return from;
        }
        if scale.contains(n as u8) {
            return n as u8;
        }
        n += dir;
    }
    from
}
