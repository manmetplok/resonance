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

use crate::message::{DragMessage, Message, PoolMessage};
use crate::state::DragPlacement;
use crate::Resonance;

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
