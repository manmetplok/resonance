//! Keyboard event handler for the timeline canvas.

use iced::keyboard;

use crate::message::*;
use super::super::TimelineCanvas;
use super::{captured, UpdateResult};

impl TimelineCanvas<'_> {
    pub(in crate::view::timeline) fn handle_key(
        &self,
        key: &keyboard::Key,
    ) -> UpdateResult {
        use keyboard::key::Named;
        // Esc abandons an in-flight drag-to-timeline placement (todo #605).
        if self.drag.is_some() && matches!(key, keyboard::Key::Named(Named::Escape)) {
            return captured(Message::Drag(DragMessage::Cancel));
        }
        let is_delete = matches!(
            key,
            keyboard::Key::Named(Named::Delete) | keyboard::Key::Named(Named::Backspace)
        );
        if !is_delete {
            return None;
        }
        // Delete selected global track event.
        if self.selected_global_event.is_some() {
            return captured(Message::GlobalTrack(
                GlobalTrackMessage::DeleteSelectedEvent,
            ));
        }
        if let Some(clip_id) = self.selected_midi_clip {
            return captured(Message::MidiClip(MidiClipMessage::DeleteMidiClip(clip_id)));
        }
        if let Some(clip_id) = self.selected_clip {
            return captured(Message::Clip(ClipMessage::DeleteClip(clip_id)));
        }
        None
    }
}
