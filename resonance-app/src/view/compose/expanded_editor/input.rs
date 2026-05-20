//! Interaction helpers for the expanded editor canvas. These methods
//! translate pointer events into note edits — adding notes, hit-testing
//! existing notes for move / resize drags, and the right-click delete.
//! Drawing lives in [`super::draw`]; this file is the input counterpart.

use iced::widget::canvas;
use iced::Point;

use crate::message::*;
use crate::piano_roll::{hit_test_note, snap_tick, NoteEdge, PianoRollLayout, PianoRollViewport};

use super::{
    DragMode, ExpandedEditorCanvas, ExpandedEditorState, DEFAULT_VELOCITY, SNAP_TICKS,
    TOOLBAR_HEIGHT,
};

// ---------------------------------------------------------------------------
// Interaction helpers
// ---------------------------------------------------------------------------

impl<'a> ExpandedEditorCanvas<'a> {
    pub(super) fn handle_grid_click(
        &self,
        state: &mut ExpandedEditorState,
        layout: &PianoRollLayout,
        viewport: &PianoRollViewport,
        pos: Point,
        gy: f32,
    ) -> Option<canvas::Action<Message>> {
        let grid_x = layout.grid_x();
        let click_note = viewport.y_local_to_note(gy);

        // Check existing notes for move / resize
        for clip in self
            .midi_clips
            .iter()
            .filter(|c| c.track_id == self.track_id)
        {
            if !self.clip_intersects_section(clip) {
                continue;
            }
            for (i, n) in clip.notes.iter().enumerate() {
                let rect = self.note_rect(layout, viewport, n);
                if let Some(edge) = hit_test_note(rect, pos) {
                    state.drag = Some(match edge {
                        NoteEdge::ResizeRight => DragMode::ResizeNote {
                            note_index: i,
                            anchor_tick: n.start_tick,
                            clip_id: clip.id,
                        },
                        NoteEdge::Body => {
                            let click_tick = viewport.x_local_to_tick(pos.x - grid_x);
                            let tick_offset = n.start_tick as i64 - click_tick as i64;
                            DragMode::MoveNote {
                                note_index: i,
                                start_tick_offset: tick_offset,
                                clip_id: clip.id,
                            }
                        }
                    });
                    return Some(canvas::Action::capture());
                }
            }
        }

        // Empty space: add note
        let click_tick = viewport.x_local_to_tick(pos.x - grid_x);
        let snapped = snap_tick(click_tick, SNAP_TICKS);

        for clip in self
            .midi_clips
            .iter()
            .filter(|c| c.track_id == self.track_id)
        {
            let clip_end = self.midi_clip_end_sample(clip);
            if self.section_start >= clip.start_sample && self.section_start < clip_end {
                return Some(canvas::Action::publish(Message::MidiEditor(MidiEditorMessage::AddNote {
                        clip_id: clip.id,
                        note: click_note,
                        start_tick: snapped,
                        duration_ticks: SNAP_TICKS,
                        velocity: DEFAULT_VELOCITY,
                    })).and_capture());
            }
        }

        Some(canvas::Action::capture())
    }

    pub(super) fn handle_right_click(
        &self,
        layout: &PianoRollLayout,
        viewport: &PianoRollViewport,
        pos: Point,
    ) -> Option<canvas::Action<Message>> {
        for clip in self
            .midi_clips
            .iter()
            .filter(|c| c.track_id == self.track_id)
        {
            if !self.clip_intersects_section(clip) {
                continue;
            }
            for (i, n) in clip.notes.iter().enumerate() {
                let rect = self.note_rect(layout, viewport, n);
                if hit_test_note(rect, pos).is_some() {
                    return Some(canvas::Action::publish(Message::MidiEditor(MidiEditorMessage::RemoveNote {
                            clip_id: clip.id,
                            note_index: i,
                        })).and_capture());
                }
            }
        }
        None
    }

    pub(super) fn handle_drag(
        &self,
        state: &mut ExpandedEditorState,
        viewport: &PianoRollViewport,
        pos: Point,
        grid_x: f32,
    ) -> Option<Message> {
        match &state.drag {
            Some(DragMode::MoveNote {
                note_index,
                start_tick_offset,
                clip_id,
                ..
            }) => {
                if pos.x >= grid_x && pos.y > TOOLBAR_HEIGHT {
                    let gy = pos.y - TOOLBAR_HEIGHT;
                    let tick = viewport.x_local_to_tick(pos.x - grid_x);
                    let raw = (tick as i64 + start_tick_offset).max(0) as u64;
                    let snapped = snap_tick(raw, SNAP_TICKS);
                    let note = viewport.y_local_to_note(gy);
                    return Some(Message::MidiEditor(MidiEditorMessage::MoveNote {
                        clip_id: *clip_id,
                        note_index: *note_index,
                        new_start_tick: snapped,
                        new_note: note,
                    }));
                }
                None
            }
            Some(DragMode::ResizeNote {
                note_index,
                anchor_tick,
                clip_id,
            }) => {
                if pos.x >= grid_x {
                    let tick = viewport.x_local_to_tick(pos.x - grid_x);
                    let snapped = snap_tick(tick, SNAP_TICKS);
                    let new_dur = snapped.saturating_sub(*anchor_tick).max(SNAP_TICKS);
                    return Some(Message::MidiEditor(MidiEditorMessage::ResizeNote {
                        clip_id: *clip_id,
                        note_index: *note_index,
                        new_duration_ticks: new_dur,
                    }));
                }
                None
            }
            None => None,
        }
    }
}
