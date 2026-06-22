//! Update handler for the Export modal shell (design doc #155).
//!
//! Owns the modal lifecycle — open, close, and the mode-tab switch — plus
//! the footer's primary action. The per-tab body controls and the render
//! orchestration live in follow-up todos (#326/#327 bodies, #330/#331
//! render); `Confirm` is wired but inert until then.

use iced::Task;

use crate::message::{ExportMessage, Message};
use crate::state::ExportDialogState;
use crate::Resonance;

pub fn handle(r: &mut Resonance, msg: ExportMessage) -> Task<Message> {
    match msg {
        ExportMessage::Open => {
            r.export_dialog = Some(ExportDialogState::new());
            Task::none()
        }
        ExportMessage::Close => {
            r.export_dialog = None;
            Task::none()
        }
        ExportMessage::SetMode(mode) => {
            if let Some(dialog) = r.export_dialog.as_mut() {
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
