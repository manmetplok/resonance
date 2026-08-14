//! Runtime edit/tool state for the vocal Expression dock (doc #154, todo
//! #336) plus the pure helpers its handlers lean on.
//!
//! The [`ExpressionCurves`](super::expression::ExpressionCurves) model
//! (todo #332) holds the *data* a user shapes; this module holds the
//! transient *tooling* around an edit session: which curve is active, the
//! pen mode, and whether breakpoint times snap to syllable/note onsets.
//! None of it persists with the project — it's UI state, the same as the
//! drumroll view state.
//!
//! The snap helpers are kept here (rather than inline in the update
//! handler) so they're unit-testable from `tests/` without constructing a
//! whole `Resonance`: the handler computes onsets from the lane's MIDI
//! clip and defers the maths to [`snap_time`].

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use resonance_audio::types::MidiNote;

use super::vocal_svs::CurveKind;

/// How the pen places points when the user draws on a curve canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PenMode {
    /// Freehand — drag lays down a dense trail of breakpoints.
    #[default]
    Draw,
    /// Click places a single breakpoint at a time.
    Points,
    /// Click sets line segment endpoints (straight ramps between points).
    Line,
}

/// Transient tool state for the Expression dock. Runtime-only — never
/// persisted with the project. Shared by whichever vocal lane is open in
/// the dock (one dock is visible at a time).
#[derive(Debug, Clone, PartialEq)]
pub struct ExpressionDockState {
    /// The curve currently shown in the canvas / inspector.
    pub active: CurveKind,
    /// Active pen mode for canvas edits.
    pub pen: PenMode,
    /// Whether breakpoint times snap to syllable/note onsets on placement.
    pub snap: bool,
}

impl Default for ExpressionDockState {
    fn default() -> Self {
        ExpressionDockState {
            active: CurveKind::Dynamics,
            pen: PenMode::default(),
            snap: false,
        }
    }
}

impl ExpressionDockState {
    /// Make `kind` the active curve shown in the dock.
    pub fn select_curve(&mut self, kind: CurveKind) {
        self.active = kind;
    }

    /// Switch the pen mode used for canvas edits.
    pub fn set_pen(&mut self, pen: PenMode) {
        self.pen = pen;
    }

    /// Toggle snap-to-syllables on or off.
    pub fn set_snap(&mut self, snap: bool) {
        self.snap = snap;
    }
}

/// Normalised note onsets (each in `[0, 1]`) usable as snap targets,
/// derived from a clip's `notes` and its `duration_ticks`. Sorted ascending
/// and de-duplicated; an empty / zero-length clip yields no targets.
pub fn normalized_onsets(notes: &[MidiNote], duration_ticks: u64) -> Vec<f32> {
    if duration_ticks == 0 {
        return Vec::new();
    }
    let dur = duration_ticks as f32;
    let mut onsets: Vec<f32> = notes
        .iter()
        .map(|n| (n.start_tick as f32 / dur).clamp(0.0, 1.0))
        .collect();
    onsets.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    onsets.dedup();
    onsets
}

/// Snap normalised time `t` to the nearest value in `onsets`. Returns `t`
/// (clamped to `[0, 1]`) unchanged when there are no onsets to snap to.
pub fn snap_time(t: f32, onsets: &[f32]) -> f32 {
    let t = t.clamp(0.0, 1.0);
    onsets
        .iter()
        .copied()
        .min_by(|a, b| {
            (a - t)
                .abs()
                .partial_cmp(&(b - t).abs())
                .unwrap_or(Ordering::Equal)
        })
        .unwrap_or(t)
}
