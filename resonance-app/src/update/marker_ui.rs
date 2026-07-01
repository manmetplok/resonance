//! Transient marker-interaction reducers (todo #369).
//!
//! These handle the view-only state the timeline ruler hit-testing drives:
//! marker selection highlighting, the right-click context menu, and the
//! inline rename field. None of them mutate the persisted marker set, so
//! they carry no undo weight (see the `Message::MarkerUi(_) => Skip`
//! classification in `undo.rs`). The one edit they produce — committing a
//! rename — is emitted as a follow-up [`MarkerMessage::Rename`], which the
//! classifier records as a normal undoable edit.

use iced::Task;

use crate::message::{MarkerUiMessage, Message};
use crate::state::{MarkerMenuState, MarkerRenameState};
use crate::Resonance;

pub fn handle(r: &mut Resonance, m: MarkerUiMessage) -> Task<Message> {
    match m {
        MarkerUiMessage::Select(id) => {
            r.interaction.selected_marker_id = id;
            // A fresh selection dismisses any open menu so a click on
            // another marker doesn't leave a stale menu floating.
            r.interaction.marker_menu = None;
        }
        MarkerUiMessage::OpenMenu { id, x, y } => {
            // Opening the menu also selects the marker so the ruler flag
            // renders with the accent while the menu is up.
            if r.markers.contains(id) {
                r.interaction.selected_marker_id = Some(id);
                r.interaction.marker_rename = None;
                r.interaction.marker_menu = Some(MarkerMenuState { marker_id: id, x, y });
            }
        }
        MarkerUiMessage::CloseMenu => {
            r.interaction.marker_menu = None;
        }
        MarkerUiMessage::BeginRename { id, x, y } => {
            if let Some(marker) = r.markers.get(id) {
                let text = marker.name.clone();
                r.interaction.selected_marker_id = Some(id);
                r.interaction.marker_menu = None;
                r.interaction.marker_rename = Some(MarkerRenameState {
                    marker_id: id,
                    text,
                    x,
                    y,
                });
            }
        }
        MarkerUiMessage::RenameChanged(text) => {
            if let Some(rename) = r.interaction.marker_rename.as_mut() {
                rename.text = text;
            }
        }
        MarkerUiMessage::CommitRename => {
            if let Some(rename) = r.interaction.marker_rename.take() {
                let name = rename.text.trim().to_string();
                // Empty names are meaningless for a flag label — drop the
                // edit rather than blank the marker out. The edit is applied
                // in-place here; the message is classified `Record` in
                // `undo.rs` so it lands on the undo stack just like a
                // `MarkerMessage::Rename`.
                if !name.is_empty() {
                    if let Some(marker) = r.markers.get_mut(rename.marker_id) {
                        marker.name = name;
                    }
                }
            }
        }
        MarkerUiMessage::CancelRename => {
            r.interaction.marker_rename = None;
        }
    }
    Task::none()
}
