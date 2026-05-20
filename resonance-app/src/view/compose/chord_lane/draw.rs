//! Pure-draw helpers for the chord lane canvas. The big `draw_into`
//! method paints the lane side panel, ruler ticks, chord blocks (with
//! drag-preview overrides), and bottom separator. Drawing lives here so
//! `mod.rs` can stay focused on the canvas program glue and state.

use iced::widget::canvas::{self, Frame, Path, Stroke};
use iced::{Color, Point, Rectangle, Size};

use crate::theme;

use crate::view::compose::lane_side::{self, LaneKind};
use crate::view::compose::tracks::NAME_COLUMN_WIDTH;

use super::{apply_drag_preview, roman_numeral_for, ChordDrag, ChordLaneCanvas, RESIZE_HANDLE_PX, RULER_HEIGHT};

impl<'a> ChordLaneCanvas<'a> {
    pub(super) fn draw_into(&self, frame: &mut Frame, bounds: Rectangle, drag: &Option<ChordDrag>) {
        // ---- Lane side panel ----
        let chord_count = self.definition.chords.len();
        let scale_word = match (chord_count, self.definition.scale.as_ref()) {
            (n, Some(scale)) => format!("{} \u{00b7} {} chords", scale.root, n),
            (n, None) => format!("{} chords", n),
        };
        lane_side::draw(
            frame,
            Rectangle {
                x: 0.0,
                y: 0.0,
                width: NAME_COLUMN_WIDTH,
                height: bounds.height,
            },
            LaneKind::Harmony,
            "Chords",
            Some(&scale_word),
            self.chords_selected,
        );

        // ---- Grid area (right of name column) ----
        let grid_x = NAME_COLUMN_WIDTH;
        let grid_w = (bounds.width - NAME_COLUMN_WIDTH).max(0.0);

        frame.fill_rectangle(
            Point::new(grid_x, 0.0),
            Size::new(grid_w, bounds.height),
            theme::BG_1,
        );

        let total_beats = self.total_beats();
        if total_beats == 0 || grid_w <= 0.0 {
            return;
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

        // Chord blocks (with drag preview overrides). The redesign frames
        // each chord as a rounded card with a lavender wash + border;
        // selected/dragging cards get a stronger lavender border.
        let block_top = RULER_HEIGHT + 4.0;
        let block_h = bounds.height - block_top - 4.0;
        for chord in &self.definition.chords {
            let (start, dur) = apply_drag_preview(
                chord.id,
                chord.start_beat,
                chord.duration_beats,
                drag,
            );
            let x = grid_x + start as f32 * beat_width + 1.0;
            let w = (dur as f32 * beat_width - 2.0).max(2.0);
            let selected = Some(chord.id) == self.selected_chord_id;
            let dragging = matches!(drag, Some(ChordDrag::Move { chord_id, .. } | ChordDrag::Resize { chord_id, .. }) if *chord_id == chord.id);

            let fill = if selected || dragging {
                Color {
                    a: 0.22,
                    ..theme::ACCENT
                }
            } else {
                theme::BG_2
            };
            let border = if selected || dragging {
                theme::ACCENT
            } else {
                theme::LINE_2
            };
            let card = Path::rounded_rectangle(
                Point::new(x, block_top),
                Size::new(w, block_h),
                8.0.into(),
            );
            frame.fill(&card, fill);
            frame.stroke(
                &card,
                Stroke::default()
                    .with_width(if selected || dragging { 1.5 } else { 1.0 })
                    .with_color(border),
            );

            // Right-edge resize hint (tiny vertical bar)
            if w > RESIZE_HANDLE_PX * 2.0 {
                frame.fill_rectangle(
                    Point::new(x + w - 4.0, block_top + 6.0),
                    Size::new(2.0, block_h - 12.0),
                    Color {
                        a: 0.28,
                        ..theme::TEXT_2
                    },
                );
            }

            // Roman-numeral degree (small, mono, top-left). Computed
            // inline against the section's scale; "—" if no scale or the
            // chord root isn't on the scale.
            let degree = roman_numeral_for(&chord.chord, self.definition.scale.as_ref());
            frame.fill_text(canvas::Text {
                content: degree,
                position: Point::new(x + 8.0, block_top + 4.0),
                color: theme::TEXT_3,
                size: 9.0.into(),
                font: theme::MONO_FONT,
                ..canvas::Text::default()
            });

            // Chord symbol — italic serif, primary text color.
            frame.fill_text(canvas::Text {
                content: chord.chord.to_string(),
                position: Point::new(x + 8.0, block_top + 16.0),
                color: theme::TEXT_1,
                size: 18.0.into(),
                font: theme::SERIF_ITALIC_FONT,
                ..canvas::Text::default()
            });
        }

        // Bottom separator
        frame.fill_rectangle(
            Point::new(0.0, bounds.height - 1.0),
            Size::new(bounds.width, 1.0),
            theme::SEPARATOR,
        );
    }
}
