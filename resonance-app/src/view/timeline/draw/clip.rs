//! Audio clip drawing: body, fades, handles, waveform, and crossfades.
use iced::widget::canvas;
use iced::{Color, Point, Size};

use crate::state::ClipState;
use crate::theme;
use crate::view::arrange_layout::ArrangeRowLayout;
use resonance_audio::types::FadeCurve;
use super::TimelineCanvas;
use super::chrome::{
    ClipChrome, clip_lane_rect, FADE_WEDGE_COLOR, gain_tinted_body, overlap_range,
    fade_envelope, fade_wedge_path, stroke_polyline, draw_bead, draw_clip_hatch,
    draw_clip_gain_tag, draw_crossfade_badge,
};

impl TimelineCanvas<'_> {
    pub(in crate::view::timeline) fn draw_clip(
        &self,
        frame: &mut canvas::Frame,
        clip: &ClipState,
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

        let start_seconds = clip.start_sample as f32 / self.sample_rate as f32;
        let duration_seconds = clip.duration_samples as f32 / self.sample_rate as f32;

        let x = start_seconds * self.zoom - self.scroll_offset + indent;
        let w = duration_seconds * self.zoom;
        if w <= 0.0 {
            return;
        }

        // Audio clips: warm/amber wash + tinted border. Name text in
        // WARM so the kind reads at a glance. A frozen / rendered track
        // has no editable sample source — its clips are "unsupported":
        // diagonal hatch, no fade handles (gain still applies). Design #153.
        let is_selected = self.selected_clip == Some(clip.id);
        let fadeable = !self.frozen_tracks.contains(&clip.track_id);
        let chrome = ClipChrome {
            x,
            y,
            w,
            h: clip_height,
            // Clip-gain tint: louder brightens the warm wash, quieter
            // darkens it, so level reads at a glance (design #153).
            body_color: gain_tinted_body(clip.gain_db),
            border_color: if is_selected {
                theme::ACCENT
            } else {
                Color {
                    a: 0.32,
                    ..theme::WARM
                }
            },
            is_selected,
            name: &clip.name,
            name_color: theme::WARM,
            show_name: x + 6.0 < x + w,
        };

        chrome.draw(frame, |frame| {
            self.draw_clip_waveform(frame, clip, x, y, w, clip_height);
            if fadeable {
                self.draw_clip_fades(frame, clip, x, y, w, clip_height);
            } else {
                draw_clip_hatch(frame, x, y, w, clip_height);
            }
        });

        // Overlays drawn on top of the border: the fade-handle / gain
        // beads and the mono dB header tag.
        self.draw_clip_handles(frame, clip, x, y, w, clip_height, fadeable, is_selected);
        draw_clip_gain_tag(frame, clip, x, y, w);
    }

    /// Fade-in / fade-out ramps: a darkened wedge over the attenuated
    /// region (so the waveform under it reads as faded) plus a warm ramp
    /// line tracing the chosen curve. Drawn inside the clip body, over
    /// the waveform and under the name / border.
    fn draw_clip_fades(
        &self,
        frame: &mut canvas::Frame,
        clip: &ClipState,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) {
        let sr = self.sample_rate as f32;
        let fade_in_w = ((clip.fade_in_frames as f32 / sr) * self.zoom).clamp(0.0, w);
        let fade_out_w = ((clip.fade_out_frames as f32 / sr) * self.zoom).clamp(0.0, w);

        if fade_in_w > 0.5 {
            let env = fade_envelope(clip.fade_in_curve, x, fade_in_w, y, h, true);
            frame.fill(&fade_wedge_path(&env, x, fade_in_w, y), FADE_WEDGE_COLOR);
            stroke_polyline(frame, &env, theme::WARM, 1.5);
        }
        if fade_out_w > 0.5 {
            let x0 = x + w - fade_out_w;
            let env = fade_envelope(clip.fade_out_curve, x0, fade_out_w, y, h, false);
            frame.fill(&fade_wedge_path(&env, x0, fade_out_w, y), FADE_WEDGE_COLOR);
            stroke_polyline(frame, &env, theme::WARM, 1.5);
        }
    }

    /// Circular fade-handle beads on the two top corners (warm) and the
    /// clip-gain bead at top-centre (lavender). Fade beads ride inward
    /// as the fade grows (`handle x = ramp end`); they stay hidden on a
    /// clean clip until it is selected, but are always shown once a fade
    /// exists. Gain is available even on frozen clips. Design #153.
    #[allow(clippy::too_many_arguments)]
    fn draw_clip_handles(
        &self,
        frame: &mut canvas::Frame,
        clip: &ClipState,
        x: f32,
        y: f32,
        w: f32,
        _h: f32,
        fadeable: bool,
        is_selected: bool,
    ) {
        let sr = self.sample_rate as f32;
        let right = x + w;

        if fadeable {
            if clip.fade_in_frames > 0 || is_selected {
                let bx =
                    (x + (clip.fade_in_frames as f32 / sr) * self.zoom).clamp(x, right);
                draw_bead(frame, bx, y, theme::WARM);
            }
            if clip.fade_out_frames > 0 || is_selected {
                let bx =
                    (right - (clip.fade_out_frames as f32 / sr) * self.zoom).clamp(x, right);
                draw_bead(frame, bx, y, theme::WARM);
            }
        }

        // Gain bead at top-centre. Shown once gain departs unity, or on
        // selection so an untouched clip stays clean but discoverable.
        if clip.gain_db.abs() > 0.05 || is_selected {
            draw_bead(frame, x + w / 2.0, y, theme::ACCENT);
        }
    }

    /// Waveform — warm-tinted bars on top of the wash.
    fn draw_clip_waveform(
        &self,
        frame: &mut canvas::Frame,
        clip: &ClipState,
        x: f32,
        y: f32,
        w: f32,
        clip_height: f32,
    ) {
        let header_height = 18.0;
        if !clip.waveform_peaks.is_empty() {
            let wave_y = y + header_height;
            let wave_h = clip_height - header_height - 4.0;
            let wave_center = wave_y + wave_h * 0.5;

            let peak_frames = resonance_audio::types::WAVEFORM_PEAK_FRAMES as f32;
            let seconds_per_peak = peak_frames / self.sample_rate as f32;
            let pixels_per_peak = seconds_per_peak * self.zoom;

            let trim_start_peaks = clip.trim_start_frames as f32 / peak_frames;

            let waveform_color = Color {
                a: 0.7,
                ..theme::WARM
            };

            // Only the columns inside the cull window are tessellated
            // (review VIEW-28). The walk keeps its original stepping so
            // every drawn bar lands on exactly the same x as before.
            let (lo, hi) = crate::view::timeline::cull::clip_px_range(x, w, self.cull_window());
            let start_px = (-x).max(0.0);
            let mut px = start_px;
            while px < w {
                if px > hi {
                    break;
                }
                if px + pixels_per_peak.max(1.0) < lo {
                    px += pixels_per_peak.max(1.0);
                    continue;
                }
                let peak_idx_f = trim_start_peaks + px / pixels_per_peak;
                let peak_idx = peak_idx_f as usize;
                if peak_idx >= clip.waveform_peaks.len() {
                    break;
                }
                let (min_val, max_val) = clip.waveform_peaks[peak_idx];

                let draw_x = x + px;
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
    }

    /// Automatic crossfades: wherever two audio clips on the same track
    /// overlap, the seam gets a lavender overlap wash, two crossing
    /// equal-power curves (left clip fading out, right clip fading in),
    /// and an `⤬` badge. Crossfade is derived, never stored — overlap
    /// implies crossfade regardless of the clips' manual fades (design
    /// #153 / arch #156).
    pub(in crate::view::timeline) fn draw_crossfades(
        &self,
        frame: &mut canvas::Frame,
        layout: &ArrangeRowLayout,
        ruler_height: f32,
        y_off: f32,
        visible_height: f32,
    ) {
        // Candidate pairs come from a per-track sweep (review VIEW-28),
        // in the same (i, j) order the old all-pairs walk used.
        let spans: Vec<(u64, u64, u64)> = self
            .clips
            .iter()
            .map(|c| {
                (
                    c.track_id,
                    c.start_sample,
                    c.start_sample.saturating_add(c.duration_samples),
                )
            })
            .collect();
        let cull = self.cull_window();
        for (i, j) in crate::view::timeline::cull::overlapping_pairs(&spans) {
            let a = &self.clips[i];
            let b = &self.clips[j];
            let Some((ov_start, ov_end)) = overlap_range(
                a.start_sample,
                a.duration_samples,
                b.start_sample,
                b.duration_samples,
            ) else {
                continue;
            };
            let Some((y, h, indent)) = clip_lane_rect(
                self,
                a.track_id,
                layout,
                ruler_height,
                y_off,
                visible_height,
            ) else {
                continue;
            };

            let x0 = self.sample_to_x(ov_start) + indent;
            let x1 = self.sample_to_x(ov_end) + indent;
            let ow = x1 - x0;
            if ow <= 0.5 {
                continue;
            }
            if !crate::view::timeline::cull::span_may_be_visible((x0, x1), cull) {
                continue;
            }

            // Lavender overlap wash.
            frame.fill_rectangle(
                Point::new(x0, y),
                Size::new(ow, h),
                Color {
                    a: 0.16,
                    ..theme::ACCENT
                },
            );

            // Crossing equal-power curves: the earlier clip fades out
            // across the overlap, the later clip fades in. Their sum
            // is constant power — a click-free seam. Equal-power is
            // symmetric, so the same pair of curves serves either
            // ordering of the overlapping clips.
            let fade_out = fade_envelope(FadeCurve::EqualPower, x0, ow, y, h, false);
            let fade_in = fade_envelope(FadeCurve::EqualPower, x0, ow, y, h, true);
            stroke_polyline(frame, &fade_out, theme::ACCENT_SOFT, 1.5);
            stroke_polyline(frame, &fade_in, theme::ACCENT_SOFT, 1.5);

            // `⤬` badge centred at the top of the overlap.
            draw_crossfade_badge(frame, x0 + ow / 2.0, y + 9.0);
        }
    }
}
