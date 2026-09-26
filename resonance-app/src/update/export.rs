//! Update handler for the Export modal shell (design doc #155).
//!
//! Owns the modal lifecycle — open, close, and the mode-tab switch — plus
//! the footer's primary action. The per-tab body controls and the render
//! orchestration live in follow-up todos (#326/#327 bodies, #330/#331
//! render); `Confirm` is wired but inert until then.

use iced::Task;

use crate::message::Message;
use crate::state::{ExportDialogState, ExportMode};
use crate::Resonance;

/// User actions in the Export modal (design doc #155). The scaffold wires
/// the shell lifecycle - open/close and the mode-tab switch - plus the
/// footer's primary action. The per-tab body controls (source checklist,
/// range/format, destination) emit their own messages added by the body
/// todos (#326/#327); `Confirm` kicks off the render in #330/#331.
#[derive(Debug, Clone)]
pub enum ExportMessage {
    /// Open the modal in its default (Audio-stems) state.
    Open,
    /// Dismiss the modal, discarding the transient selection.
    Close,
    /// Switch the active mode tab (Audio stems / MIDI).
    SetMode(ExportMode),
    /// Footer primary action - render the selected sources. Wired here so
    /// the shell is complete; the actual orchestration lands in #330/#331.
    Confirm,
}

impl ExportMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::UndoAction;
        match self {
            // The export dialog reads the project; it never edits it.
            Self::Open | Self::Close | Self::SetMode(..) | Self::Confirm => UndoAction::Skip,
        }
    }
}

pub fn handle(r: &mut Resonance, msg: ExportMessage) -> Task<Message> {
    match msg {
        ExportMessage::Open => {
            r.modals.export_dialog = Some(ExportDialogState::new());
            Task::none()
        }
        ExportMessage::Close => {
            r.modals.export_dialog = None;
            Task::none()
        }
        ExportMessage::SetMode(mode) => {
            if let Some(dialog) = r.modals.export_dialog.as_mut() {
                dialog.mode = mode;
            }
            Task::none()
        }
        ExportMessage::Confirm => {
            // Render orchestration lands in #330 (stems) / #331 (MIDI). The
            // shell only guards the action behind a non-empty selection so
            // the footer button is wired; kicking off the render is a
            // follow-up.
            Task::none()
        }
    }
}
