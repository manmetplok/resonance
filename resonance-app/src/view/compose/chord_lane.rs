use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::Canvas;
use iced::{mouse, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use resonance_music_theory::{ChordQuality, PitchClass};

use crate::compose::{ComposeMessage, SectionDefinitionState};
use crate::message::Message;
use crate::theme;

pub const LANE_HEIGHT: f32 = 64.0;
const RULER_HEIGHT: f32 = 18.0;
const DEFAULT_NEW_CHORD_BEATS: u32 = 4;
/// Horizontal pixels from a chord block's right edge that count as the
/// resize handle. Anything inside that strip starts a resize drag; the rest
/// of the body starts a move drag.
const RESIZE_HANDLE_PX: f32 = 8.0;

pub fn view<'a>(
    definition: &'a SectionDefinitionState,
    time_sig_num: u8,
    selected_chord_id: Option<u64>,
) -> Element<'a, Message> {
    Canvas::new(ChordLaneCanvas {
        definition,
        time_sig_num,
        selected_chord_id,
    })
    .width(Length::Fill)
    .height(Length::Fixed(LANE_HEIGHT))
    .into()
}

pub struct ChordLaneCanvas<'a> {
    pub definition: &'a SectionDefinitionState,
    pub time_sig_num: u8,
    pub selected_chord_id: Option<u64>,
}

#[derive(Debug, Default)]
pub struct ChordLaneState {
    drag: Option<ChordDrag>,
}

#[derive(Debug, Clone, Copy)]
enum ChordDrag {
    /// Moving a chord: `grab_beat` is the beat offset inside the chord where
    /// the mouse grabbed it, so the chord sticks to the cursor naturally.
    Move {
        chord_id: u64,
        grab_beat: u32,
        pending_start_beat: u32,
    },
    /// Resizing a chord from its right edge.
    Resize {
        chord_id: u64,
        pending_duration_beats: u32,
    },
}

impl<'a> canvas::Program<Message> for ChordLaneCanvas<'a> {
    type State = ChordLaneState;

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());

        frame.fill_rectangle(Point::ORIGIN, bounds.size(), theme::PANEL_DARK);

        let total_beats = self.total_beats();
        if total_beats == 0 {
            return vec![frame.into_geometry()];
        }
        let beat_width = bounds.width / total_beats as f32;
        let bar_beats = self.time_sig_num as u32;

        // Ruler ticks + bar numbers
        for beat in 0..=total_beats {
            let x = beat as f32 * beat_width;
            let is_bar = beat % bar_beats == 0;
            let tick_color = if is_bar {
                theme::TEXT_DIM
            } else {
                theme::SEPARATOR
            };
            let tick_h = if is_bar { RULER_HEIGHT } else { RULER_HEIGHT * 0.5 };
            frame.stroke(
                &Path::line(Point::new(x, 0.0), Point::new(x, tick_h)),
                Stroke::default().with_width(1.0).with_color(tick_color),
            );
            if is_bar && beat < total_beats {
                let bar_num = (beat / bar_beats) + 1;
                frame.fill_text(canvas::Text {
                    content: format!("{}", bar_num),
                    position: Point::new(x + 3.0, 2.0),
                    color: theme::TEXT_DIM,
                    size: 10.0.into(),
                    ..canvas::Text::default()
                });
            }
        }

        // Separator between ruler and chord area
        frame.fill_rectangle(
            Point::new(0.0, RULER_HEIGHT),
            Size::new(bounds.width, 1.0),
            theme::SEPARATOR,
        );

        // Chord blocks (with drag preview overrides)
        let block_top = RULER_HEIGHT + 3.0;
        let block_h = bounds.height - block_top - 3.0;
        for chord in &self.definition.chords {
            let (start, dur) = apply_drag_preview(chord.id, chord.start_beat, chord.duration_beats, &state.drag);
            let x = start as f32 * beat_width;
            let w = (dur as f32 * beat_width - 1.0).max(2.0);
            let selected = Some(chord.id) == self.selected_chord_id;
            let dragging = matches!(state.drag, Some(ChordDrag::Move { chord_id, .. } | ChordDrag::Resize { chord_id, .. }) if chord_id == chord.id);

            let fill = if selected || dragging {
                theme::ACCENT
            } else {
                Color::from_rgb(0.24, 0.34, 0.48)
            };
            frame.fill_rectangle(Point::new(x + 0.5, block_top), Size::new(w, block_h), fill);

            if selected || dragging {
                frame.stroke(
                    &Path::rectangle(Point::new(x + 0.5, block_top), Size::new(w, block_h)),
                    Stroke::default().with_width(1.5).with_color(Color::WHITE),
                );
            }

            // Right-edge resize hint (tiny vertical bar)
            if w > RESIZE_HANDLE_PX * 2.0 {
                frame.fill_rectangle(
                    Point::new(x + w - 3.0, block_top + 2.0),
                    Size::new(2.0, block_h - 4.0),
                    Color::from_rgba(1.0, 1.0, 1.0, 0.35),
                );
            }

            frame.fill_text(canvas::Text {
                content: chord.chord.to_string(),
                position: Point::new(x + 6.0, block_top + 4.0),
                color: Color::WHITE,
                size: 12.0.into(),
                ..canvas::Text::default()
            });
        }

        vec![frame.into_geometry()]
    }

    fn update(
        &self,
        state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        let total_beats = self.total_beats();
        if total_beats == 0 || bounds.width <= 0.0 {
            return (canvas::event::Status::Ignored, None);
        }
        let beat_width = bounds.width / total_beats as f32;

        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(pos) = cursor.position_in(bounds) else {
                    return (canvas::event::Status::Ignored, None);
                };
                if pos.y < RULER_HEIGHT {
                    return (canvas::event::Status::Ignored, None);
                }
                let beat = (pos.x / beat_width) as u32;
                if beat >= total_beats {
                    return (canvas::event::Status::Ignored, None);
                }

                // Hit-test existing chords: right edge => resize, body => move.
                for chord in &self.definition.chords {
                    let end = chord.start_beat + chord.duration_beats;
                    if beat >= chord.start_beat && beat < end {
                        let chord_left_px = chord.start_beat as f32 * beat_width;
                        let chord_right_px = end as f32 * beat_width;
                        if chord_right_px - pos.x <= RESIZE_HANDLE_PX
                            && chord.duration_beats >= 1
                        {
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
                        let _ = chord_left_px;
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::Compose(ComposeMessage::SelectChord {
                                chord_id: chord.id,
                            })),
                        );
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
                    return (canvas::event::Status::Ignored, None);
                }
                (
                    canvas::event::Status::Captured,
                    Some(Message::Compose(ComposeMessage::AddChord {
                        definition_id: self.definition.id,
                        start_beat: beat,
                        duration_beats: duration,
                        root: PitchClass::C,
                        quality: ChordQuality::Maj,
                    })),
                )
            }

            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let Some(pos) = cursor.position_in(bounds) else {
                    return (canvas::event::Status::Ignored, None);
                };
                let Some(drag) = state.drag.as_mut() else {
                    return (canvas::event::Status::Ignored, None);
                };
                let beat_f = (pos.x / beat_width).max(0.0);
                let beat = (beat_f as u32).min(total_beats.saturating_sub(1));
                match drag {
                    ChordDrag::Move {
                        chord_id,
                        grab_beat,
                        pending_start_beat,
                    } => {
                        let chord = match self.definition.chords.iter().find(|c| c.id == *chord_id) {
                            Some(c) => c,
                            None => return (canvas::event::Status::Ignored, None),
                        };
                        let new_start = beat.saturating_sub(*grab_beat);
                        let max_start = total_beats.saturating_sub(chord.duration_beats);
                        *pending_start_beat = new_start.min(max_start);
                        (canvas::event::Status::Captured, None)
                    }
                    ChordDrag::Resize {
                        chord_id,
                        pending_duration_beats,
                    } => {
                        let chord = match self.definition.chords.iter().find(|c| c.id == *chord_id) {
                            Some(c) => c,
                            None => return (canvas::event::Status::Ignored, None),
                        };
                        let end_beat = ((pos.x / beat_width).ceil() as u32).min(total_beats);
                        let new_dur = end_beat.saturating_sub(chord.start_beat).max(1);
                        *pending_duration_beats = new_dur;
                        (canvas::event::Status::Captured, None)
                    }
                }
            }

            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
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
                            return (canvas::event::Status::Captured, None);
                        }
                        (
                            canvas::event::Status::Captured,
                            Some(Message::Compose(ComposeMessage::MoveChord {
                                definition_id: self.definition.id,
                                chord_id,
                                start_beat: pending_start_beat,
                            })),
                        )
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
                            return (canvas::event::Status::Captured, None);
                        }
                        (
                            canvas::event::Status::Captured,
                            Some(Message::Compose(ComposeMessage::ResizeChord {
                                definition_id: self.definition.id,
                                chord_id,
                                duration_beats: pending_duration_beats,
                            })),
                        )
                    }
                    None => (canvas::event::Status::Ignored, None),
                }
            }

            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                let Some(pos) = cursor.position_in(bounds) else {
                    return (canvas::event::Status::Ignored, None);
                };
                if pos.y < RULER_HEIGHT {
                    return (canvas::event::Status::Ignored, None);
                }
                let beat = (pos.x / beat_width) as u32;
                for chord in &self.definition.chords {
                    let end = chord.start_beat + chord.duration_beats;
                    if beat >= chord.start_beat && beat < end {
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::Compose(ComposeMessage::DeleteChord {
                                definition_id: self.definition.id,
                                chord_id: chord.id,
                            })),
                        );
                    }
                }
                (canvas::event::Status::Ignored, None)
            }

            _ => (canvas::event::Status::Ignored, None),
        }
    }
}

impl<'a> ChordLaneCanvas<'a> {
    fn total_beats(&self) -> u32 {
        self.definition.length_bars * self.time_sig_num as u32
    }
}

/// Applies the active drag's pending values for the given chord so the
/// draw pass can render the preview in place of the persisted state.
fn apply_drag_preview(
    chord_id: u64,
    start_beat: u32,
    duration_beats: u32,
    drag: &Option<ChordDrag>,
) -> (u32, u32) {
    match drag {
        Some(ChordDrag::Move {
            chord_id: id,
            pending_start_beat,
            ..
        }) if *id == chord_id => (*pending_start_beat, duration_beats),
        Some(ChordDrag::Resize {
            chord_id: id,
            pending_duration_beats,
        }) if *id == chord_id => (start_beat, *pending_duration_beats),
        _ => (start_beat, duration_beats),
    }
}
