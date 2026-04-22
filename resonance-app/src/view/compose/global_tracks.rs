//! Cropped global tracks (tempo + time signature) for the Compose view.
//!
//! Shows a read-only slice of the global tempo and signature tracks,
//! restricted to the section's bar range. The coordinate system is
//! tick-proportional (same as the other compose canvases) so bar widths
//! are determined by time signature, not tempo.

use iced::widget::canvas::{self, Frame, Geometry};
use iced::widget::Canvas;
use iced::{mouse, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use resonance_audio::types::{TempoMap, TICKS_PER_QUARTER_NOTE};

use crate::message::Message;
use crate::theme;

/// Height of each global track row in the compose view.
const ROW_HEIGHT: f32 = 28.0;
/// Total height of the global tracks area (tempo + signature).
pub const GLOBAL_TRACKS_HEIGHT: f32 = ROW_HEIGHT * 2.0;

pub fn view<'a>(
    tempo_map: &'a TempoMap,
    start_bar: u32,
    section_length_bars: u32,
) -> Element<'a, Message> {
    Canvas::new(ComposeGlobalTracksCanvas {
        tempo_map,
        start_bar,
        section_length_bars,
    })
    .width(Length::Fill)
    .height(Length::Fixed(GLOBAL_TRACKS_HEIGHT))
    .into()
}

struct ComposeGlobalTracksCanvas<'a> {
    tempo_map: &'a TempoMap,
    start_bar: u32,
    section_length_bars: u32,
}

impl<'a> ComposeGlobalTracksCanvas<'a> {
    fn section_total_ticks(&self) -> u64 {
        (0..self.section_length_bars)
            .map(|b| {
                self.tempo_map.numerator_at_bar(self.start_bar + b) as u64
                    * TICKS_PER_QUARTER_NOTE
            })
            .sum()
    }

    /// Map a section-relative tick to pixel x.
    fn tick_to_x(&self, tick: f64, width: f32) -> f32 {
        let total = self.section_total_ticks() as f64;
        if total <= 0.0 {
            return 0.0;
        }
        (tick / total * width as f64) as f32
    }

    /// Convert a bar number to a section-relative tick position.
    fn bar_to_section_tick(&self, bar: u32) -> f64 {
        let mut tick: f64 = 0.0;
        let end = bar.min(self.start_bar + self.section_length_bars);
        if bar > self.start_bar {
            for b in self.start_bar..end {
                tick +=
                    self.tempo_map.numerator_at_bar(b) as f64 * TICKS_PER_QUARTER_NOTE as f64;
            }
        } else if bar < self.start_bar {
            for b in bar..self.start_bar {
                tick -=
                    self.tempo_map.numerator_at_bar(b) as f64 * TICKS_PER_QUARTER_NOTE as f64;
            }
        }
        tick
    }
}

impl<'a> canvas::Program<Message> for ComposeGlobalTracksCanvas<'a> {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let width = bounds.width;
        let section_end_bar = self.start_bar + self.section_length_bars;

        // ---- Tempo row background ----
        frame.fill_rectangle(
            Point::ORIGIN,
            Size::new(width, ROW_HEIGHT),
            theme::GLOBAL_TRACK_BG,
        );
        frame.fill_rectangle(
            Point::new(0.0, ROW_HEIGHT - 1.0),
            Size::new(width, 1.0),
            theme::SEPARATOR,
        );

        // ---- Signature row background ----
        frame.fill_rectangle(
            Point::new(0.0, ROW_HEIGHT),
            Size::new(width, ROW_HEIGHT),
            theme::GLOBAL_TRACK_BG,
        );
        frame.fill_rectangle(
            Point::new(0.0, ROW_HEIGHT * 2.0 - 1.0),
            Size::new(width, 1.0),
            theme::SEPARATOR,
        );

        // ---- Draw tempo line graph ----
        if !self.tempo_map.tempo_points.is_empty() {
            self.draw_tempo_row(&mut frame, width);
        }

        // ---- Draw signature event markers ----
        self.draw_signature_row(&mut frame, width, section_end_bar);

        vec![frame.into_geometry()]
    }

    fn update(
        &self,
        _state: &mut Self::State,
        _event: canvas::Event,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        // Read-only: no interaction in the compose view.
        (canvas::event::Status::Ignored, None)
    }
}

impl<'a> ComposeGlobalTracksCanvas<'a> {
    fn draw_tempo_row(&self, frame: &mut Frame, width: f32) {
        // Collect tempo points that fall within the section range
        // (plus one on each side for proper edge interpolation).
        let section_end_bar = self.start_bar + self.section_length_bars;

        // Determine BPM range for vertical mapping across visible points.
        let mut min_bpm = f32::MAX;
        let mut max_bpm = f32::MIN;
        for e in &self.tempo_map.tempo_points {
            if e.bar + 1 >= self.start_bar && e.bar <= section_end_bar {
                min_bpm = min_bpm.min(e.bpm);
                max_bpm = max_bpm.max(e.bpm);
            }
        }
        // Include the interpolated BPM at the section boundaries.
        let start_bpm =
            resonance_audio::types::bpm_at_bar(self.start_bar as f64, &self.tempo_map.tempo_points)
                as f32;
        let end_bpm = resonance_audio::types::bpm_at_bar(
            section_end_bar as f64,
            &self.tempo_map.tempo_points,
        ) as f32;
        min_bpm = min_bpm.min(start_bpm).min(end_bpm);
        max_bpm = max_bpm.max(start_bpm).max(end_bpm);

        let range = (max_bpm - min_bpm).max(10.0);
        let pad = range * 0.15;
        let lo = min_bpm - pad;
        let hi = max_bpm + pad;

        let graph_top = 3.0;
        let graph_bot = ROW_HEIGHT - 3.0;
        let graph_h = graph_bot - graph_top;

        let bpm_to_y = |bpm: f32| -> f32 { graph_bot - ((bpm - lo) / (hi - lo)) * graph_h };

        // Build points: start with the section start edge, then each tempo
        // event within range, then the section end edge.
        let mut points: Vec<(f32, f32, f32)> = Vec::new(); // (x, y, bpm)

        // Left edge
        points.push((0.0, bpm_to_y(start_bpm), start_bpm));

        // Tempo events inside the section
        for e in &self.tempo_map.tempo_points {
            if e.bar <= self.start_bar || e.bar >= section_end_bar {
                continue;
            }
            let tick = self.bar_to_section_tick(e.bar);
            let x = self.tick_to_x(tick, width);
            let y = bpm_to_y(e.bpm);
            points.push((x, y, e.bpm));
        }

        // Right edge
        points.push((width, bpm_to_y(end_bpm), end_bpm));

        // Draw connecting lines and filled area.
        let line_color = Color::from_rgba(0.9, 0.55, 0.15, 0.7);
        let fill_color = Color::from_rgba(0.9, 0.55, 0.15, 0.10);

        for pair in points.windows(2) {
            let (x1, y1, _) = pair[0];
            let (x2, y2, _) = pair[1];
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
            // Line itself
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

        // Draw dots and BPM labels for events inside the section.
        for e in &self.tempo_map.tempo_points {
            if e.bar <= self.start_bar || e.bar >= section_end_bar {
                continue;
            }
            let tick = self.bar_to_section_tick(e.bar);
            let x = self.tick_to_x(tick, width);
            let y = bpm_to_y(e.bpm);

            // Dot
            let dot_r = 3.0;
            let dot_color = Color::from_rgb(0.9, 0.55, 0.15);
            frame.fill_rectangle(
                Point::new(x - dot_r, y - dot_r),
                Size::new(dot_r * 2.0, dot_r * 2.0),
                dot_color,
            );
            // Vertical marker
            frame.fill_rectangle(
                Point::new(x, 0.0),
                Size::new(1.0, ROW_HEIGHT),
                Color::from_rgba(0.9, 0.55, 0.15, 0.3),
            );
            // BPM label
            let label_x = x + 5.0;
            if label_x < width - 10.0 {
                frame.fill_text(canvas::Text {
                    content: format!("{:.0}", e.bpm),
                    position: Point::new(label_x, 2.0),
                    color: theme::TEXT_DIM,
                    size: 10.0.into(),
                    ..canvas::Text::default()
                });
            }
        }

        // BPM label at the left edge
        frame.fill_text(canvas::Text {
            content: format!("{:.0} bpm", start_bpm),
            position: Point::new(3.0, 2.0),
            color: theme::TEXT_DIM,
            size: 10.0.into(),
            ..canvas::Text::default()
        });
    }

    fn draw_signature_row(&self, frame: &mut Frame, width: f32, section_end_bar: u32) {
        let sig_y = ROW_HEIGHT;

        // Find the active signature at or before the section start.
        // Walk all signature events and find blocks that overlap the section.
        let sig_points = &self.tempo_map.signature_points;

        // Collect events relevant to the section: the last event at or before
        // start_bar, plus any events within the section.
        let mut visible: Vec<(u32, u8, u8)> = Vec::new(); // (bar, num, den)

        // Find the signature active at section start (last event <= start_bar).
        let mut active_num = self.tempo_map.numerator;
        let mut active_den = self.tempo_map.denominator;
        let mut active_bar = 0u32;
        for e in sig_points {
            if e.bar <= self.start_bar {
                active_num = e.numerator;
                active_den = e.denominator;
                active_bar = e.bar;
            }
        }
        // The block that covers section start
        visible.push((active_bar, active_num, active_den));

        // Events within the section (excluding start_bar since it's covered).
        for e in sig_points {
            if e.bar > self.start_bar && e.bar < section_end_bar {
                visible.push((e.bar, e.numerator, e.denominator));
            }
        }

        for (i, &(bar, num, den)) in visible.iter().enumerate() {
            // Block x-start: clamp to section start
            let block_start_bar = bar.max(self.start_bar);
            let block_start_tick = self.bar_to_section_tick(block_start_bar);
            let x = self.tick_to_x(block_start_tick, width);

            // Block x-end: next event or section end
            let next_x = if let Some(&(next_bar, _, _)) = visible.get(i + 1) {
                let next_tick = self.bar_to_section_tick(next_bar);
                self.tick_to_x(next_tick, width)
            } else {
                width
            };

            let block_w = (next_x - x).max(2.0);

            let block_color = Color::from_rgba(0.3, 0.6, 0.9, 0.12);
            frame.fill_rectangle(
                Point::new(x, sig_y + 1.0),
                Size::new(block_w, ROW_HEIGHT - 2.0),
                block_color,
            );

            // Left edge marker (skip for the first block if it starts before the section)
            if bar >= self.start_bar && bar > visible.first().map(|v| v.0).unwrap_or(0) {
                frame.fill_rectangle(
                    Point::new(x, sig_y),
                    Size::new(1.0, ROW_HEIGHT),
                    theme::TEXT_DIM,
                );
            }

            // Label
            let label_x = x + 3.0;
            if label_x < width - 10.0 {
                frame.fill_text(canvas::Text {
                    content: format!("{}/{}", num, den),
                    position: Point::new(label_x, sig_y + 5.0),
                    color: theme::TEXT_DIM,
                    size: 10.0.into(),
                    ..canvas::Text::default()
                });
            }
        }
    }
}
