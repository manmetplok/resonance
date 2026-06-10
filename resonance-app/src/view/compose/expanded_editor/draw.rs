//! Pure-draw helpers for the expanded editor canvas. These methods take a
//! `&mut Frame` and render note-row backgrounds, the beat grid, and the
//! note rectangles. They're in a separate file so `mod.rs` can stay
//! focused on canvas event handling and state.

use iced::widget::canvas::{self, Frame};
use iced::{Color, Point, Size};

use resonance_audio::types::TICKS_PER_QUARTER_NOTE;

use crate::view::piano_roll::{
    self, is_black_key, note_name, NoteStyle, PianoRollLayout, PianoRollViewport, NOTE_COUNT,
};
use crate::theme;

use super::{ExpandedEditorCanvas, TOOLBAR_HEIGHT};

// ---------------------------------------------------------------------------
// Drawing helpers
// ---------------------------------------------------------------------------

impl<'a> ExpandedEditorCanvas<'a> {
    pub(super) fn draw_note_rows(
        &self,
        frame: &mut Frame,
        layout: &PianoRollLayout,
        viewport: &PianoRollViewport,
        grid_w: f32,
    ) {
        let grid_x = layout.grid_x();
        for midi_note in 0..NOTE_COUNT {
            let y = layout.grid_top + viewport.note_to_y_local(midi_note);
            let h = viewport.zoom_y;

            if y + h < layout.grid_top || y > layout.grid_top + layout.grid_h {
                continue;
            }

            let is_black = is_black_key(midi_note);
            let in_scale = self.scale.map(|s| s.contains(midi_note)).unwrap_or(true);

            // Black-key rows render darker against the BG_2 backdrop;
            // out-of-scale rows get a subtle warm tint. White-key rows
            // skip the fill entirely so the BG_2 backdrop shows through.
            let color = if !in_scale {
                Some(Color {
                    a: 0.06,
                    ..theme::WARM
                })
            } else if is_black {
                Some(theme::BG_1)
            } else {
                None
            };
            if let Some(color) = color {
                frame.fill_rectangle(Point::new(grid_x, y), Size::new(grid_w, h), color);
            }

            if midi_note % 12 == 0 {
                frame.fill_rectangle(
                    Point::new(grid_x, y + h - 1.0),
                    Size::new(grid_w, 1.0),
                    theme::LINE_2,
                );
            }
        }
    }

    pub(super) fn draw_beat_grid(
        &self,
        frame: &mut Frame,
        layout: &PianoRollLayout,
        viewport: &PianoRollViewport,
        grid_w: f32,
    ) {
        if self.section_length_bars == 0 {
            return;
        }
        let grid_x = layout.grid_x();
        let grid_h = layout.grid_h;
        let tpb = TICKS_PER_QUARTER_NOTE;
        let total_ticks = self.section_ticks();

        // Walk bars for correct placement with varying time signatures.
        let mut tick_pos: u64 = 0;
        for bar_offset in 0..self.section_length_bars {
            let bar = self.start_bar + bar_offset;
            let num = self.tempo_map.numerator_at_bar(bar) as u64;
            let bar_ticks = num * tpb;

            // Bar line — LINE, 1px hairline like the rest of the redesign.
            let x = grid_x + viewport.tick_to_x_local(tick_pos);
            if x >= grid_x && x <= grid_x + grid_w {
                frame.fill_rectangle(
                    Point::new(x, TOOLBAR_HEIGHT),
                    Size::new(1.0, grid_h),
                    theme::LINE,
                );
                if tick_pos < total_ticks {
                    frame.fill_text(canvas::Text {
                        content: format!("{}", bar_offset + 1),
                        position: Point::new(x + 3.0, TOOLBAR_HEIGHT + 2.0),
                        color: theme::TEXT_3,
                        size: 9.0.into(),
                        font: theme::MONO_FONT,
                        ..canvas::Text::default()
                    });
                }
            }

            // Beat lines — LINE_2 hairlines.
            for beat in 1..num {
                let beat_tick = tick_pos + beat * tpb;
                let bx = grid_x + viewport.tick_to_x_local(beat_tick);
                if bx >= grid_x && bx <= grid_x + grid_w {
                    frame.fill_rectangle(
                        Point::new(bx, TOOLBAR_HEIGHT),
                        Size::new(1.0, grid_h),
                        theme::LINE_2,
                    );
                }
            }

            tick_pos += bar_ticks;
        }
        // Final bar line at section end
        let x = grid_x + viewport.tick_to_x_local(tick_pos);
        if x >= grid_x && x <= grid_x + grid_w {
            frame.fill_rectangle(
                Point::new(x, TOOLBAR_HEIGHT),
                Size::new(1.0, grid_h),
                theme::LINE,
            );
        }

        // Subdivision lines (16th notes) when zoomed in enough — even
        // softer than beat lines so they don't compete.
        let sub = tpb / 4;
        let sub_px = sub as f32 * viewport.zoom_x;
        if sub_px >= 6.0 {
            let sub_color = Color {
                a: 0.5,
                ..theme::LINE_2
            };
            for idx in 0..=(total_ticks / sub) {
                let tick = idx * sub;
                if tick.is_multiple_of(tpb) {
                    continue;
                }
                let x = grid_x + viewport.tick_to_x_local(tick);
                if x < grid_x || x > grid_x + grid_w {
                    continue;
                }
                frame.fill_rectangle(
                    Point::new(x, TOOLBAR_HEIGHT),
                    Size::new(1.0, grid_h),
                    sub_color,
                );
            }
        }
    }

    pub(super) fn draw_notes(
        &self,
        frame: &mut Frame,
        layout: &PianoRollLayout,
        viewport: &PianoRollViewport,
    ) {
        let grid_x = layout.grid_x();
        for clip in self
            .midi_clips
            .iter()
            .filter(|c| c.track_id == self.track_id)
        {
            if !self.clip_intersects_section(clip) {
                continue;
            }

            for n in &clip.notes {
                let rect = self.note_rect(layout, viewport, n);

                if rect.x + rect.width < grid_x
                    || rect.y + rect.height < layout.grid_top
                    || rect.y > layout.grid_top + layout.grid_h
                {
                    continue;
                }

                // Always paint with the brighter ACCENT stroke (no
                // selection state in this canvas); labels render inside
                // notes large enough to be readable.
                let style = NoteStyle {
                    stroke: theme::ACCENT,
                    stroke_width: 1.0,
                    label: Some(note_name(n.note)),
                };
                piano_roll::draw_note(frame, rect, n.velocity, style);
            }
        }
    }
}
