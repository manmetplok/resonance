//! [`canvas::Program`] impl for [`VocalRollCanvas`] — event dispatch
//! (mouse drags, keyboard shortcuts, scroll wheel) and the cached
//! per-frame draw entry point.

use iced::widget::canvas::{self, Frame, Geometry};
use iced::{mouse, Rectangle, Renderer, Theme};

use resonance_audio::types::TICKS_PER_QUARTER_NOTE;

use crate::message::{Message, MidiEditorMessage};

use super::{
    DragMode, VocalRollCanvas, VocalRollState, DEFAULT_VELOCITY, HEADER_TOTAL_HEIGHT,
    RESIZE_EDGE_PX, VR_KEYBOARD_WIDTH, VR_VELOCITY_LANE_HEIGHT,
};

impl canvas::Program<Message> for VocalRollCanvas<'_> {
    type State = VocalRollState;

    fn update(
        &self,
        state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        let grid_x = VR_KEYBOARD_WIDTH;
        let grid_top = HEADER_TOTAL_HEIGHT;
        let grid_h = bounds.height - HEADER_TOTAL_HEIGHT - VR_VELOCITY_LANE_HEIGHT;
        let grid_bottom = grid_top + grid_h;

        match event {
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if cursor.position_in(bounds).is_none() {
                    return (canvas::event::Status::Ignored, None);
                }
                match delta {
                    mouse::ScrollDelta::Lines { x, y } => {
                        if x.abs() > f32::EPSILON {
                            return (canvas::event::Status::Ignored, None);
                        }
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::MidiEditor(MidiEditorMessage::ScrollY(-y * 30.0))),
                        );
                    }
                    mouse::ScrollDelta::Pixels { x, y } => {
                        if x.abs() > f32::EPSILON {
                            return (canvas::event::Status::Ignored, None);
                        }
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::MidiEditor(MidiEditorMessage::ScrollY(-y))),
                        );
                    }
                }
            }

            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    // Piano keyboard preview — only in the grid band.
                    if pos.x < grid_x && pos.y >= grid_top && pos.y < grid_bottom {
                        if let Some(note) = self.y_to_note(pos.y - grid_top, grid_h) {
                            state.previewing_note = Some(note);
                            return (
                                canvas::event::Status::Captured,
                                Some(Message::MidiEditor(MidiEditorMessage::PreviewNote(
                                    self.track_id,
                                    note,
                                ))),
                            );
                        }
                    }

                    if pos.y < grid_top || pos.y >= grid_bottom {
                        // Velocity lane / chord strip / lyric strip are
                        // read-only for now — clicks just select the
                        // editor (no message).
                        return (canvas::event::Status::Ignored, None);
                    }

                    if pos.x >= grid_x {
                        let rel_x = pos.x - grid_x;
                        let rel_y = pos.y - grid_top;
                        let click_tick = self.x_to_tick(rel_x);
                        let Some(click_note) = self.y_to_note(rel_y, grid_h) else {
                            return (canvas::event::Status::Ignored, None);
                        };

                        for (i, n) in self.clip.notes.iter().enumerate() {
                            let nx = self.tick_to_x(n.start_tick);
                            let nw = self.duration_to_width(n.duration_ticks);
                            let Some(ny) = self.note_to_y(n.note, grid_h) else {
                                continue;
                            };
                            let nh = self.zoom_y;
                            if rel_x >= nx && rel_x <= nx + nw && rel_y >= ny && rel_y <= ny + nh
                            {
                                if (nx + nw) - rel_x < RESIZE_EDGE_PX {
                                    state.drag = Some(DragMode::ResizeNote {
                                        note_index: i,
                                        anchor_tick: n.start_tick,
                                    });
                                    return (
                                        canvas::event::Status::Captured,
                                        Some(Message::MidiEditor(MidiEditorMessage::SelectNote {
                                            note_index: Some(i),
                                        })),
                                    );
                                }
                                let tick_offset = n.start_tick as i64 - click_tick as i64;
                                state.drag = Some(DragMode::MoveNote {
                                    note_index: i,
                                    start_tick_offset: tick_offset,
                                });
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::MidiEditor(MidiEditorMessage::SelectNote {
                                        note_index: Some(i),
                                    })),
                                );
                            }
                        }

                        let snapped = self.snap(click_tick);
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::MidiEditor(MidiEditorMessage::AddNote {
                                clip_id: self.clip.id,
                                note: click_note,
                                start_tick: snapped,
                                duration_ticks: self.snap_ticks.max(TICKS_PER_QUARTER_NOTE / 4),
                                velocity: DEFAULT_VELOCITY,
                            })),
                        );
                    }
                }
            }

            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    if pos.x >= grid_x && pos.y >= grid_top && pos.y < grid_bottom {
                        let rel_x = pos.x - grid_x;
                        let rel_y = pos.y - grid_top;
                        for (i, n) in self.clip.notes.iter().enumerate() {
                            let nx = self.tick_to_x(n.start_tick);
                            let nw = self.duration_to_width(n.duration_ticks);
                            let Some(ny) = self.note_to_y(n.note, grid_h) else {
                                continue;
                            };
                            let nh = self.zoom_y;
                            if rel_x >= nx && rel_x <= nx + nw && rel_y >= ny && rel_y <= ny + nh
                            {
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::MidiEditor(MidiEditorMessage::RemoveNote {
                                        clip_id: self.clip.id,
                                        note_index: i,
                                    })),
                                );
                            }
                        }
                    }
                }
            }

            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    let rel_x = pos.x - grid_x;
                    let rel_y = pos.y - grid_top;
                    match &state.drag {
                        Some(DragMode::MoveNote {
                            note_index,
                            start_tick_offset,
                            ..
                        }) if pos.x >= grid_x && pos.y >= grid_top && pos.y < grid_bottom => {
                            let tick = self.x_to_tick(rel_x);
                            let raw_tick = (tick as i64 + start_tick_offset).max(0) as u64;
                            let snapped_tick = self.snap(raw_tick);
                            let Some(note) = self.y_to_note(rel_y, grid_h) else {
                                return (canvas::event::Status::Ignored, None);
                            };
                            return (
                                canvas::event::Status::Captured,
                                Some(Message::MidiEditor(MidiEditorMessage::MoveNote {
                                    clip_id: self.clip.id,
                                    note_index: *note_index,
                                    new_start_tick: snapped_tick,
                                    new_note: note,
                                })),
                            );
                        }
                        Some(DragMode::ResizeNote {
                            note_index,
                            anchor_tick,
                        }) if pos.x >= grid_x => {
                            let tick = self.x_to_tick(rel_x);
                            let snapped = self.snap(tick);
                            let new_dur =
                                snapped.saturating_sub(*anchor_tick).max(self.snap_ticks);
                            return (
                                canvas::event::Status::Captured,
                                Some(Message::MidiEditor(MidiEditorMessage::ResizeNote {
                                    clip_id: self.clip.id,
                                    note_index: *note_index,
                                    new_duration_ticks: new_dur,
                                })),
                            );
                        }
                        Some(_) | None => {}
                    }
                }
            }

            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.drag = None;
                if let Some(note) = state.previewing_note.take() {
                    return (
                        canvas::event::Status::Captured,
                        Some(Message::MidiEditor(MidiEditorMessage::StopPreview(
                            self.track_id,
                            note,
                        ))),
                    );
                }
            }

            canvas::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Delete),
                ..
            })
            | canvas::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Backspace),
                ..
            }) => {
                if let Some(idx) = self.selected_note {
                    if idx < self.clip.notes.len() {
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::MidiEditor(MidiEditorMessage::RemoveNote {
                                clip_id: self.clip.id,
                                note_index: idx,
                            })),
                        );
                    }
                }
            }

            // OpenUtau-style slur toggle. Pressing `s` (or `+`) on the
            // selected note flips its lyric between the slur marker
            // and the auto-syllabified surface form. Mirrors the
            // shortcut users coming from OpenUtau / Vocaloid editors
            // expect.
            canvas::Event::Keyboard(iced::keyboard::Event::KeyPressed { ref text, .. }) => {
                if let Some(idx) = self.selected_note {
                    if idx < self.clip.notes.len() {
                        if let Some(t) = text.as_deref() {
                            let key = t.trim();
                            if key.eq_ignore_ascii_case("s") || key == "+" {
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::MidiEditor(MidiEditorMessage::ToggleSlur {
                                        clip_id: self.clip.id,
                                        note_index: idx,
                                    })),
                                );
                            }
                        }
                    }
                }
            }

            _ => {}
        }
        (canvas::event::Status::Ignored, None)
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let fp = self.fingerprint(state);
        if state.cache_fingerprint.get() != fp {
            state.cache.clear();
            state.cache_fingerprint.set(fp);
        }
        let geo = state.cache.draw(renderer, bounds.size(), |frame: &mut Frame| {
            self.draw_into(frame, bounds);
        });
        vec![geo]
    }
}
