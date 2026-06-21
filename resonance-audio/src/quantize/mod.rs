//! Pure MIDI quantize / humanize / groove algorithms.
//!
//! This module is deliberately free of any engine or app dependencies
//! beyond [`MidiNote`](crate::types::MidiNote) and
//! [`TempoMap`](crate::types::TempoMap): every function is pure and
//! deterministic, takes notes by slice and a selection by index, and
//! returns a fresh `Vec<MidiNote>` of the same length and order. Notes
//! are never reordered, merged, or dropped — only the selected indices
//! are modified.
//!
//! Grid geometry is derived purely from
//! [`TICKS_PER_QUARTER_NOTE`](crate::types::TICKS_PER_QUARTER_NOTE) and
//! the tempo map's signature track, so odd meters and mid-project time
//! signature changes are honoured without needing a sample rate.

mod grid;
mod groove;
mod humanize;
mod rng;

pub use grid::{BarRuler, Division, GridModifier, GridValue};
pub use groove::{apply_groove, extract_groove, stock_grooves, GrooveTemplate};
pub use humanize::humanize_notes;

use crate::types::{MidiNote, TempoMap};

use grid::snap_to_grid;

/// What a quantize pass adjusts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantizeMode {
    /// Move note starts only; durations are preserved.
    StartOnly,
    /// Move note starts and snap each note's length to the grid.
    StartAndLength,
}

/// Quantize the selected notes to `grid`.
///
/// * `selection` — indices into `notes` to quantize; out-of-range indices
///   are ignored and an empty selection is a no-op (every note is copied
///   through unchanged).
/// * `strength` — `0.0..=1.0` blend toward the grid (`1.0` snaps exactly).
/// * `swing` — `0.0..=1.0` swing applied to odd grid steps (see
///   [`grid`]); `2/3` yields a triplet feel.
/// * `mode` — [`QuantizeMode::StartAndLength`] also snaps each note's
///   length to a grid multiple; [`QuantizeMode::StartOnly`] leaves length
///   alone.
/// * `quantize_ends` — when `true`, the note-off is snapped to the grid
///   (the duration is recomputed from the quantized start and end). This
///   takes precedence over the length snapping implied by
///   `StartAndLength`.
/// * `iterative` — when `true`, the strength blend is applied repeatedly
///   so partial-strength quantization pulls notes closer to the grid than
///   a single pass would (no effect at `strength == 1.0`).
/// * `clip_start_tick` — the clip's absolute start tick on the project
///   timeline, so grid lines align to bars even when the clip does not
///   begin on a bar boundary.
#[allow(clippy::too_many_arguments)]
pub fn quantize_notes(
    notes: &[MidiNote],
    selection: &[usize],
    grid: Division,
    strength: f32,
    swing: f32,
    mode: QuantizeMode,
    quantize_ends: bool,
    iterative: bool,
    tempo: &TempoMap,
    clip_start_tick: u64,
) -> Vec<MidiNote> {
    let mut out = notes.to_vec();
    let strength = strength.clamp(0.0, 1.0);
    if strength == 0.0 {
        return out;
    }
    let g = grid.ticks();
    let ruler = BarRuler::new(tempo);
    // More passes converge geometrically on the grid for strength < 1.
    let passes = if iterative { 8 } else { 1 };

    for &i in selection {
        let Some(n) = out.get_mut(i) else { continue };
        let start_abs = clip_start_tick + n.start_tick;
        let end_abs = start_abs + n.duration_ticks;

        let new_start_abs = blend_to_grid(start_abs, &ruler, g, swing, strength, passes);

        if quantize_ends {
            let new_end_abs = blend_to_grid(end_abs, &ruler, g, swing, strength, passes);
            let dur = new_end_abs.saturating_sub(new_start_abs).max(1);
            n.duration_ticks = dur;
        } else if mode == QuantizeMode::StartAndLength {
            n.duration_ticks = quantize_length(n.duration_ticks, g, strength);
        }

        n.start_tick = new_start_abs.saturating_sub(clip_start_tick);
    }
    out
}

/// Blend `abs_tick` toward its snapped grid line by `strength`, repeated
/// `passes` times.
fn blend_to_grid(
    abs_tick: u64,
    ruler: &BarRuler,
    g: u64,
    swing: f32,
    strength: f32,
    passes: u32,
) -> u64 {
    let mut pos = abs_tick;
    for _ in 0..passes {
        let target = snap_to_grid(pos, ruler, g, swing);
        let delta = target as i64 - pos as i64;
        let moved = (delta as f64 * strength as f64).round() as i64;
        pos = (pos as i64 + moved).max(0) as u64;
        if moved == 0 {
            break;
        }
    }
    pos
}

/// Snap a duration to the nearest grid multiple by `strength`, never
/// shrinking below one grid step.
fn quantize_length(dur: u64, g: u64, strength: f32) -> u64 {
    let steps = (dur as f64 / g as f64).round().max(1.0);
    let target = steps as u64 * g;
    let delta = target as i64 - dur as i64;
    let moved = (delta as f64 * strength as f64).round() as i64;
    (dur as i64 + moved).max(1) as u64
}
