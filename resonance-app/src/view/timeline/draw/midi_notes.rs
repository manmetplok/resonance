//! MIDI clip drawing: chrome and note-preview minimap.
use iced::widget::canvas;
use iced::{Color, Point, Size};

use crate::state::MidiClipState;
use crate::theme;
use crate::view::arrange_layout::ArrangeRowLayout;
use super::TimelineCanvas;
use super::chrome::{ClipChrome, clip_lane_rect};

impl TimelineCanvas<'_> {
    pub(in crate::view::timeline) fn draw_midi_clip(
        &self,
        frame: &mut canvas::Frame,
        clip: &MidiClipState,
        layout: &ArrangeRowLayout,
        ruler_height: f32,
        y_off: f32,
        visible_height: f32,
    ) {
        let Some((y, clip_height, indent)) = clip_lane_rect(
            self,
            clip.track_id,
            layout,
            ruler_height,
            y_off,
            visible_height,
        ) else {
            return;
        };

        let clip_end_sample = self.tempo_map.tick_to_abs_sample(
            clip.start_sample,
            clip.duration_ticks,
            self.sample_rate,
        );
        let duration_samples = clip_end_sample.saturating_sub(clip.start_sample) as f64;
        let start_seconds = clip.start_sample as f32 / self.sample_rate as f32;
        let duration_seconds = duration_samples as f32 / self.sample_rate as f32;

        let x = start_seconds * self.zoom - self.scroll_offset + indent;
        let w = duration_seconds * self.zoom;
        if w <= 0.0 {
            return;
        }

        // MIDI clips: lavender wash + lavender border, name in lavender
        // accent.
        let is_selected = self.selected_midi_clip == Some(clip.id);
        let chrome = ClipChrome {
            x,
            y,
            w,
            h: clip_height,
            body_color: Color {
                a: 0.10,
                ..theme::ACCENT
            },
            border_color: if is_selected {
                theme::ACCENT
            } else {
                theme::ACCENT_LINE
            },
            is_selected,
            name: &clip.name,
            name_color: theme::ACCENT_SOFT,
            show_name: true,
        };

        chrome.draw(frame, |frame| {
            self.draw_midi_clip_notes(frame, clip, x, y, w, clip_height)
        });
    }

    /// Note preview — small lavender rects mapped to the clip's note
    /// range. Drawn dimmed so the wash still reads as lavender.
    fn draw_midi_clip_notes(
        &self,
        frame: &mut canvas::Frame,
        clip: &MidiClipState,
        x: f32,
        y: f32,
        w: f32,
        clip_height: f32,
    ) {
        let header_height = 18.0;
        let note_area_y = y + header_height;
        let note_area_h = clip_height - header_height - 4.0;

        if !clip.notes.is_empty() && note_area_h > 2.0 && w > 2.0 {
            let mut min_note: u8 = 127;
            let mut max_note: u8 = 0;
            for note in &clip.notes {
                if note.note < min_note {
                    min_note = note.note;
                }
                if note.note > max_note {
                    max_note = note.note;
                }
            }
            let range_min = min_note.saturating_sub(2);
            let range_max = (max_note + 2).min(127);
            let note_range = (range_max - range_min).max(1) as f32;

            let total_ticks = clip.duration_ticks as f32;
            if total_ticks > 0.0 {
                let note_color = Color {
                    a: 0.85,
                    ..theme::ACCENT_SOFT
                };
                for note in &clip.notes {
                    let note_start_in_clip =
                        note.start_tick as f32 - clip.trim_start_ticks as f32;
                    if note_start_in_clip + note.duration_ticks as f32 <= 0.0 {
                        continue;
                    }
                    if note_start_in_clip >= total_ticks {
                        continue;
                    }
                    let visible_start = note_start_in_clip.max(0.0);
                    let visible_end =
                        (note_start_in_clip + note.duration_ticks as f32).min(total_ticks);

                    let nx = x + (visible_start / total_ticks) * w;
                    let nw = ((visible_end - visible_start) / total_ticks) * w;

                    let ny = note_area_y
                        + (1.0 - (note.note as f32 - range_min as f32) / note_range)
                            * (note_area_h - 3.0);
                    let nh = (note_area_h / note_range).clamp(2.0, 6.0);

                    frame.fill_rectangle(
                        Point::new(nx, ny),
                        Size::new(nw.max(1.0), nh),
                        note_color,
                    );
                }
            }
        }
    }
}
