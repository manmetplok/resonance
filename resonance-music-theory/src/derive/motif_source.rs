use serde::{Deserialize, Serialize};

use crate::scale::Scale;

use super::motif_engine::MotifNote;

/// Section-level motif knobs shared across all motif-style lanes.
///
/// Both the melody motif renderer and the bass motif renderer consume
/// these so that, when both lanes' styles are `Motif`, they share the
/// same underlying motif identity (intervals + rhythm + accents). Only
/// register / velocity / contour / articulation / phrase length differ
/// per lane.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MotifParams {
    pub seed: u64,
    /// 0.0 = simple, 1.0 = maximum development. Drives motif length,
    /// rhythm pattern, and per-phrase transform variety.
    pub complexity: f32,
    /// Motif length override (0 = auto from complexity, else clamped to 2..=6).
    pub motif_len: u8,
    /// Probability of a leap vs step when generating motif intervals.
    pub leap_chance: f32,
}

impl Default for MotifParams {
    fn default() -> Self {
        Self {
            seed: 0,
            complexity: 0.5,
            motif_len: 0,
            leap_chance: 0.21,
        }
    }
}

/// One note in a hand-drawn motif. Stored as a signed scale-degree offset
/// from the motif anchor (0 = anchor, +7 = octave up, −7 = octave down)
/// plus a duration in sixteenth-notes and an accent flag. When `is_rest`
/// is true the entry is a rest: `scale_step` and `accent` are ignored
/// during rendering, but the duration still advances the per-chord cursor
/// so the rest takes up time. Mapped to a semitone interval at render
/// time using the section's `Scale`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualMotifNote {
    pub scale_step: i8,
    pub duration_sixteenths: u8,
    pub accent: bool,
    /// When true this entry is a rest: the cursor advances by
    /// `duration_sixteenths` but no MIDI note is emitted.
    #[serde(default)]
    pub is_rest: bool,
}

impl Default for ManualMotifNote {
    fn default() -> Self {
        Self {
            scale_step: 0,
            duration_sixteenths: 2, // an eighth note
            accent: false,
            is_rest: false,
        }
    }
}

/// Where a section's motif comes from. Either generated procedurally from
/// `MotifParams` (the historical default) or hand-drawn by the user.
///
/// In `Manual` mode, the embedded `params` is preserved so flipping back
/// to `Generated` restores the prior knob settings, and `params.seed` /
/// `params.complexity` still drive the per-phrase transform plan so a
/// hand-drawn motif also develops across phrases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MotifSource {
    Generated(MotifParams),
    Manual {
        notes: Vec<ManualMotifNote>,
        params: MotifParams,
    },
}

impl Default for MotifSource {
    fn default() -> Self {
        MotifSource::Generated(MotifParams::default())
    }
}

impl MotifSource {
    pub fn params(&self) -> &MotifParams {
        match self {
            MotifSource::Generated(p) | MotifSource::Manual { params: p, .. } => p,
        }
    }

    pub fn params_mut(&mut self) -> &mut MotifParams {
        match self {
            MotifSource::Generated(p) | MotifSource::Manual { params: p, .. } => p,
        }
    }

    pub fn is_manual(&self) -> bool {
        matches!(self, MotifSource::Manual { .. })
    }

    /// Mutable access to the manual notes vector. Returns `None` for a
    /// `Generated` motif. The chord-inspector update layer uses this to
    /// edit the canvas without having to repeatedly destructure
    /// `MotifSource::Manual` at every call site.
    pub fn manual_notes_mut(&mut self) -> Option<&mut Vec<ManualMotifNote>> {
        match self {
            MotifSource::Manual { notes, .. } => Some(notes),
            MotifSource::Generated(_) => None,
        }
    }

    /// Default note sequence when a section is first switched to Manual
    /// mode: a short scale-step ascent so the canvas produces audible
    /// output on first switch instead of an empty grid.
    pub fn default_manual_notes() -> Vec<ManualMotifNote> {
        vec![
            ManualMotifNote { scale_step: 0, duration_sixteenths: 2, accent: true,  is_rest: false },
            ManualMotifNote { scale_step: 2, duration_sixteenths: 2, accent: false, is_rest: false },
            ManualMotifNote { scale_step: 4, duration_sixteenths: 2, accent: false, is_rest: false },
            ManualMotifNote { scale_step: 2, duration_sixteenths: 2, accent: false, is_rest: false },
        ]
    }
}

impl From<MotifParams> for MotifSource {
    fn from(p: MotifParams) -> Self {
        MotifSource::Generated(p)
    }
}

/// What the user clicked on the manual-motif canvas: a pitched cell or
/// the dedicated rest row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualMotifCell {
    Note { scale_step: i8 },
    Rest,
}

/// Toggle a manual-motif cell. The motif is an ordered sequence of notes
/// (and rests) laid out left-to-right; `beat_16` is the start beat (in
/// sixteenths) of the click target.
///
/// - Empty cell past the end: append a 1-sixteenth entry of the requested
///   kind.
/// - Click at the start of an entry that matches the requested kind:
///   remove it.
/// - Click at the start of an entry of the *other* kind, or inside an
///   entry's tail: convert it to the requested kind. For Note → Rest the
///   accent is dropped.
pub fn toggle_manual_motif_cell(
    notes: &mut Vec<ManualMotifNote>,
    cell: ManualMotifCell,
    beat_16: u8,
) {
    let target = beat_16 as u32;
    let mut cursor: u32 = 0;
    for i in 0..notes.len() {
        let dur = notes[i].duration_sixteenths.max(1) as u32;
        if cursor == target {
            if entry_matches_cell(&notes[i], cell) {
                notes.remove(i);
            } else {
                apply_cell(&mut notes[i], cell);
            }
            return;
        }
        if cursor < target && cursor + dur > target {
            apply_cell(&mut notes[i], cell);
            return;
        }
        cursor += dur;
    }
    notes.push(ManualMotifNote {
        scale_step: match cell {
            ManualMotifCell::Note { scale_step } => scale_step,
            ManualMotifCell::Rest => 0,
        },
        duration_sixteenths: 1,
        accent: false,
        is_rest: matches!(cell, ManualMotifCell::Rest),
    });
}

fn entry_matches_cell(entry: &ManualMotifNote, cell: ManualMotifCell) -> bool {
    match cell {
        ManualMotifCell::Note { scale_step } => !entry.is_rest && entry.scale_step == scale_step,
        ManualMotifCell::Rest => entry.is_rest,
    }
}

fn apply_cell(entry: &mut ManualMotifNote, cell: ManualMotifCell) {
    match cell {
        ManualMotifCell::Note { scale_step } => {
            entry.scale_step = scale_step;
            entry.is_rest = false;
        }
        ManualMotifCell::Rest => {
            entry.is_rest = true;
            entry.accent = false;
        }
    }
}

/// Convert a signed scale-step offset to a semitone interval relative to
/// the motif anchor, using the section's scale for the diatonic mapping.
/// When no scale is provided, scale steps are interpreted chromatically
/// (step n = n semitones) so manual motifs still produce audible output.
pub(super) fn scale_step_to_semitones(step: i8, scale: Option<Scale>) -> i8 {
    let Some(scale) = scale else {
        return step;
    };
    let intervals = scale.mode.intervals();
    if intervals.is_empty() {
        return step;
    }
    let len = intervals.len() as i8;
    let octaves = step.div_euclid(len);
    let degree = step.rem_euclid(len) as usize;
    octaves * 12 + intervals[degree] as i8
}

/// Materialize a hand-drawn motif into the engine's internal `MotifNote`
/// shape. The resulting cell feeds straight into `transform_motif` and
/// the per-chord render path so manual motifs reuse the entire phrase
/// development pipeline.
pub(super) fn manual_motif_to_motif_notes(
    notes: &[ManualMotifNote],
    scale: Option<Scale>,
) -> Vec<MotifNote> {
    notes
        .iter()
        .map(|m| MotifNote {
            interval: scale_step_to_semitones(m.scale_step, scale),
            duration_ratio: m.duration_sixteenths.max(1),
            accent: m.accent && !m.is_rest,
            silent: m.is_rest,
        })
        .collect()
}
