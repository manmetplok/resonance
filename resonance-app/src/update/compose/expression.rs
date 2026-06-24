//! Vocal Expression-dock edit handlers (doc #154, todo #336).
//!
//! Pure state transitions over the lane's
//! [`ExpressionCurves`](crate::compose::ExpressionCurves) and the
//! [`ExpressionDockState`](crate::compose::ExpressionDockState) — no
//! drawing. The breakpoint / depth / smoothing / reset arms request a
//! vocal re-render so the WAV reflects the edit; the tool-state arms
//! (select curve, pen mode, snap toggle) are view-only and don't.
//!
//! Snapping reuses the lane's derived MIDI clip: the clip's note onsets are
//! the syllable boundaries, normalised to the clip span and handed to
//! [`snap_time`].

use iced::Task;

use resonance_audio::types::TrackId;

use crate::compose::expression_edit::{normalized_onsets, snap_time};
use crate::compose::messages::ExpressionMessage;
use crate::message::Message;

pub(super) fn handle(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    msg: ExpressionMessage,
) -> Task<Message> {
    use ExpressionMessage::*;
    match msg {
        // ---- Tool state: view-only, no re-render ----
        SelectCurve(kind) => {
            r.compose.expression_dock.select_curve(kind);
            Task::none()
        }
        SetPenMode(pen) => {
            r.compose.expression_dock.set_pen(pen);
            Task::none()
        }
        SetSnap(on) => {
            r.compose.expression_dock.set_snap(on);
            Task::none()
        }

        // ---- Curve edits: mutate the model, then re-render the vocal ----
        AddBreakpoint { kind, t, value } => {
            let t = snapped_time(r, definition_id, track_id, t);
            r.compose
                .expression_curves_mut(definition_id, track_id)
                .curve_mut(kind)
                .add_breakpoint(t, value);
            rerender(r, definition_id, track_id)
        }
        MoveBreakpoint {
            kind,
            index,
            t,
            value,
        } => {
            let t = snapped_time(r, definition_id, track_id, t);
            r.compose
                .expression_curves_mut(definition_id, track_id)
                .curve_mut(kind)
                .move_breakpoint(index, t, value);
            rerender(r, definition_id, track_id)
        }
        RemoveBreakpoint { kind, index } => {
            r.compose
                .expression_curves_mut(definition_id, track_id)
                .curve_mut(kind)
                .remove_breakpoint(index);
            rerender(r, definition_id, track_id)
        }
        SetDepth { kind, depth } => {
            r.compose
                .expression_curves_mut(definition_id, track_id)
                .curve_mut(kind)
                .set_depth(depth);
            rerender(r, definition_id, track_id)
        }
        SetSmoothing { kind, smoothing } => {
            r.compose
                .expression_curves_mut(definition_id, track_id)
                .curve_mut(kind)
                .set_smoothing(smoothing);
            rerender(r, definition_id, track_id)
        }
        Reset { kind } => {
            r.compose
                .expression_curves_mut(definition_id, track_id)
                .reset(kind);
            rerender(r, definition_id, track_id)
        }
    }
}

/// Quantise `t` to the lane's note onsets when snap-to-syllables is on;
/// pass `t` through unchanged otherwise.
fn snapped_time(r: &crate::Resonance, definition_id: u64, track_id: TrackId, t: f32) -> f32 {
    if !r.compose.expression_dock.snap {
        return t;
    }
    snap_time(t, &lane_onsets(r, definition_id, track_id))
}

/// Normalised note onsets for the lane's derived MIDI clip — the snap
/// targets. Empty when the lane has no rendered clip yet.
fn lane_onsets(r: &crate::Resonance, definition_id: u64, track_id: TrackId) -> Vec<f32> {
    let clip_id = r
        .compose
        .derived_clips
        .iter()
        .find_map(|(&(def, _placement, track), &clip_id)| {
            (def == definition_id && track == track_id).then_some(clip_id)
        });
    let Some(clip_id) = clip_id else {
        return Vec::new();
    };
    let Some(clip) = r.midi_clips.iter().find(|c| c.id == clip_id) else {
        return Vec::new();
    };
    normalized_onsets(&clip.notes, clip.duration_ticks)
}

/// Request a vocal re-render so the freshly-edited curves reach the WAV.
fn rerender(r: &mut crate::Resonance, definition_id: u64, track_id: TrackId) -> Task<Message> {
    super::vocal_render::rerender_vocal_audio(r, definition_id, track_id)
}
