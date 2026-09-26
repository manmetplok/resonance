//! Keyboard event handler for the timeline canvas.

use iced::keyboard;

use crate::message::*;
use super::super::{TimelineCanvas, TimelineState};
use super::{captured, UpdateResult};

impl TimelineCanvas<'_> {
    pub(in crate::view::timeline) fn handle_key(
        &self,
        state: &mut TimelineState,
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
        // Delete only acts when the timeline was the last surface pressed,
        // not while the user works in the piano roll or a text field.
        if !is_delete || !state.key_focus.owns_keys() {
            return None;
        }
        // Delete the breakpoint last clicked/dragged (most specific, and the
        // most recent explicit selection), before clip / global deletes.
        if let Some((target, index)) = state.selected_breakpoint.take() {
            return captured(Message::Automation(AutomationMessage::DeleteBreakpoint {
                target,
                index,
            }));
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
