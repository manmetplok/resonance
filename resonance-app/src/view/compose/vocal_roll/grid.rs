//! Structural backdrop of the vocal roll: per-note-row backgrounds,
//! bar/beat grid lines, the read-only chord strip, and the phoneme strip.
//! Everything here paints a region that's independent of the note
//! contents — those overlays live in [`super::notes`].

use iced::widget::canvas::{self, Frame, Path, Stroke};
use iced::{Color, Point, Size};

use resonance_audio::types::TICKS_PER_QUARTER_NOTE;
use resonance_music_theory::g2p;

use crate::theme;

use super::{
    chord_label, is_black_key, VocalRollCanvas, VR_CHORD_STRIP_HEIGHT, VR_PHONEME_STRIP_HEIGHT,
};

impl VocalRollCanvas<'_> {
    pub(super) fn draw_note_rows(
        &self,
        frame: &mut Frame,
        grid_x: f32,
        grid_w: f32,
        grid_top: f32,
        grid_h: f32,
    ) {
        // Backdrop
        frame.fill_rectangle(
            Point::new(grid_x, grid_top),
            Size::new(grid_w, grid_h),
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
            if is_black_key(note) {
                frame.fill_rectangle(Point::new(grid_x, y), Size::new(grid_w, h), theme::BG_1);
            }
            if note % 12 == 0 {
                frame.fill_rectangle(
                    Point::new(grid_x, y + h - 1.0),
                    Size::new(grid_w, 1.0),
                    theme::LINE_2,
                );
            }
        }
    }

    pub(super) fn draw_grid_lines(
        &self,
        frame: &mut Frame,
        grid_x: f32,
        grid_w: f32,
        grid_top: f32,
        grid_h: f32,
    ) {
        let ticks_per_beat = TICKS_PER_QUARTER_NOTE;
        let ticks_per_bar = TICKS_PER_QUARTER_NOTE * self.time_sig_num as u64;
        let pixels_per_beat = ticks_per_beat as f32 * self.zoom_x;
        if pixels_per_beat < 4.0 {
            return;
        }
        let max_tick = ((grid_w / self.zoom_x) as u64) + ticks_per_beat;
        let mut tick = 0u64;
        while tick <= max_tick {
            let x = grid_x + self.tick_to_x(tick);
            if x > grid_x + grid_w {
                break;
            }
            let is_bar = tick.is_multiple_of(ticks_per_bar);
            let color = if is_bar { theme::BAR_LINE } else { theme::BEAT_LINE };
            frame.fill_rectangle(
                Point::new(x, grid_top),
                Size::new(1.0, grid_h),
                color,
            );
            tick = tick.saturating_add(ticks_per_beat);
        }

        // 16th sub-divisions when zoomed in.
        let snap_px = self.snap_ticks as f32 * self.zoom_x;
        if snap_px >= 8.0 && self.snap_ticks < ticks_per_beat {
            let mut tick = 0u64;
            while tick <= max_tick {
                if !tick.is_multiple_of(ticks_per_beat) {
                    let x = grid_x + self.tick_to_x(tick);
                    if x > grid_x + grid_w {
                        break;
                    }
                    frame.fill_rectangle(
                        Point::new(x, grid_top),
                        Size::new(1.0, grid_h),
                        Color { a: 0.5, ..theme::LINE_2 },
                    );
                }
                tick = tick.saturating_add(self.snap_ticks);
            }
        }
    }

    /// Read-only chord context strip aligned to the section's beat
    /// timeline so chord boundaries land on bar lines.
    pub(super) fn draw_chord_strip(&self, frame: &mut Frame, grid_x: f32, grid_w: f32) {
        // Background — warm tint.
        frame.fill_rectangle(
            Point::new(grid_x, 0.0),
            Size::new(grid_w, VR_CHORD_STRIP_HEIGHT),
            Color { a: 0.06, ..theme::WARM },
        );
        let ticks_per_beat = TICKS_PER_QUARTER_NOTE;
        let section_ticks = (self.section_beats as u64) * ticks_per_beat;
        if section_ticks == 0 {
            return;
        }
        for c in self.chords {
            let start_tick = c.start_beat as u64 * ticks_per_beat;
            let dur_tick = c.duration_beats as u64 * ticks_per_beat;
            let x0 = grid_x + self.tick_to_x(start_tick);
            let w = self.duration_to_width(dur_tick);
            if w < 1.0 {
                continue;
            }
            // Cell border
            frame.stroke(
                &Path::rounded_rectangle(
                    Point::new(x0 + 1.0, 3.0),
                    Size::new((w - 2.0).max(2.0), VR_CHORD_STRIP_HEIGHT - 6.0),
                    3.0.into(),
                ),
                Stroke::default()
                    .with_color(Color { a: 0.45, ..theme::WARM })
                    .with_width(1.0),
            );
            // Symbol — root + quality. Italic serif for the symbol.
            let label = chord_label(&c.chord);
            frame.fill_text(canvas::Text {
                content: label,
                position: Point::new(x0 + 8.0, 6.0),
                color: theme::WARM,
                size: 13.0.into(),
                font: theme::SERIF_ITALIC_FONT,
                ..canvas::Text::default()
            });
        }
    }

    /// Phoneme strip — per-note ARPAbet breakdown. Reads phonemes
    /// directly from `AssignedSyllable::phonemes`, so the labels here
    /// are guaranteed to match what `build_segment` feeds the SVS
    /// model. Slur notes show the held vowel from the previous
    /// syllable; non-slur notes show their full phoneme list.
    pub(super) fn draw_phoneme_strip(
        &self,
        frame: &mut Frame,
        grid_x: f32,
        grid_w: f32,
        assigned: &[g2p::AssignedSyllable],
    ) {
        frame.fill_rectangle(
            Point::new(grid_x, VR_CHORD_STRIP_HEIGHT),
            Size::new(grid_w, VR_PHONEME_STRIP_HEIGHT),
            theme::BG_1,
        );
        // Section label on the left edge.
        frame.fill_text(canvas::Text {
            content: "PHN".to_string(),
            position: Point::new(6.0, VR_CHORD_STRIP_HEIGHT + 5.0),
            color: theme::TEXT_3,
            size: 8.5.into(),
            font: theme::UI_FONT_SEMIBOLD,
            ..canvas::Text::default()
        });
        let strip_y = VR_CHORD_STRIP_HEIGHT + 4.0;

        if self.clip.notes.is_empty() || assigned.iter().all(|a| a.phonemes.is_empty()) {
            frame.fill_text(canvas::Text {
                content: "(no phonemes \u{2014} generate from the right rail)".to_string(),
                position: Point::new(grid_x + 8.0, strip_y + 1.0),
                color: theme::TEXT_3,
                size: 10.5.into(),
                font: theme::SERIF_ITALIC_FONT,
                ..canvas::Text::default()
            });
            return;
        }

        for (i, n) in self.clip.notes.iter().enumerate() {
            let Some(a) = assigned.get(i) else { break };
            if a.phonemes.is_empty() {
                continue;
            }
            let display = a.phonemes.join(" ");
            let x = grid_x + self.tick_to_x(n.start_tick);
            let nw = self.duration_to_width(n.duration_ticks).max(8.0);
            if x > grid_x + grid_w {
                break;
            }
            let pill_w = nw.clamp(14.0, 72.0);
            let pill_alpha = if a.is_slur { 0.06 } else { 0.10 };
            frame.fill(
                &Path::rounded_rectangle(
                    Point::new(x + 1.0, strip_y - 1.0),
                    Size::new(pill_w - 2.0, VR_PHONEME_STRIP_HEIGHT - 6.0),
                    3.0.into(),
                ),
                Color { a: pill_alpha, ..theme::WARM },
            );
            let text_color = if a.is_slur {
                Color { a: 0.65, ..theme::WARM }
            } else {
                theme::WARM
            };
            frame.fill_text(canvas::Text {
                content: display,
                position: Point::new(x + 4.0, strip_y + 1.0),
                color: text_color,
                size: 9.5.into(),
                font: theme::MONO_FONT,
                ..canvas::Text::default()
            });
        }
    }
}
