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

pub(super) type UpdateResult = Option<canvas::Action<Message>>;

pub(super) fn captured(msg: Message) -> UpdateResult {
    Some(canvas::Action::publish(msg).and_capture())
}
