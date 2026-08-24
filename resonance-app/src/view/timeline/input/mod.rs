//! Canvas event handling for the timeline. Split into focused submodules:
//!
//! - [`hit_test`]: geometry helpers and clip/marker hit-testing
//! - [`pointer`]: wheel / press (with helpers) / move / release handlers
//! - [`hover`]: hover cursor shape, right-press, and viewport reporting
//! - [`global_tracks`]: click handling for the global-tracks shelf
//! - [`keyboard`]: keyboard event handler

use iced::widget::canvas;

use crate::message::Message;

pub(in crate::view::timeline) mod hit_test;
mod global_tracks;
mod hover;
mod keyboard;
mod pointer;

/// Maximum interval between two clicks to count as a double-click.
pub(super) const DOUBLE_CLICK_MS: u128 = 400;

/// Which part of a clip is being dragged. The drag state itself lives in
/// the per-clip `*DragState` / `*TrimState` structs on `Resonance`; this
/// enum exists only to remember which of those is currently active so the
/// pointer-move and pointer-release handlers dispatch to the right end-drag
/// message.
#[derive(Debug, Clone)]
pub(crate) enum ClipInteraction {
    Move,
    Trim,
    /// Dragging an audio-clip fade handle (in or out) — horizontal drag.
    Fade,
    /// Dragging the audio-clip gain bead — vertical drag.
    Gain,
    MidiMove,
    MidiTrim,
}

/// Active drag on an arrangement marker: either the start pole (moves the
/// marker) or a region's end edge (resizes it). The target sample is
/// recomputed from the pointer x on every move, so only the marker id and
/// which handle is grabbed need to persist across the gesture.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MarkerDrag {
    pub id: u64,
    pub hit: super::hit_test::MarkerHit,
}

/// Active drag on a tempo event point.
#[derive(Debug)]
pub(crate) struct TempoDrag {
    /// Index into `tempo_events` at drag start.
    pub index: usize,
    /// Original BPM of the dragged event.
    pub original_bpm: f32,
    /// Mouse y at drag start.
    pub anchor_y: f32,
}

/// Active drag on an automation breakpoint (todo #382). Holds which lane
/// and which point (by time-sorted index) is moving. The index stays valid
/// for the whole gesture because `TimelineCanvas::breakpoint_drag_to`
/// clamps the point between its time-neighbors, so the lane never reorders
/// mid-drag.
#[derive(Debug, Clone)]
pub(crate) struct BreakpointDrag {
    pub target: resonance_common::AutomationTarget,
    pub index: usize,
}

/// An in-flight comping gesture on a take card (epic #15, todo #414).
///
/// One press starts it and the *release* decides what it was: a click
/// (under [`TAKE_DRAG_SLOP_PX`]) solos the take, anything wider promotes
/// the dragged range into the comp. That is why nothing is published on
/// press — the two verbs are indistinguishable until the pointer stops.
///
/// Only the gesture's own identity and pointer travel live here. The
/// slot, the take's audible extent and the row's screen geometry are all
/// re-resolved from the mirror and the layout on every frame rather than
/// captured at press time, so a scroll mid-drag moves the preview band
/// with its row instead of leaving it behind, and an engine echo arriving
/// mid-gesture cannot leave the preview describing a stale lane.
///
/// [`TAKE_DRAG_SLOP_PX`]: super::takes::input::TAKE_DRAG_SLOP_PX
#[derive(Debug, Clone, Copy)]
pub(crate) struct TakePromoteDrag {
    pub track_id: resonance_audio::types::TrackId,
    pub group_id: resonance_common::TakeGroupId,
    pub take_id: resonance_common::TakeId,
    pub anchor_x: f32,
    pub cursor_x: f32,
    /// Latest pointer y, for anchoring the caption only.
    pub cursor_y: f32,
}

impl TakePromoteDrag {
    /// Has the pointer travelled far enough for this gesture to be a
    /// promote rather than a click? Read on the *release* — that is the
    /// moment the two verbs stop being the same gesture.
    pub(crate) fn is_promote(&self) -> bool {
        (self.cursor_x - self.anchor_x).abs() >= super::takes::input::TAKE_DRAG_SLOP_PX
    }
}

pub(super) type UpdateResult = Option<canvas::Action<Message>>;

pub(super) fn captured(msg: Message) -> UpdateResult {
    Some(canvas::Action::publish(msg).and_capture())
}
