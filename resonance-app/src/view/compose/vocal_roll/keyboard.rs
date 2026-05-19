//! Piano keyboard column on the left edge of the vocal roll. Paints the
//! black/white key rectangles and labels the C-row octaves. Click
//! handling lives in [`super::canvas_program`].

use iced::widget::canvas::{self, Frame};
use iced::{Point, Size};

use crate::theme;

use super::{is_black_key, note_name, VocalRollCanvas, VR_KEYBOARD_WIDTH};

impl VocalRollCanvas<'_> {
    pub(super) fn draw_keyboard(&self, frame: &mut Frame, grid_top: f32, grid_h: f32) {
        frame.fill_rectangle(
            Point::new(0.0, grid_top),
            Size::new(VR_KEYBOARD_WIDTH, grid_h),
            theme::BG_2,
        );
        let (lo, hi) = self.params.range;
        for note in lo..=hi {
            let Some(y_local) = self.note_to_y(note, grid_h) else {
                continue;
            };
            let y = grid_top + y_local;
            let h = self.zoom_y;
            if y + h < grid_top || y > grid_top + grid_h {
                continue;
            }
            let black = is_black_key(note);
            let key_color = if black { theme::BG_0 } else { theme::BG_3 };
            let key_w = if black {
                VR_KEYBOARD_WIDTH * 0.6
            } else {
                VR_KEYBOARD_WIDTH - 1.0
            };
            frame.fill_rectangle(
                Point::new(0.0, y),
                Size::new(key_w, h - 1.0),
                key_color,
            );
            // Label only on C rows when there's headroom.
            if note % 12 == 0 && h >= 8.0 {
                frame.fill_text(canvas::Text {
                    content: note_name(note),
                    position: Point::new(3.0, y + 1.0),
                    color: theme::TEXT_3,
                    size: (h * 0.7).min(10.0).into(),
                    font: theme::MONO_FONT,
                    ..canvas::Text::default()
                });
            }
        }
        // Right edge separator
        frame.fill_rectangle(
            Point::new(VR_KEYBOARD_WIDTH - 1.0, grid_top),
            Size::new(1.0, grid_h),
            theme::LINE_2,
        );
    }
}
