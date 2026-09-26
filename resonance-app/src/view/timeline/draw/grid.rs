//! Bar/beat grid, ruler, and arrangement markers.
use iced::widget::canvas;
use iced::{Color, Point, Size};

use crate::theme;
use resonance_audio::types::{avg_bpm_for_bar, bar_len_quarters};
use super::TimelineCanvas;

/// One bar yielded by [`TimelineCanvas::for_each_visible_bar`]: its
/// index, on-screen x position, pixel width, and the timing context
/// needed to place beat subdivisions within the bar.
pub(super) struct VisibleBar {
    /// Zero-based bar index.
    bar: u32,
    /// X position of the bar start, in canvas pixels.
    x: f32,
    /// Width of this bar in pixels at the current zoom.
    pixel_width: f32,
    /// Time-signature numerator (beats per bar) active in this bar.
    numerator: u8,
    sample_pos: f64,
    /// Samples in one beat of *this bar's* signature — the bar's sample
    /// span divided by its numerator, so an eighth-note beat in 6/8 is
    /// half a quarter-note beat (ba todo #1389).
    samples_per_beat: f64,
    sr: f64,
    zoom: f32,
}

impl VisibleBar {
    /// X position of `beat` (1-based within the bar), in canvas pixels.
    fn beat_x(&self, beat: u8) -> f32 {
        let beat_sample = self.sample_pos + beat as f64 * self.samples_per_beat;
        (beat_sample / self.sr) as f32 * self.zoom
    }
}

impl TimelineCanvas<'_> {
    /// Walk bars left-to-right across the visible range, calling `f`
    /// once per bar that should be drawn. Bar positions follow per-bar
    /// tempo and time-signature values from the tempo map, so spacing
    /// correctly follows tempo changes.
    ///
    /// `min_bar_px` controls decimation at low zoom: when a bar is
    /// narrower than this, bars are skipped so the surviving lines stay
    /// at least `min_bar_px` apart (grid uses 20 px, ruler 40 px).
    pub(super) fn for_each_visible_bar(
        &self,
        width: f32,
        min_bar_px: f32,
        mut f: impl FnMut(&VisibleBar),
    ) {
        let sr = self.sample_rate as f64;

        // Walk bars from 0, accumulating sample positions with interpolation.
        let mut sample_pos: f64 = 0.0;
        let mut cur_num = self
            .tempo_map
            .signature_points
            .first()
            .map(|e| e.numerator)
            .unwrap_or(4);
        let mut cur_den = self
            .tempo_map
            .signature_points
            .first()
            .map(|e| e.denominator)
            .unwrap_or(4);
        let mut si: usize =
            if self.tempo_map.signature_points.first().map(|e| e.bar) == Some(0) {
                1
            } else {
                0
            };

        for bar in 0u32.. {
            while let Some(e) = self.tempo_map.signature_points.get(si) {
                if e.bar == bar {
                    cur_num = e.numerator;
                    cur_den = e.denominator;
                    si += 1;
                } else {
                    break;
                }
            }

            let cur_bpm = avg_bpm_for_bar(bar, &self.tempo_map.tempo_points);
            // BPM counts quarter notes; a bar is `bar_len_quarters` of
            // them (3.0 for 6/8, not 6.0). The beat spacing then follows
            // from the bar, so beat lines land on eighth notes in 6/8.
            let samples_per_quarter = sr * 60.0 / cur_bpm;
            let samples_per_bar = samples_per_quarter * bar_len_quarters(cur_num, cur_den);
            let samples_per_beat = if cur_num > 0 {
                samples_per_bar / cur_num as f64
            } else {
                samples_per_bar
            };
            let bar_seconds = samples_per_bar / sr;
            let bar_pixel_width = bar_seconds as f32 * self.zoom;

            let x = (sample_pos / sr) as f32 * self.zoom;

            // Past the right edge — done.
            if x > width + 1.0 {
                break;
            }
            // Safety limit.
            if bar > 20_000 {
                break;
            }

            // Bar step: skip bars for readability at low zoom.
            let bar_step = if bar_pixel_width < min_bar_px {
                (min_bar_px / bar_pixel_width).ceil() as u32
            } else {
                1
            };
            let draw_this = bar_step <= 1 || bar % bar_step == 0;

            if draw_this && x >= -1.0 {
                f(&VisibleBar {
                    bar,
                    x,
                    pixel_width: bar_pixel_width,
                    numerator: cur_num,
                    sample_pos,
                    samples_per_beat,
                    sr,
                    zoom: self.zoom,
                });
            }

            sample_pos += samples_per_bar;
        }
    }

    /// Draw vertical bar and beat grid lines in the track area.
    /// Iterates bars using per-bar tempo and time-signature values so
    /// that grid spacing correctly follows tempo changes.
    pub(in crate::view::timeline) fn draw_grid_lines(
        &self,
        frame: &mut canvas::Frame,
        width: f32,
        ruler_height: f32,
        track_area_height: f32,
        _y_off: f32,
    ) {
        let line_height = track_area_height.max(600.0);

        self.for_each_visible_bar(width, 20.0, |bar| {
            frame.fill_rectangle(
                Point::new(bar.x, ruler_height),
                Size::new(1.0, line_height),
                theme::BAR_LINE,
            );

            // Beat lines within this bar.
            if bar.pixel_width >= 40.0 {
                for beat in 1..bar.numerator {
                    let bx = bar.beat_x(beat);
                    if bx >= 0.0 && bx <= width {
                        frame.fill_rectangle(
                            Point::new(bx, ruler_height),
                            Size::new(1.0, line_height),
                            theme::BEAT_LINE,
                        );
                    }
                }
            }
        });
    }

    /// Render arrangement markers in the ruler band: point markers as a
    /// colour-tinted flag (pole + swallow-tailed pennant + name label),
    /// ranged markers as a translucent labelled span with start/end edge
    /// lines. Deliberately distinct from the amber loop range (which fills
    /// the ruler with centred triangle handles) and the rounded Compose
    /// section pills (which sit in their own band below the ruler).
    ///
    /// The selected marker (if any) gets the stronger accent: a full-opacity
    /// pole, an outlined flag, and a bright `TEXT_1` label.
    pub(in crate::view::timeline) fn draw_markers(
        &self,
        frame: &mut canvas::Frame,
        width: f32,
        ruler_height: f32,
    ) {
        const FLAG_W: f32 = 11.0;
        const FLAG_H: f32 = 9.0;

        for marker in self.markers {
            let start_x = self.sample_to_x(marker.start_sample);
            let end_x = marker.end_sample.map(|e| self.sample_to_x(e));

            // Cull markers wholly off either edge (a region's right edge or
            // a point's own x).
            let right_edge = end_x.unwrap_or(start_x);
            if right_edge < 0.0 || start_x > width {
                continue;
            }

            let is_selected = self.selected_marker_id == Some(marker.id);
            let color = Color::from_rgb(
                marker.color[0] as f32 / 255.0,
                marker.color[1] as f32 / 255.0,
                marker.color[2] as f32 / 255.0,
            );

            // Ranged region: translucent fill across the ruler + an edge
            // line at the end so the span's extent reads clearly.
            if let Some(end_x) = end_x {
                let span_x = start_x.max(0.0);
                let span_w = (end_x.min(width) - span_x).max(0.0);
                if span_w > 0.0 {
                    frame.fill_rectangle(
                        Point::new(span_x, 0.0),
                        Size::new(span_w, ruler_height),
                        Color {
                            a: if is_selected { 0.26 } else { 0.16 },
                            ..color
                        },
                    );
                }
                if end_x >= 0.0 && end_x <= width {
                    frame.fill_rectangle(
                        Point::new(end_x - 0.5, 0.0),
                        Size::new(1.0, ruler_height),
                        Color { a: 0.7, ..color },
                    );
                }
            }

            // Start pole — the flag's mast, shared by point and ranged
            // markers so a region also gets a clear start handle.
            if start_x >= 0.0 && start_x <= width {
                frame.fill_rectangle(
                    Point::new(start_x - 0.5, 0.0),
                    Size::new(1.0, ruler_height),
                    if is_selected {
                        color
                    } else {
                        Color { a: 0.8, ..color }
                    },
                );
            }

            // Flag pennant at the top of the pole.
            if start_x <= width && start_x + FLAG_W >= 0.0 {
                let fx = start_x;
                let flag = canvas::Path::new(|b| {
                    b.move_to(Point::new(fx, 0.0));
                    b.line_to(Point::new(fx + FLAG_W, 0.0));
                    b.line_to(Point::new(fx + FLAG_W - 3.0, FLAG_H * 0.5));
                    b.line_to(Point::new(fx + FLAG_W, FLAG_H));
                    b.line_to(Point::new(fx, FLAG_H));
                    b.close();
                });
                frame.fill(&flag, color);
                if is_selected {
                    frame.stroke(
                        &flag,
                        canvas::Stroke::default()
                            .with_width(1.0)
                            .with_color(theme::TEXT_1),
                    );
                }
            }

            // Name label, just right of the flag near the top of the ruler.
            let label_x = start_x.max(0.0) + FLAG_W + 4.0;
            if label_x < width - 6.0 {
                frame.fill_text(canvas::Text {
                    content: crate::util::short_with(&marker.name, 18, "..."),
                    position: Point::new(label_x, 1.0),
                    color: if is_selected { theme::TEXT_1 } else { color },
                    size: 10.0.into(),
                    font: theme::UI_FONT_SEMIBOLD,
                    ..canvas::Text::default()
                });
            }
        }
    }

    /// Draw the bar/beat ruler at the top.
    /// Uses per-bar tempo and time-signature values so bar numbers are
    /// positioned correctly when tempo changes.
    pub(in crate::view::timeline) fn draw_ruler(
        &self,
        frame: &mut canvas::Frame,
        width: f32,
        ruler_height: f32,
    ) {
        self.for_each_visible_bar(width, 40.0, |bar| {
            let bar_number = bar.bar as i64 + 1; // 1-based

            // Major tick (bar)
            frame.fill_rectangle(
                Point::new(bar.x, ruler_height - 12.0),
                Size::new(1.0, 12.0),
                theme::TEXT_DIM,
            );

            // Bar number label
            frame.fill_text(canvas::Text {
                content: format!("{}", bar_number),
                position: Point::new(bar.x + 3.0, ruler_height - 24.0),
                color: theme::TEXT_DIM,
                size: 11.0.into(),
                ..canvas::Text::default()
            });

            // Beat ticks within bar (only if enough space)
            if bar.pixel_width >= 40.0 {
                for beat in 1..bar.numerator {
                    let bx = bar.beat_x(beat);
                    if bx >= 0.0 && bx <= width {
                        frame.fill_rectangle(
                            Point::new(bx, ruler_height - 6.0),
                            Size::new(1.0, 6.0),
                            Color::from_rgb(0.25, 0.25, 0.25),
                        );
                    }
                }
            }
        });

        // Ruler bottom line
        frame.fill_rectangle(
            Point::new(0.0, ruler_height - 1.0),
            Size::new(width, 1.0),
            theme::SEPARATOR,
        );
    }
}
