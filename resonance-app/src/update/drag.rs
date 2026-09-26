//! Update handlers for the drag-to-timeline placement gesture (doc #175,
//! todo #605).
//!
//! The pill / lit-lane / ghost-clip / tooltip a drag paints are pure
//! preview state, so [`Start`](DragMessage::Start),
//! [`Hover`](DragMessage::Hover) and [`Cancel`](DragMessage::Cancel) only
//! mutate the transient [`DragPlacement`] on `Resonance` and record no undo
//! (classified `UndoAction::Skip`).
//!
//! [`Drop`](DragMessage::Drop) is the one durable step. Rather than mutate
//! the project here, it re-dispatches a [`PoolMessage::ImportAndPlace`] for
//! the resolved target through the normal update pipeline — so the import +
//! placement is captured as exactly one undoable action by the pool arm of
//! the undo classifier, identical to a drop made through any other entry
//! point. A drop with no resolved target (the pointer never reached the
//! lanes) is a no-op that just clears the drag.

use iced::Task;
use resonance_audio::types::{SamplePos, TrackId};

use crate::message::{Message, PoolMessage};
use crate::state::{DragPlacement, DraggedAsset, DropResolution};
use crate::Resonance;

/// Drag-to-timeline placement gesture (doc #175, todo #605). The primary
/// way audio lands on the arrangement: a browser row is dragged over the
/// timeline, which previews a grid-snapped ghost clip, lights the target
/// lane, and — on release — drops the file as a clip.
///
/// Every variant is **transient** (classified `UndoAction::Skip`): the
/// pill / ghost / tooltip are pure preview state. Only the drop has a
/// durable effect, and it borrows the undoable
/// [`PoolMessage::ImportAndPlace`] path so the whole import + placement is
/// a single undo entry. Routed through `update::drag::handle`.
#[derive(Debug, Clone)]
pub enum DragMessage {
    /// Begin dragging `asset` (a browser row) onto the timeline. Records the
    /// in-flight drag so the timeline can start previewing it.
    Start(DraggedAsset),
    /// Pointer moved to `cursor` (timeline-canvas content coordinates) with
    /// a freshly resolved drop target. Published by the timeline canvas each
    /// move while a drag is active; updates the pill / ghost / tooltip.
    /// `resolved` is `None` when the cursor is off the lane area.
    Hover {
        cursor: iced::Point,
        resolved: Option<DropResolution>,
    },
    /// Release over the timeline: commit the current resolution. Reads the
    /// resolved [`DropTarget`], clears the drag, and re-dispatches a
    /// [`PoolMessage::ImportAndPlace`] so the placement is imported +
    /// undoable. A no-op if the drag never resolved a target.
    Drop,
    /// Abandon the drag (released off the timeline, or Esc). Clears the
    /// preview with no placement.
    Cancel,
}

impl DragMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::UndoAction;
        match self {
            // Drag-to-timeline placement preview (doc #175, todo #605) is pure
            // transient UI: the drag pill, lit lane, ghost clip and tooltip are
            // never undoable and never in the project file. The one durable
            // effect — the drop — re-dispatches a `Pool(ImportAndPlace)`, which
            // records its own single undo entry.
            Self::Start(..) | Self::Hover { .. } | Self::Drop | Self::Cancel => UndoAction::Skip,
        }
    }
}

/// Where a drop-import lands its clips (doc #175, ba todo #598). The
/// sample position is the **raw** drop position; the orchestration snaps
/// it to the timeline grid (the same snap the clip-drag handlers use)
/// before placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropTarget {
    /// Place each imported file on an existing track at `start_sample`.
    ExistingTrack {
        track_id: TrackId,
        start_sample: SamplePos,
    },
    /// Spawn a new audio track (the new-audio-track drop zone below the
    /// last lane) and place each imported file on it at `start_sample`.
    NewTrack { start_sample: SamplePos },
}

pub fn handle(app: &mut Resonance, message: DragMessage) -> Task<Message> {
    match message {
        DragMessage::Start(asset) => {
            app.drag_placement = Some(DragPlacement::new(asset));
            Task::none()
        }
        DragMessage::Hover { cursor, resolved } => {
            if let Some(drag) = app.drag_placement.as_mut() {
                drag.cursor = cursor;
                drag.resolved = resolved;
            }
            Task::none()
        }
        DragMessage::Cancel => {
            app.drag_placement = None;
            Task::none()
        }
        DragMessage::Drop => {
            // Take the drag out first so the preview clears regardless of
            // whether it resolved a target.
            let Some(drag) = app.drag_placement.take() else {
                return Task::none();
            };
            let Some(resolution) = drag.resolved else {
                // Released without ever landing over the lanes — nothing to
                // place.
                return Task::none();
            };
            // Re-enter the full update pipeline so the placement records its
            // own single undo entry (Pool => Record) exactly as a dialog /
            // OS-drop import would. Recursion is one level deep and returns
            // the orchestration's task (engine import command, etc.).
            app.update(Message::Pool(PoolMessage::ImportAndPlace {
                paths: vec![drag.asset.path],
                target: resolution.target,
            }))
        }
    }
}
