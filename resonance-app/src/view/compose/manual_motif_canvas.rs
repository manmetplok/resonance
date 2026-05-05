//! Inline canvas for editing the section's hand-drawn motif. Sits inside
//! the chord-lane inspector. Draws a small grid of scale-step rows by
//! sixteenth-note columns; click to add/remove notes, right-click to
//! toggle accent, scroll to cycle duration.
//!
//! The motif is anchored at scale step 0 (the section scale's tonic at
//! the chord root). Each motif consumer (melody, bass, drum) re-anchors
//! it onto its own register and chord at render time, so this canvas
//! represents the *shape* of the cell rather than absolute MIDI pitches.

use iced::widget::canvas;
use iced::{mouse, Color, Point, Rectangle, Renderer, Size, Theme};

use resonance_music_theory::{ManualMotifNote, Scale};

use crate::compose::messages::ChordInspectorMsg;
use crate::compose::ComposeMessage;
use crate::message::Message;
use crate::theme;

/// 16 sixteenth-cells = one bar of 4/4 — plenty of room for a 1- to 2-bar
/// motif. Cells past the last note are visible but empty (clicking one
/// appends a note there).
pub const GRID_COLS: u8 = 16;
/// Scale steps from -8 to +8 (17 rows). Covers one full octave above and
/// below the anchor — degree ±7 is the upper/lower tonic, ±8 lets motifs
/// reach just past it for an extra step of leading-tone headroom.
pub const GRID_ROWS: i8 = 17;
const ROW_CENTER: i8 = (GRID_ROWS - 1) / 2; // index 8 = scale_step 0
/// One extra row at the bottom of the canvas dedicated to rests. Clicking
/// it inserts/removes a rest at that beat using the same toggle semantics
/// as a pitched cell.
pub const REST_ROW_INDEX: i8 = GRID_ROWS;
/// Total rendered rows including the rest row.
pub const TOTAL_ROWS: i8 = GRID_ROWS + 1;
pub const CELL_W: f32 = 14.0;
pub const CELL_H: f32 = 12.0;

pub struct ManualMotifCanvas<'a> {
    pub definition_id: u64,
    pub notes: &'a [ManualMotifNote],
    pub scale: Option<Scale>,
}

impl<'a> canvas::Program<Message> for ManualMotifCanvas<'a> {
    type State = ();

    fn update(
        &self,
        _state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        let Some(pos) = cursor.position_in(bounds) else {
            return (canvas::event::Status::Ignored, None);
        };

        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                match self.cell_at(pos) {
                    Some(CellHit::Note { scale_step, beat_16 }) => (
                        canvas::event::Status::Captured,
                        Some(Message::Compose(ComposeMessage::ChordInspector {
                            definition_id: self.definition_id,
                            msg: ChordInspectorMsg::ToggleManualMotifCell {
                                scale_step,
                                beat_16,
                            },
                        })),
                    ),
                    Some(CellHit::Rest { beat_16 }) => (
                        canvas::event::Status::Captured,
                        Some(Message::Compose(ComposeMessage::ChordInspector {
                            definition_id: self.definition_id,
                            msg: ChordInspectorMsg::ToggleManualMotifRest { beat_16 },
                        })),
                    ),
                    None => (canvas::event::Status::Ignored, None),
                }
            }

            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                let beat_16 = match self.cell_at(pos) {
                    Some(CellHit::Note { beat_16, .. }) | Some(CellHit::Rest { beat_16 }) => beat_16,
                    None => return (canvas::event::Status::Ignored, None),
                };
                let Some(idx) = self.note_index_starting_at(beat_16) else {
                    return (canvas::event::Status::Ignored, None);
                };
                // Right-click on a rest is a no-op — rests have no accent.
                if self.notes.get(idx).is_some_and(|n| n.is_rest) {
                    return (canvas::event::Status::Captured, None);
                }
                (
                    canvas::event::Status::Captured,
                    Some(Message::Compose(ComposeMessage::ChordInspector {
                        definition_id: self.definition_id,
                        msg: ChordInspectorMsg::ToggleManualMotifAccent { index: idx },
                    })),
                )
            }

            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let dy = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => y,
                    mouse::ScrollDelta::Pixels { y, .. } => y,
                };
                if dy.abs() < f32::EPSILON {
                    return (canvas::event::Status::Ignored, None);
                }
                let beat_16 = match self.cell_at(pos) {
                    Some(CellHit::Note { beat_16, .. }) | Some(CellHit::Rest { beat_16 }) => beat_16,
                    None => return (canvas::event::Status::Ignored, None),
                };
                let Some(idx) = self.note_index_covering(beat_16) else {
                    return (canvas::event::Status::Ignored, None);
                };
                (
                    canvas::event::Status::Captured,
                    Some(Message::Compose(ComposeMessage::ChordInspector {
                        definition_id: self.definition_id,
                        msg: ChordInspectorMsg::CycleManualMotifNoteDuration { index: idx },
                    })),
                )
            }

            _ => (canvas::event::Status::Ignored, None),
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());

        frame.fill_rectangle(Point::ORIGIN, bounds.size(), theme::BG);

        // Pitched row backgrounds — alternate by chord-tone-ness vs
        // scale-step. Scale tonic (step 0) gets the accent tint; chord
        // tones (1, 3, 5, 7 of the scale) get a warmer tint.
        let chord_tone_steps: [i8; 4] = [0, 2, 4, 6];
        for r in 0..GRID_ROWS {
            let scale_step = (ROW_CENTER - r) as i8;
            let y = r as f32 * CELL_H;
            let normalized = scale_step.rem_euclid(7);
            let is_anchor = scale_step == 0;
            let is_chord_tone = chord_tone_steps.contains(&normalized);
            let row_bg = if is_anchor {
                tint(theme::ACCENT, 0.18)
            } else if is_chord_tone {
                tint(theme::PANEL, 1.10)
            } else {
                theme::PANEL
            };
            frame.fill_rectangle(
                Point::new(0.0, y),
                Size::new(bounds.width, CELL_H),
                row_bg,
            );
        }

        // Rest row at the bottom — visually distinct so the user can spot
        // the dedicated rest lane.
        let rest_y = REST_ROW_INDEX as f32 * CELL_H;
        frame.fill_rectangle(
            Point::new(0.0, rest_y),
            Size::new(bounds.width, CELL_H),
            tint(theme::PANEL, 0.7),
        );

        // Vertical grid lines: heavier on beat (every 4 sixteenths).
        for c in 0..=GRID_COLS {
            let x = c as f32 * CELL_W;
            let color = if c % 4 == 0 {
                theme::SEPARATOR
            } else {
                with_alpha(theme::SEPARATOR, 0.3)
            };
            frame.fill_rectangle(Point::new(x, 0.0), Size::new(1.0, bounds.height), color);
        }

        // Horizontal row separators (including the divider above the
        // rest row, which gets a slightly stronger line).
        for r in 0..=TOTAL_ROWS {
            let y = r as f32 * CELL_H;
            let color = if r == REST_ROW_INDEX {
                with_alpha(theme::SEPARATOR, 0.6)
            } else {
                with_alpha(theme::SEPARATOR, 0.25)
            };
            frame.fill_rectangle(Point::new(0.0, y), Size::new(bounds.width, 1.0), color);
        }

        // Notes and rests.
        let mut cursor_beat: u32 = 0;
        for n in self.notes {
            let dur = n.duration_sixteenths.max(1) as u32;
            if cursor_beat >= GRID_COLS as u32 {
                cursor_beat += dur;
                continue;
            }
            let x = cursor_beat as f32 * CELL_W;
            let visible_w =
                ((cursor_beat + dur).min(GRID_COLS as u32) - cursor_beat) as f32 * CELL_W;
            if n.is_rest {
                // Render rests as a dimmed horizontal block in the rest
                // row with diagonal hatching so they're obviously not pitches.
                let y = rest_y;
                frame.fill_rectangle(
                    Point::new(x + 1.0, y + 1.0),
                    Size::new(visible_w - 2.0, CELL_H - 2.0),
                    with_alpha(theme::TEXT_DIM, 0.6),
                );
                // Hatching lines.
                let hatch_step = 4.0;
                let mut hx = x + 2.0;
                while hx < x + visible_w {
                    frame.fill_rectangle(
                        Point::new(hx, y + 2.0),
                        Size::new(1.0, CELL_H - 4.0),
                        with_alpha(theme::PANEL, 0.7),
                    );
                    hx += hatch_step;
                }
            } else {
                let row = ROW_CENTER as i32 - n.scale_step as i32;
                if row >= 0 && row < GRID_ROWS as i32 {
                    let y = row as f32 * CELL_H;
                    let fill = if n.accent {
                        tint(theme::ACCENT, 1.10)
                    } else {
                        theme::ACCENT
                    };
                    frame.fill_rectangle(
                        Point::new(x + 1.0, y + 1.0),
                        Size::new(visible_w - 2.0, CELL_H - 2.0),
                        fill,
                    );
                    if n.accent {
                        // Small bar at the top to flag accented notes.
                        frame.fill_rectangle(
                            Point::new(x + 1.0, y + 1.0),
                            Size::new(visible_w - 2.0, 2.0),
                            theme::TEXT,
                        );
                    }
                }
            }
            cursor_beat += dur;
        }

        // Anchor row label hint — a tiny pip on the left of the anchor row.
        let anchor_y = ROW_CENTER as f32 * CELL_H;
        frame.fill_rectangle(
            Point::new(0.0, anchor_y + CELL_H / 2.0 - 1.0),
            Size::new(3.0, 2.0),
            theme::ACCENT,
        );

        let _ = self.scale; // reserved for future scale-aware row labeling
        vec![frame.into_geometry()]
    }
}

/// What was clicked in the canvas — either a pitched cell or the rest row.
enum CellHit {
    Note { scale_step: i8, beat_16: u8 },
    Rest { beat_16: u8 },
}

impl ManualMotifCanvas<'_> {
    fn cell_at(&self, pos: Point) -> Option<CellHit> {
        let col = (pos.x / CELL_W).floor() as i32;
        let row = (pos.y / CELL_H).floor() as i32;
        if col < 0 || col >= GRID_COLS as i32 || row < 0 || row >= TOTAL_ROWS as i32 {
            return None;
        }
        let beat_16 = col as u8;
        if row == REST_ROW_INDEX as i32 {
            return Some(CellHit::Rest { beat_16 });
        }
        let scale_step = (ROW_CENTER as i32 - row) as i8;
        Some(CellHit::Note { scale_step, beat_16 })
    }

    /// Index of the note that *starts* at the given beat, if any.
    fn note_index_starting_at(&self, beat_16: u8) -> Option<usize> {
        let mut cursor: u32 = 0;
        for (i, n) in self.notes.iter().enumerate() {
            if cursor as u8 == beat_16 {
                return Some(i);
            }
            cursor += n.duration_sixteenths.max(1) as u32;
        }
        None
    }

    /// Index of the note whose duration covers the given beat (inclusive
    /// of start, exclusive of start + duration).
    fn note_index_covering(&self, beat_16: u8) -> Option<usize> {
        let mut cursor: u32 = 0;
        for (i, n) in self.notes.iter().enumerate() {
            let dur = n.duration_sixteenths.max(1) as u32;
            if (cursor..cursor + dur).contains(&(beat_16 as u32)) {
                return Some(i);
            }
            cursor += dur;
        }
        None
    }
}

fn tint(c: Color, factor: f32) -> Color {
    Color {
        r: (c.r * factor).clamp(0.0, 1.0),
        g: (c.g * factor).clamp(0.0, 1.0),
        b: (c.b * factor).clamp(0.0, 1.0),
        a: c.a,
    }
}

fn with_alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}
