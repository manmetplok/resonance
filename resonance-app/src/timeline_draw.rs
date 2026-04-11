//! Canvas drawing for the timeline. These are the pure-draw methods
//! that take a `&mut Frame` and render bar/beat grids, rulers, audio
//! clips, and MIDI clips. They're in a separate impl block (and file)
//! so `timeline.rs` can stay focused on canvas event handling and state.
use iced::widget::canvas;
use iced::{Color, Point, Size};
use resonance_audio::types::TICKS_PER_QUARTER_NOTE;

use crate::state::{ClipState, MidiClipState, TrackState};
use crate::theme;
use crate::timeline::TimelineCanvas;

impl TimelineCanvas<'_> {
    /// Draw vertical bar and beat grid lines in the track area.
    pub(super) fn draw_grid_lines(
        &self,
        frame: &mut canvas::Frame,
        width: f32,
        ruler_height: f32,
        track_area_height: f32,
        _y_off: f32,
    ) {
        let spb_seconds = self.seconds_per_beat();
        let spbar_seconds = self.seconds_per_bar();
        let line_height = track_area_height.max(600.0);

        let start_time = self.scroll_offset / self.zoom;
        let end_time = start_time + width / self.zoom;

        // Determine bar step for readability at low zoom
        let bar_pixel_width = spbar_seconds * self.zoom;
        let bar_step = if bar_pixel_width < 20.0 {
            (20.0 / bar_pixel_width).ceil() as u32
        } else {
            1
        };

        let first_bar = (start_time / spbar_seconds).floor() as i64;
        let last_bar = (end_time / spbar_seconds).ceil() as i64;

        for bar_idx in first_bar..=last_bar {
            if bar_step > 1 && bar_idx.rem_euclid(bar_step as i64) != 0 {
                continue;
            }
            let bar_time = bar_idx as f32 * spbar_seconds;

            // Bar line
            let x = bar_time * self.zoom - self.scroll_offset;
            if x >= -1.0 && x <= width + 1.0 {
                frame.fill_rectangle(
                    Point::new(x, ruler_height),
                    Size::new(1.0, line_height),
                    theme::BAR_LINE,
                );
            }

            // Beat lines within this bar (skip beat 1, that's the bar line)
            if bar_pixel_width >= 40.0 {
                for beat in 1..self.time_sig_num {
                    let beat_time = bar_time + beat as f32 * spb_seconds;
                    let bx = beat_time * self.zoom - self.scroll_offset;
                    if bx >= 0.0 && bx <= width {
                        frame.fill_rectangle(
                            Point::new(bx, ruler_height),
                            Size::new(1.0, line_height),
                            theme::BEAT_LINE,
                        );
                    }
                }
            }
        }
    }

    /// Draw the bar/beat ruler at the top.
    pub(super) fn draw_ruler(
        &self,
        frame: &mut canvas::Frame,
        width: f32,
        ruler_height: f32,
    ) {
        let spbar_seconds = self.seconds_per_bar();
        let spb_seconds = self.seconds_per_beat();

        let start_time = self.scroll_offset / self.zoom;
        let end_time = start_time + width / self.zoom;

        // Determine bar step for readability at low zoom
        let bar_pixel_width = spbar_seconds * self.zoom;
        let bar_step = if bar_pixel_width < 40.0 {
            (40.0 / bar_pixel_width).ceil() as u32
        } else {
            1
        };

        let first_bar = (start_time / spbar_seconds).floor() as i64;
        let last_bar = (end_time / spbar_seconds).ceil() as i64;

        for bar_idx in first_bar..=last_bar {
            let bar_time = bar_idx as f32 * spbar_seconds;
            let bar_number = bar_idx + 1; // 1-based

            if bar_step > 1 && bar_idx.rem_euclid(bar_step as i64) != 0 {
                continue;
            }

            let x = bar_time * self.zoom - self.scroll_offset;

            if x < -1.0 || x > width + 1.0 {
                continue;
            }

            // Major tick (bar)
            frame.fill_rectangle(
                Point::new(x, ruler_height - 12.0),
                Size::new(1.0, 12.0),
                theme::TEXT_DIM,
            );

            // Bar number label
            frame.fill_text(canvas::Text {
                content: format!("{}", bar_number),
                position: Point::new(x + 3.0, ruler_height - 24.0),
                color: theme::TEXT_DIM,
                size: 11.0.into(),
                ..canvas::Text::default()
            });

            // Beat ticks within bar (only if enough space)
            if bar_pixel_width >= 40.0 {
                for beat in 1..self.time_sig_num {
                    let beat_time = bar_time + beat as f32 * spb_seconds;
                    let bx = beat_time * self.zoom - self.scroll_offset;
                    if bx >= 0.0 && bx <= width {
                        frame.fill_rectangle(
                            Point::new(bx, ruler_height - 6.0),
                            Size::new(1.0, 6.0),
                            Color::from_rgb(0.25, 0.25, 0.25),
                        );
                    }
                }
            }
        }

        // Ruler bottom line
        frame.fill_rectangle(
            Point::new(0.0, ruler_height - 1.0),
            Size::new(width, 1.0),
            theme::SEPARATOR,
        );
    }

    pub(super) fn draw_clip(
        &self,
        frame: &mut canvas::Frame,
        clip: &ClipState,
        sorted_tracks: &[&TrackState],
        ruler_height: f32,
        y_off: f32,
        visible_height: f32,
    ) {
        let track_index = sorted_tracks.iter().position(|t| t.id == clip.track_id);

        let track_index = match track_index {
            Some(i) => i,
            None => return,
        };

        let y = ruler_height + track_index as f32 * theme::TRACK_HEIGHT + 2.0 - y_off;

        // Skip clips on tracks outside visible area
        if y + theme::TRACK_HEIGHT < ruler_height || y > visible_height {
            return;
        }
        let clip_height = theme::TRACK_HEIGHT - 4.0;

        let start_seconds = clip.start_sample as f32 / self.sample_rate as f32;
        let duration_seconds = clip.duration_samples as f32 / self.sample_rate as f32;

        let x = start_seconds * self.zoom - self.scroll_offset;
        let w = duration_seconds * self.zoom;

        // Clip body
        frame.fill_rectangle(
            Point::new(x, y),
            Size::new(w, clip_height),
            theme::CLIP_BODY,
        );

        // Waveform rendering
        let header_height = 18.0;
        if !clip.waveform_peaks.is_empty() {
            let wave_y = y + header_height;
            let wave_h = clip_height - header_height;
            let wave_center = wave_y + wave_h * 0.5;

            let peak_frames = resonance_audio::types::WAVEFORM_PEAK_FRAMES as f32;
            let seconds_per_peak = peak_frames / self.sample_rate as f32;
            let pixels_per_peak = seconds_per_peak * self.zoom;

            // Determine which peaks are visible (accounting for trim)
            let trim_start_peaks = clip.trim_start_frames as f32 / peak_frames;
            let _total_visible_peaks = clip.duration_samples as f32 / peak_frames;

            let waveform_color = Color::from_rgba(0.7, 0.85, 1.0, 0.5);

            let start_px = (-x).max(0.0);
            let mut px = start_px;
            while px < w {
                let peak_idx_f = trim_start_peaks + px / pixels_per_peak;
                let peak_idx = peak_idx_f as usize;
                if peak_idx >= clip.waveform_peaks.len() {
                    break;
                }
                let (min_val, max_val) = clip.waveform_peaks[peak_idx];

                let draw_x = x + px;
                // Only draw if on-screen
                if draw_x + pixels_per_peak >= 0.0 && draw_x <= w + x {
                    let top = wave_center - max_val * wave_h * 0.5;
                    let bottom = wave_center - min_val * wave_h * 0.5;
                    let bar_h = (bottom - top).max(1.0);
                    frame.fill_rectangle(
                        Point::new(draw_x, top),
                        Size::new(pixels_per_peak.max(1.0), bar_h),
                        waveform_color,
                    );
                }
                px += pixels_per_peak.max(1.0);
            }
        }

        // Clip header bar
        frame.fill_rectangle(
            Point::new(x, y),
            Size::new(w, header_height),
            theme::CLIP_HEADER,
        );

        // Clip name (truncated safely for multi-byte UTF-8)
        let display_name: String = if clip.name.chars().count() > 20 {
            let mut truncated: String = clip.name.chars().take(17).collect();
            truncated.push_str("...");
            truncated
        } else {
            clip.name.clone()
        };
        frame.fill_text(canvas::Text {
            content: display_name,
            position: Point::new(x + 4.0, y + 2.0),
            color: theme::TEXT,
            size: 11.0.into(),
            ..canvas::Text::default()
        });

        // Clip border (highlighted if selected)
        let is_selected = self.selected_clip == Some(clip.id);
        let border = canvas::Path::rectangle(Point::new(x, y), Size::new(w, clip_height));
        if is_selected {
            frame.stroke(
                &border,
                canvas::Stroke::default()
                    .with_color(theme::CLIP_SELECTED_BORDER)
                    .with_width(2.0),
            );
        } else {
            frame.stroke(
                &border,
                canvas::Stroke::default()
                    .with_color(Color::from_rgba(0.0, 0.0, 0.0, 0.3))
                    .with_width(1.0),
            );
        }
    }

    pub(super) fn draw_midi_clip(
        &self,
        frame: &mut canvas::Frame,
        clip: &MidiClipState,
        sorted_tracks: &[&TrackState],
        ruler_height: f32,
        y_off: f32,
        visible_height: f32,
    ) {
        let track_index = sorted_tracks.iter().position(|t| t.id == clip.track_id);

        let track_index = match track_index {
            Some(i) => i,
            None => return,
        };

        let y = ruler_height + track_index as f32 * theme::TRACK_HEIGHT + 2.0 - y_off;

        if y + theme::TRACK_HEIGHT < ruler_height || y > visible_height {
            return;
        }
        let clip_height = theme::TRACK_HEIGHT - 4.0;

        // Convert tick duration to samples, then to seconds for pixel width
        let samples_per_tick =
            (self.sample_rate as f64 * 60.0 / self.bpm as f64) / TICKS_PER_QUARTER_NOTE as f64;
        let duration_samples = clip.duration_ticks as f64 * samples_per_tick;
        let start_seconds = clip.start_sample as f32 / self.sample_rate as f32;
        let duration_seconds = duration_samples as f32 / self.sample_rate as f32;

        let x = start_seconds * self.zoom - self.scroll_offset;
        let w = duration_seconds * self.zoom;

        // Teal/cyan clip body
        let midi_body_color = Color::from_rgb(0.12, 0.22, 0.25);
        frame.fill_rectangle(
            Point::new(x, y),
            Size::new(w, clip_height),
            midi_body_color,
        );

        // Draw note rectangles inside the clip body
        let header_height = 18.0;
        let note_area_y = y + header_height;
        let note_area_h = clip_height - header_height;

        if !clip.notes.is_empty() && note_area_h > 2.0 && w > 2.0 {
            // Find the note range for vertical mapping
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
            // Add padding so notes aren't flush with edges
            let range_min = min_note.saturating_sub(2);
            let range_max = (max_note + 2).min(127);
            let note_range = (range_max - range_min).max(1) as f32;

            let total_ticks = clip.duration_ticks as f32;
            if total_ticks > 0.0 {
                for note in &clip.notes {
                    // Horizontal position: note start relative to clip visible start
                    let note_start_in_clip =
                        note.start_tick as f32 - clip.trim_start_ticks as f32;
                    if note_start_in_clip + note.duration_ticks as f32 <= 0.0 {
                        continue; // note is before visible area
                    }
                    if note_start_in_clip >= total_ticks {
                        continue; // note is after visible area
                    }
                    let visible_start = note_start_in_clip.max(0.0);
                    let visible_end =
                        (note_start_in_clip + note.duration_ticks as f32).min(total_ticks);

                    let nx = x + (visible_start / total_ticks) * w;
                    let nw = ((visible_end - visible_start) / total_ticks) * w;

                    // Vertical position: highest note at top
                    let ny = note_area_y
                        + (1.0 - (note.note as f32 - range_min as f32) / note_range)
                            * (note_area_h - 3.0);
                    let nh = (note_area_h / note_range).max(2.0).min(6.0);

                    // Color intensity maps to velocity
                    let vel = note.velocity.clamp(0.0, 1.0);
                    let note_color = Color::from_rgba(
                        0.2 + 0.6 * vel,
                        0.7 + 0.3 * vel,
                        0.8 + 0.2 * vel,
                        0.7 + 0.3 * vel,
                    );

                    frame.fill_rectangle(
                        Point::new(nx, ny),
                        Size::new(nw.max(1.0), nh),
                        note_color,
                    );
                }
            }
        }

        // Clip header bar (teal accent)
        let midi_header_color = Color::from_rgb(0.15, 0.45, 0.50);
        frame.fill_rectangle(
            Point::new(x, y),
            Size::new(w, header_height),
            midi_header_color,
        );

        // Clip name
        let display_name: String = if clip.name.chars().count() > 20 {
            let mut truncated: String = clip.name.chars().take(17).collect();
            truncated.push_str("...");
            truncated
        } else {
            clip.name.clone()
        };
        frame.fill_text(canvas::Text {
            content: display_name,
            position: Point::new(x + 4.0, y + 2.0),
            color: theme::TEXT,
            size: 11.0.into(),
            ..canvas::Text::default()
        });

        // Clip border (highlighted if selected)
        let is_selected = self.selected_midi_clip == Some(clip.id);
        let border = canvas::Path::rectangle(Point::new(x, y), Size::new(w, clip_height));
        if is_selected {
            frame.stroke(
                &border,
                canvas::Stroke::default()
                    .with_color(theme::CLIP_SELECTED_BORDER)
                    .with_width(2.0),
            );
        } else {
            frame.stroke(
                &border,
                canvas::Stroke::default()
                    .with_color(Color::from_rgba(0.0, 0.0, 0.0, 0.3))
                    .with_width(1.0),
            );
        }
    }
}
