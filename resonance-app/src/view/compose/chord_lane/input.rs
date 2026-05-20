//! Pointer event handling for the chord lane canvas. Translates clicks
//! and drags into `ComposeMessage`s (add / select / move / resize / delete
//! chord). Drawing lives in [`super::draw`]; this file is the input
//! counterpart.

use iced::widget::canvas;
use iced::{mouse, Rectangle};

use resonance_music_theory::{ChordQuality, PitchClass};

use crate::compose::{ComposeMessage, SelectedLane};
use crate::message::Message;

use crate::view::compose::tracks::NAME_COLUMN_WIDTH;

use super::{ChordDrag, ChordLaneCanvas, ChordLaneState, DEFAULT_NEW_CHORD_BEATS, RESIZE_HANDLE_PX, RULER_HEIGHT};

impl<'a> ChordLaneCanvas<'a> {
    pub(super) fn update_inner(
        &self,
        state: &mut ChordLaneState,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let grid_x = NAME_COLUMN_WIDTH;
        let grid_w = (bounds.width - NAME_COLUMN_WIDTH).max(0.0);
        let total_beats = self.total_beats();
        if total_beats == 0 || grid_w <= 0.0 {
            return None;
        }
        let beat_width = grid_w / total_beats as f32;

        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let pos = cursor.position_in(bounds)?;

                // Click on the name column: select the chords lane.
                if pos.x < NAME_COLUMN_WIDTH {
                    return Some(canvas::Action::publish(Message::Compose(ComposeMessage::SelectLane(
                            SelectedLane::Chords,
                        ))).and_capture());
                }

                if pos.y < RULER_HEIGHT {
                    return None;
                }
                let rel_x = pos.x - grid_x;
                let beat = (rel_x / beat_width) as u32;
                if beat >= total_beats {
                    return None;
                }

                // Hit-test existing chords: right edge => resize, body => move.
                for chord in &self.definition.chords {
                    let end = chord.start_beat + chord.duration_beats;
                    if beat >= chord.start_beat && beat < end {
                        let chord_right_px = grid_x + end as f32 * beat_width;
                        if chord_right_px - pos.x <= RESIZE_HANDLE_PX && chord.duration_beats >= 1 {
                            state.drag = Some(ChordDrag::Resize {
                                chord_id: chord.id,
                                pending_duration_beats: chord.duration_beats,
                            });
                        } else {
                            let grab_beat = beat.saturating_sub(chord.start_beat);
                            state.drag = Some(ChordDrag::Move {
                                chord_id: chord.id,
                                grab_beat,
                                pending_start_beat: chord.start_beat,
                            });
                        }
                        return Some(canvas::Action::publish(Message::Compose(ComposeMessage::SelectChord {
                                chord_id: chord.id,
                            })).and_capture());
                    }
                }

                // Empty slot: add a default C-major chord covering up to
                // DEFAULT_NEW_CHORD_BEATS beats without overrunning the next
                // chord or the section end.
                let mut duration = DEFAULT_NEW_CHORD_BEATS.min(total_beats - beat);
                for chord in &self.definition.chords {
                    if chord.start_beat > beat {
                        let gap = chord.start_beat - beat;
                        if gap < duration {
                            duration = gap;
                        }
                        break;
                    }
                }
                if duration == 0 {
                    return None;
                }
                Some(canvas::Action::publish(Message::Compose(ComposeMessage::AddChord {
                        definition_id: self.definition.id,
                        start_beat: beat,
                        duration_beats: duration,
                        root: PitchClass::C,
                        quality: ChordQuality::Maj,
                    })).and_capture())
            }

            iced::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let pos = cursor.position_in(bounds)?;
                let drag = state.drag.as_mut()?;
                let rel_x = (pos.x - grid_x).max(0.0);
                let beat_f = (rel_x / beat_width).max(0.0);
                let beat = (beat_f as u32).min(total_beats.saturating_sub(1));
                match drag {
                    ChordDrag::Move {
                        chord_id,
                        grab_beat,
                        pending_start_beat,
                    } => {
                        let chord = self
                            .definition
                            .chords
                            .iter()
                            .find(|c| c.id == *chord_id)?;
                        let new_start = beat.saturating_sub(*grab_beat);
                        let max_start = total_beats.saturating_sub(chord.duration_beats);
                        *pending_start_beat = new_start.min(max_start);
                        Some(canvas::Action::capture())
                    }
                    ChordDrag::Resize {
                        chord_id,
                        pending_duration_beats,
                    } => {
                        let chord = self
                            .definition
                            .chords
                            .iter()
                            .find(|c| c.id == *chord_id)?;
                        let end_beat = ((rel_x / beat_width).ceil() as u32).min(total_beats);
                        let new_dur = end_beat.saturating_sub(chord.start_beat).max(1);
                        *pending_duration_beats = new_dur;
                        Some(canvas::Action::capture())
                    }
                }
            }

            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let drag = state.drag.take();
                match drag {
                    Some(ChordDrag::Move {
                        chord_id,
                        pending_start_beat,
                        ..
                    }) => {
                        let current = self
                            .definition
                            .chords
                            .iter()
                            .find(|c| c.id == chord_id)
                            .map(|c| c.start_beat);
                        if current == Some(pending_start_beat) {
                            return Some(canvas::Action::capture());
                        }
                        Some(canvas::Action::publish(Message::Compose(ComposeMessage::MoveChord {
                                definition_id: self.definition.id,
                                chord_id,
                                start_beat: pending_start_beat,
                            })).and_capture())
                    }
                    Some(ChordDrag::Resize {
                        chord_id,
                        pending_duration_beats,
                    }) => {
                        let current = self
                            .definition
                            .chords
                            .iter()
                            .find(|c| c.id == chord_id)
                            .map(|c| c.duration_beats);
                        if current == Some(pending_duration_beats) {
                            return Some(canvas::Action::capture());
                        }
                        Some(canvas::Action::publish(Message::Compose(ComposeMessage::ResizeChord {
                                definition_id: self.definition.id,
                                chord_id,
                                duration_beats: pending_duration_beats,
                            })).and_capture())
                    }
                    None => None,
                }
            }

            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                let pos = cursor.position_in(bounds)?;
                if pos.x < NAME_COLUMN_WIDTH || pos.y < RULER_HEIGHT {
                    return None;
                }
                let rel_x = pos.x - grid_x;
                let beat = (rel_x / beat_width) as u32;
                for chord in &self.definition.chords {
                    let end = chord.start_beat + chord.duration_beats;
                    if beat >= chord.start_beat && beat < end {
                        return Some(canvas::Action::publish(Message::Compose(ComposeMessage::DeleteChord {
                                definition_id: self.definition.id,
                                chord_id: chord.id,
                            })).and_capture());
                    }
                }
                None
            }

            _ => None,
        }
    }
}
