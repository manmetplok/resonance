//! Canvas drawing for the timeline. These are the pure-draw methods
//! that take a `&mut Frame` and render bar/beat grids, rulers, audio
//! clips, and MIDI clips. They're in a separate impl block (and file)
//! so `timeline.rs` can stay focused on canvas event handling and state.
use iced::widget::canvas;
use iced::{Color, Point, Size};

use crate::state::{self, ClipState, MidiClipState, TrackState};
use crate::theme;
use crate::timeline::TimelineCanvas;
use resonance_audio::types::avg_bpm_for_bar;

impl TimelineCanvas<'_> {
    /// Draw the global tracks area (tempo + time signature) between the
    /// ruler and the regular track lanes. The tempo row shows a line
    /// graph (like Logic Pro) with draggable points connected by lines;
    /// the signature row keeps the block-marker style.
    pub(super) fn draw_global_tracks(
        &self,
        frame: &mut canvas::Frame,
        width: f32,
        ruler_height: f32,
    ) {
        if !self.global_tracks_expanded {
            return;
        }
        let row_h = theme::GLOBAL_TRACK_ROW_HEIGHT;

        // ---- Tempo row background ----
        let tempo_y = ruler_height;
        frame.fill_rectangle(
            Point::new(0.0, tempo_y),
            Size::new(width, row_h),
            theme::GLOBAL_TRACK_BG,
        );
        // Separator
        frame.fill_rectangle(
            Point::new(0.0, tempo_y + row_h - 1.0),
            Size::new(width, 1.0),
            theme::SEPARATOR,
        );

        // ---- Time signature row background ----
        let sig_y = ruler_height + row_h;
        frame.fill_rectangle(
            Point::new(0.0, sig_y),
            Size::new(width, row_h),
            theme::GLOBAL_TRACK_BG,
        );
        // Bottom separator
        frame.fill_rectangle(
            Point::new(0.0, sig_y + row_h - 1.0),
            Size::new(width, 1.0),
            theme::SEPARATOR,
        );

        // ---- Draw tempo line graph ----
        if !self.tempo_map.tempo_points.is_empty() {
            // Determine BPM range for vertical mapping.
            let mut min_bpm = f32::MAX;
            let mut max_bpm = f32::MIN;
            for e in &self.tempo_map.tempo_points {
                min_bpm = min_bpm.min(e.bpm);
                max_bpm = max_bpm.max(e.bpm);
            }
            // Add padding so points aren't flush with edges; ensure a
            // minimum range so a flat tempo doesn't compress to zero.
            let range = (max_bpm - min_bpm).max(10.0);
            let pad = range * 0.15;
            let lo = min_bpm - pad;
            let hi = max_bpm + pad;

            let graph_top = tempo_y + 3.0;
            let graph_bot = tempo_y + row_h - 3.0;
            let graph_h = graph_bot - graph_top;

            // Map BPM to y within the tempo row (high BPM = top).
            let bpm_to_y = |bpm: f32| -> f32 { graph_bot - ((bpm - lo) / (hi - lo)) * graph_h };

            // Build (x, y, bpm, is_selected) for each event point.
            let points: Vec<(f32, f32, f32, bool)> = self
                .tempo_map
                .tempo_points
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let sample = self.tempo_map.bar_to_sample(e.bar);
                    let x = self.sample_to_x(sample);
                    let y = bpm_to_y(e.bpm);
                    let selected = self.selected_global_event
                        == Some(state::SelectedGlobalEvent {
                            kind: state::GlobalTrackKind::Tempo,
                            index: i,
                        });
                    (x, y, e.bpm, selected)
                })
                .collect();

            // Draw connecting lines and filled area.
            let line_color = Color::from_rgba(0.9, 0.55, 0.15, 0.7);
            let fill_color = Color::from_rgba(0.9, 0.55, 0.15, 0.10);

            for pair in points.windows(2) {
                let (x1, y1, _, _) = pair[0];
                let (x2, y2, _, _) = pair[1];
                if x2 < -50.0 || x1 > width + 50.0 {
                    continue;
                }
                // Filled area under the line segment.
                let steps = ((x2 - x1).abs() as u32).max(1).min(400);
                for s in 0..steps {
                    let t = s as f32 / steps as f32;
                    let px = x1 + t * (x2 - x1);
                    let py = y1 + t * (y2 - y1);
                    if px >= 0.0 && px <= width {
                        frame.fill_rectangle(
                            Point::new(px, py),
                            Size::new(1.0, graph_bot - py),
                            fill_color,
                        );
                    }
                }
                // Line itself (2 px wide via two 1 px rects).
                let steps = ((x2 - x1).abs() as u32).max(1).min(800);
                for s in 0..=steps {
                    let t = s as f32 / steps as f32;
                    let px = x1 + t * (x2 - x1);
                    let py = y1 + t * (y2 - y1);
                    if px >= 0.0 && px <= width {
                        frame.fill_rectangle(
                            Point::new(px, py - 0.5),
                            Size::new(1.0, 2.0),
                            line_color,
                        );
                    }
                }
            }

            // Extend the last point to the right edge.
            if let Some(&(last_x, last_y, _, _)) = points.last() {
                if last_x < width {
                    let start = last_x.max(0.0);
                    frame.fill_rectangle(
                        Point::new(start, last_y),
                        Size::new(width - start, graph_bot - last_y),
                        fill_color,
                    );
                    frame.fill_rectangle(
                        Point::new(start, last_y - 0.5),
                        Size::new(width - start, 2.0),
                        line_color,
                    );
                }
            }

            // Draw event points (dots) and BPM labels.
            for (i, &(x, y, bpm, selected)) in points.iter().enumerate() {
                if x > width + 50.0 || x < -50.0 {
                    continue;
                }
                // Dot.
                let dot_r = if selected { 4.0 } else { 3.0 };
                let dot_color = if selected {
                    theme::ACCENT
                } else {
                    Color::from_rgb(0.9, 0.55, 0.15)
                };
                if x >= -dot_r && x <= width + dot_r {
                    frame.fill_rectangle(
                        Point::new(x - dot_r, y - dot_r),
                        Size::new(dot_r * 2.0, dot_r * 2.0),
                        dot_color,
                    );
                }
                // Vertical marker line.
                if i > 0 && x >= 0.0 {
                    let marker_color = if selected {
                        theme::ACCENT
                    } else {
                        Color::from_rgba(0.9, 0.55, 0.15, 0.3)
                    };
                    frame.fill_rectangle(
                        Point::new(x, tempo_y),
                        Size::new(1.0, row_h),
                        marker_color,
                    );
                }
                // BPM label.
                let label_x = x.max(2.0) + 5.0;
                if label_x < width - 10.0 {
                    frame.fill_text(canvas::Text {
                        content: format!("{:.0}", bpm),
                        position: Point::new(label_x, tempo_y + 2.0),
                        color: if selected {
                            theme::ACCENT
                        } else {
                            theme::TEXT_DIM
                        },
                        size: 10.0.into(),
                        ..canvas::Text::default()
                    });
                }
            }
        }

        // ---- Draw signature event markers ----
        for (i, event) in self.tempo_map.signature_points.iter().enumerate() {
            let sample = self.tempo_map.bar_to_sample(event.bar);
            let x = self.sample_to_x(sample);
            if x > width + 50.0 || x < -50.0 {
                continue;
            }
            let next_x = self
                .tempo_map
                .signature_points
                .get(i + 1)
                .map(|ne| self.sample_to_x(self.tempo_map.bar_to_sample(ne.bar)))
                .unwrap_or(width);
            let block_w = (next_x - x).max(2.0).min(width - x.max(0.0));

            let is_selected = self.selected_global_event
                == Some(state::SelectedGlobalEvent {
                    kind: state::GlobalTrackKind::Signature,
                    index: i,
                });

            let block_color = if is_selected {
                Color::from_rgba(0.3, 0.6, 0.9, 0.25)
            } else {
                Color::from_rgba(0.3, 0.6, 0.9, 0.12)
            };
            frame.fill_rectangle(
                Point::new(x.max(0.0), sig_y + 1.0),
                Size::new(block_w, row_h - 2.0),
                block_color,
            );

            if x >= 0.0 {
                let marker_color = if is_selected {
                    theme::ACCENT
                } else {
                    theme::TEXT_DIM
                };
                frame.fill_rectangle(Point::new(x, sig_y), Size::new(1.0, row_h), marker_color);
            }

            let label_x = x.max(2.0) + 3.0;
            if label_x < width - 10.0 {
                frame.fill_text(canvas::Text {
                    content: format!("{}/{}", event.numerator, event.denominator),
                    position: Point::new(label_x, sig_y + 5.0),
                    color: if is_selected {
                        theme::ACCENT
                    } else {
                        theme::TEXT_DIM
                    },
                    size: 10.0.into(),
                    ..canvas::Text::default()
                });
            }
        }
    }

    /// Draw vertical bar and beat grid lines in the track area.
    /// Iterates bars using per-bar tempo and time-signature values so
    /// that grid spacing correctly follows tempo changes.
    pub(super) fn draw_grid_lines(
        &self,
        frame: &mut canvas::Frame,
        width: f32,
        ruler_height: f32,
        track_area_height: f32,
        _y_off: f32,
    ) {
        let line_height = track_area_height.max(600.0);
        let sr = self.sample_rate as f64;

        // Walk bars from 0, accumulating sample positions with interpolation.
        let mut sample_pos: f64 = 0.0;
        let mut cur_num = self
            .tempo_map
            .signature_points
            .first()
            .map(|e| e.numerator)
            .unwrap_or(4);
        let mut si: usize = if self.tempo_map.signature_points.first().map(|e| e.bar) == Some(0) {
            1
        } else {
            0
        };

        for bar in 0u32.. {
            while let Some(e) = self.tempo_map.signature_points.get(si) {
                if e.bar == bar {
                    cur_num = e.numerator;
                    si += 1;
                } else {
                    break;
                }
            }

            let cur_bpm = avg_bpm_for_bar(bar, &self.tempo_map.tempo_points);
            let samples_per_beat = sr * 60.0 / cur_bpm;
            let samples_per_bar = samples_per_beat * cur_num as f64;
            let bar_seconds = samples_per_bar / sr;
            let bar_pixel_width = bar_seconds as f32 * self.zoom;

            let x = (sample_pos / sr) as f32 * self.zoom - self.scroll_offset;

            // Past the right edge — done.
            if x > width + 1.0 {
                break;
            }
            // Safety limit.
            if bar > 20_000 {
                break;
            }

            // Bar step: skip bars for readability at low zoom.
            let bar_step = if bar_pixel_width < 20.0 {
                (20.0 / bar_pixel_width).ceil() as u32
            } else {
                1
            };
            let draw_this = bar_step <= 1 || bar % bar_step == 0;

            if draw_this && x >= -1.0 {
                frame.fill_rectangle(
                    Point::new(x, ruler_height),
                    Size::new(1.0, line_height),
                    theme::BAR_LINE,
                );

                // Beat lines within this bar.
                if bar_pixel_width >= 40.0 {
                    for beat in 1..cur_num {
                        let beat_sample = sample_pos + beat as f64 * samples_per_beat;
                        let bx = (beat_sample / sr) as f32 * self.zoom - self.scroll_offset;
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

            sample_pos += samples_per_bar;
        }
    }

    /// Draw the bar/beat ruler at the top.
    /// Uses per-bar tempo and time-signature values so bar numbers are
    /// positioned correctly when tempo changes.
    pub(super) fn draw_ruler(&self, frame: &mut canvas::Frame, width: f32, ruler_height: f32) {
        let sr = self.sample_rate as f64;

        let mut sample_pos: f64 = 0.0;
        let mut cur_num = self
            .tempo_map
            .signature_points
            .first()
            .map(|e| e.numerator)
            .unwrap_or(4);
        let mut si: usize = if self.tempo_map.signature_points.first().map(|e| e.bar) == Some(0) {
            1
        } else {
            0
        };

        for bar in 0u32.. {
            while let Some(e) = self.tempo_map.signature_points.get(si) {
                if e.bar == bar {
                    cur_num = e.numerator;
                    si += 1;
                } else {
                    break;
                }
            }

            let cur_bpm = avg_bpm_for_bar(bar, &self.tempo_map.tempo_points);
            let samples_per_beat = sr * 60.0 / cur_bpm;
            let samples_per_bar = samples_per_beat * cur_num as f64;
            let bar_seconds = samples_per_bar / sr;
            let bar_pixel_width = bar_seconds as f32 * self.zoom;

            let x = (sample_pos / sr) as f32 * self.zoom - self.scroll_offset;

            if x > width + 1.0 {
                break;
            }
            if bar > 20_000 {
                break;
            }

            let bar_step = if bar_pixel_width < 40.0 {
                (40.0 / bar_pixel_width).ceil() as u32
            } else {
                1
            };
            let draw_this = bar_step <= 1 || bar % bar_step == 0;

            if draw_this && x >= -1.0 {
                let bar_number = bar as i64 + 1; // 1-based

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
                    for beat in 1..cur_num {
                        let beat_sample = sample_pos + beat as f64 * samples_per_beat;
                        let bx = (beat_sample / sr) as f32 * self.zoom - self.scroll_offset;
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

            sample_pos += samples_per_bar;
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

        // Convert tick duration to samples integrating the tempo map.
        let clip_end_sample = self.tempo_map.tick_to_abs_sample(
            clip.start_sample,
            clip.duration_ticks,
            self.sample_rate,
        );
        let duration_samples = clip_end_sample.saturating_sub(clip.start_sample) as f64;
        let start_seconds = clip.start_sample as f32 / self.sample_rate as f32;
        let duration_seconds = duration_samples as f32 / self.sample_rate as f32;

        let x = start_seconds * self.zoom - self.scroll_offset;
        let w = duration_seconds * self.zoom;

        // Teal/cyan clip body
        let midi_body_color = Color::from_rgb(0.12, 0.22, 0.25);
        frame.fill_rectangle(Point::new(x, y), Size::new(w, clip_height), midi_body_color);

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
                    let note_start_in_clip = note.start_tick as f32 - clip.trim_start_ticks as f32;
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
