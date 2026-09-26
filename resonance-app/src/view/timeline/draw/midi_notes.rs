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

        let is_selected = self.selected_midi_clip == Some(clip.id);

        // Frozen tracks render their cached audio, so the lane switches
        // from the live MIDI language (lavender notes) to the audio render
        // language: a warm waveform silhouette overlaid with the frost
        // wash and relabelled "frozen render" (design doc #181).
        if self.frozen_tracks.contains(&clip.track_id) {
            let chrome = ClipChrome {
                x,
                y,
                w,
                h: clip_height,
                // Warm wash body — the audio-domain language — so the
                // frozen render reads as rendered audio, not live MIDI.
                body_color: Color {
                    a: 0.10,
                    ..theme::WARM
                },
                // Frost edge marks the clip as frozen; selection still wins.
                border_color: if is_selected {
                    theme::ACCENT
                } else {
                    theme::FROST_EDGE
                },
                is_selected,
                name: "frozen render",
                name_color: theme::WARM,
                show_name: true,
            };
            chrome.draw(frame, |frame| {
                self.draw_frozen_render_waveform(frame, clip, x, y, w, clip_height);
                // Frost wash laid over the warm waveform so the render
                // reads as frozen — the one cool tone the palette admits.
                let wash = canvas::Path::rounded_rectangle(
                    Point::new(x, y),
                    Size::new(w, clip_height),
                    8.0.into(),
                );
                frame.fill(&wash, theme::FROST_WASH);
            });
            return;
        }

        // MIDI clips: lavender wash + lavender border, name in lavender
        // accent.
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

    /// Synthesise a warm "rendered audio" waveform silhouette for a frozen
    /// MIDI clip. The freeze cache's real peaks aren't carried in the view
    /// model ([`FreezeCacheRef`](resonance_common::FreezeCacheRef) holds
    /// only metadata), so the silhouette is derived deterministically from
    /// the clip's notes: audio is drawn only where a note actually sounds,
    /// with a stable per-column amplitude so the same clip always renders
    /// the same shape (golden-snapshot safe — no float trig, no RNG).
    fn draw_frozen_render_waveform(
        &self,
        frame: &mut canvas::Frame,
        clip: &MidiClipState,
        x: f32,
        y: f32,
        w: f32,
        clip_height: f32,
    ) {
        let header_height = 18.0;
        let wave_y = y + header_height;
        let wave_h = clip_height - header_height - 4.0;
        if wave_h <= 2.0 || w <= 2.0 || clip.notes.is_empty() {
            return;
        }
        let wave_center = wave_y + wave_h * 0.5;
        let total_ticks = clip.duration_ticks as f32;
        if total_ticks <= 0.0 {
            return;
        }

        // Warm bars, slightly translucent, matching the live-audio waveform
        // language (`draw_clip_waveform`).
        let waveform_color = Color {
            a: 0.7,
            ..theme::WARM
        };

        // Coverage is answered by a sweep over the sorted notes rather than
        // a scan of every note per column, and only the columns inside the
        // cull window are visited for drawing (review VIEW-28).
        let mut coverage = crate::view::timeline::cull::CoverageSweep::new(
            clip.notes
                .iter()
                .map(|n| {
                    let s = n.start_tick as f32 - clip.trim_start_ticks as f32;
                    (s, s + n.duration_ticks as f32)
                })
                .collect(),
        );
        let (lo, hi) = crate::view::timeline::cull::clip_px_range(x, w, self.cull_window());
        let start_px = (-x).max(0.0);
        let mut px = start_px;
        while px < w {
            if px > hi {
                break;
            }
            if px + 1.0 < lo {
                px += 1.0;
                continue;
            }
            // Clip-space tick under this column.
            let tick = (px / w) * total_ticks;
            // Audio only where a note sounds (coverage gate).
            if coverage.covers(tick) {
                // Deterministic per-column amplitude in [0.25, 0.95] from an
                // integer hash of the column index — looks like dense
                // rendered audio without any platform-dependent maths.
                let i = px as u32;
                let h = i.wrapping_mul(2_246_822_519) ^ (i >> 3).wrapping_mul(3_266_489_917);
                let r = (h & 0xffff) as f32 / 65_535.0;
                let amp = 0.25 + 0.70 * r;
                let half = amp * wave_h * 0.5;
                let draw_x = x + px;
                frame.fill_rectangle(
                    Point::new(draw_x, wave_center - half),
                    Size::new(1.0, half * 2.0),
                    waveform_color,
                );
            }
            px += 1.0;
        }
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
            let (lo, hi) = crate::view::timeline::cull::clip_px_range(x, w, self.cull_window());
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
                    // Off-screen notes are not tessellated (review VIEW-28).
                    if nx + nw.max(1.0) < x + lo || nx > x + hi {
                        continue;
                    }

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
