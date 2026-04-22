use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::Canvas;
use iced::{mouse, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use resonance_audio::types::TempoMap;
use resonance_music_theory::{ChordQuality, PitchClass};

use crate::compose::{ComposeMessage, SelectedLane, SectionDefinitionState};
use crate::message::Message;
use crate::theme;

use super::tracks::NAME_COLUMN_WIDTH;

pub const LANE_HEIGHT: f32 = 64.0;
const RULER_HEIGHT: f32 = 18.0;
const DEFAULT_NEW_CHORD_BEATS: u32 = 4;
/// Horizontal pixels from a chord block's right edge that count as the
/// resize handle. Anything inside that strip starts a resize drag; the rest
/// of the body starts a move drag.
const RESIZE_HANDLE_PX: f32 = 8.0;

pub fn view<'a>(
    definition: &'a SectionDefinitionState,
    tempo_map: &'a TempoMap,
    start_bar: u32,
    selected_chord_id: Option<u64>,
    chords_selected: bool,
) -> Element<'a, Message> {
    Canvas::new(ChordLaneCanvas {
        definition,
        tempo_map,
        start_bar,
        selected_chord_id,
        chords_selected,
    })
    .width(Length::Fill)
    .height(Length::Fixed(LANE_HEIGHT))
    .into()
}

pub struct ChordLaneCanvas<'a> {
    pub definition: &'a SectionDefinitionState,
    pub tempo_map: &'a TempoMap,
    pub start_bar: u32,
    pub selected_chord_id: Option<u64>,
    pub chords_selected: bool,
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

        // ---- Name column (track header) ----
        let name_bg = if self.chords_selected {
            Color::from_rgb(0.22, 0.22, 0.27)
        } else {
            theme::PANEL
        };
        frame.fill_rectangle(
            Point::ORIGIN,
            Size::new(NAME_COLUMN_WIDTH, bounds.height),
            name_bg,
        );
        frame.fill_text(canvas::Text {
            content: "Chords".to_string(),
            position: Point::new(10.0, bounds.height * 0.5 - 7.0),
            color: if self.chords_selected {
                theme::ACCENT
            } else {
                theme::TEXT
            },
            size: 12.0.into(),
            ..canvas::Text::default()
        });
        // Divider between name column and grid
        frame.fill_rectangle(
            Point::new(NAME_COLUMN_WIDTH, 0.0),
            Size::new(1.0, bounds.height),
            if self.chords_selected {
                theme::ACCENT
            } else {
                theme::SEPARATOR
            },
        );

        // ---- Grid area (right of name column) ----
        let grid_x = NAME_COLUMN_WIDTH;
        let grid_w = (bounds.width - NAME_COLUMN_WIDTH).max(0.0);

        frame.fill_rectangle(
            Point::new(grid_x, 0.0),
            Size::new(grid_w, bounds.height),
            theme::PANEL_DARK,
        );

        let total_beats = self.total_beats();
        if total_beats == 0 || grid_w <= 0.0 {
            return vec![frame.into_geometry()];
        }
        let beat_width = grid_w / total_beats as f32;

        // Ruler ticks + bar numbers — walk bars for correct placement
        // with varying time signatures.
        let mut beat_pos: u32 = 0;
        for bar_offset in 0..self.definition.length_bars {
            let bar = self.start_bar + bar_offset;
            let num = self.tempo_map.numerator_at_bar(bar) as u32;

            // Bar line
            let x = grid_x + beat_pos as f32 * beat_width;
            frame.stroke(
                &Path::line(Point::new(x, 0.0), Point::new(x, RULER_HEIGHT)),
                Stroke::default().with_width(1.0).with_color(theme::TEXT_DIM),
            );
            // Bar number
            frame.fill_text(canvas::Text {
                content: format!("{}", bar_offset + 1),
                position: Point::new(x + 3.0, 2.0),
                color: theme::TEXT_DIM,
                size: 10.0.into(),
                ..canvas::Text::default()
            });

            // Beat ticks within this bar
            for beat in 1..num {
                let bx = grid_x + (beat_pos + beat) as f32 * beat_width;
                frame.stroke(
                    &Path::line(Point::new(bx, 0.0), Point::new(bx, RULER_HEIGHT * 0.5)),
                    Stroke::default()
                        .with_width(1.0)
                        .with_color(theme::SEPARATOR),
                );
            }

            beat_pos += num;
        }
        // Final bar line at section end
        let x = grid_x + beat_pos as f32 * beat_width;
        frame.stroke(
            &Path::line(Point::new(x, 0.0), Point::new(x, RULER_HEIGHT)),
            Stroke::default().with_width(1.0).with_color(theme::TEXT_DIM),
        );

        // Separator between ruler and chord area
        frame.fill_rectangle(
            Point::new(grid_x, RULER_HEIGHT),
            Size::new(grid_w, 1.0),
            theme::SEPARATOR,
        );

        // Chord blocks (with drag preview overrides)
        let block_top = RULER_HEIGHT + 3.0;
        let block_h = bounds.height - block_top - 3.0;
        for chord in &self.definition.chords {
            let (start, dur) = apply_drag_preview(
                chord.id,
                chord.start_beat,
                chord.duration_beats,
                &state.drag,
            );
            let x = grid_x + start as f32 * beat_width;
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

        // Bottom separator
        frame.fill_rectangle(
            Point::new(0.0, bounds.height - 1.0),
            Size::new(bounds.width, 1.0),
            theme::SEPARATOR,
        );

        vec![frame.into_geometry()]
    }

    fn update(
        &self,
        state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        let grid_x = NAME_COLUMN_WIDTH;
        let grid_w = (bounds.width - NAME_COLUMN_WIDTH).max(0.0);
        let total_beats = self.total_beats();
        if total_beats == 0 || grid_w <= 0.0 {
            return (canvas::event::Status::Ignored, None);
        }
        let beat_width = grid_w / total_beats as f32;

        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(pos) = cursor.position_in(bounds) else {
                    return (canvas::event::Status::Ignored, None);
                };

                // Click on the name column: select the chords lane.
                if pos.x < NAME_COLUMN_WIDTH {
                    return (
                        canvas::event::Status::Captured,
                        Some(Message::Compose(ComposeMessage::SelectLane(
                            SelectedLane::Chords,
                        ))),
                    );
                }

                if pos.y < RULER_HEIGHT {
                    return (canvas::event::Status::Ignored, None);
                }
                let rel_x = pos.x - grid_x;
                let beat = (rel_x / beat_width) as u32;
                if beat >= total_beats {
                    return (canvas::event::Status::Ignored, None);
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
                let rel_x = (pos.x - grid_x).max(0.0);
                let beat_f = (rel_x / beat_width).max(0.0);
                let beat = (beat_f as u32).min(total_beats.saturating_sub(1));
                match drag {
                    ChordDrag::Move {
                        chord_id,
                        grab_beat,
                        pending_start_beat,
                    } => {
                        let chord = match self.definition.chords.iter().find(|c| c.id == *chord_id)
                        {
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
                        let chord = match self.definition.chords.iter().find(|c| c.id == *chord_id)
                        {
                            Some(c) => c,
                            None => return (canvas::event::Status::Ignored, None),
                        };
                        let end_beat = ((rel_x / beat_width).ceil() as u32).min(total_beats);
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
                if pos.x < NAME_COLUMN_WIDTH || pos.y < RULER_HEIGHT {
                    return (canvas::event::Status::Ignored, None);
                }
                let rel_x = pos.x - grid_x;
                let beat = (rel_x / beat_width) as u32;
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
    /// Total beats in the section, summing per-bar numerators.
    fn total_beats(&self) -> u32 {
        (0..self.definition.length_bars)
            .map(|b| self.tempo_map.numerator_at_bar(self.start_bar + b) as u32)
            .sum()
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
