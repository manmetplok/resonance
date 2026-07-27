//! Global-tracks shelf: header strip, chord lane, tempo graph, and signature markers.
use iced::widget::canvas;
use iced::{Color, Point, Size};

use crate::state;
use crate::theme;
use super::TimelineCanvas;

impl TimelineCanvas<'_> {
    /// Draw the global-tracks shelf — a collapsible strip sitting between
    /// the ruler/section-band and the regular track lanes. The shelf has
    /// three parts:
    ///
    /// 1. **Header strip** (`GLOBAL_SHELF_HEADER_HEIGHT`, always visible)
    ///    — backdrop + one-line summary `6/8 · 90 BPM · B min · N chords`.
    ///    The caret-toggle + `GLOBAL` tag live on the track-header
    ///    column side (see `view::track_header`); the canvas only paints
    ///    the summary text on the right of the header strip.
    /// 2. **Chord lane** (`GLOBAL_TRACK_CHORD_HEIGHT`) — flattened view of
    ///    every section's chord progression, rendered as section tabs
    ///    with chord blocks underneath. Only painted when the shelf is
    ///    expanded.
    /// 3. **Tempo lane** (`GLOBAL_TRACK_TEMPO_HEIGHT`) — automation curve
    ///    with anchor points + BPM labels. Only painted when expanded.
    /// 4. **Signature lane** (`GLOBAL_TRACK_SIG_HEIGHT`) — pill markers
    ///    for time-signature changes + downbeat ticks. Only when expanded.
    pub(in crate::view::timeline) fn draw_global_tracks(
        &self,
        frame: &mut canvas::Frame,
        width: f32,
        ruler_height: f32,
    ) {
        let shelf_top = ruler_height;
        let header_h = theme::GLOBAL_SHELF_HEADER_HEIGHT;

        // ---- Header strip (always visible) ----
        // Slightly elevated backdrop so the shelf reads as a distinct
        // sub-section between the ruler and the lanes. Matches the
        // design's `linear-gradient(--bg-1 0%, #131419 100%)` by tinting
        // the lower edge a notch darker than the ruler.
        frame.fill_rectangle(
            Point::new(0.0, shelf_top),
            Size::new(width, header_h),
            theme::BG_1,
        );
        // Bottom hairline of the header strip.
        frame.fill_rectangle(
            Point::new(0.0, shelf_top + header_h - 1.0),
            Size::new(width, 1.0),
            theme::LINE_2,
        );

        // Right-side summary line: `6/8 · 90 BPM · B min · N chords`.
        // The text sits on the canvas side of the shelf header — the
        // caret + GLOBAL tag live in the track-header column. Padding
        // on the left so the text aligns with the lane content below.
        let summary = self.global_shelf_summary();
        let summary_y = shelf_top + (header_h - 12.0) * 0.5;
        frame.fill_text(canvas::Text {
            content: summary,
            position: Point::new(12.0, summary_y),
            color: theme::TEXT_2,
            size: 11.5.into(),
            font: theme::UI_FONT_MEDIUM,
            ..canvas::Text::default()
        });

        if !self.global_tracks_expanded {
            return;
        }

        let chord_h = theme::GLOBAL_TRACK_CHORD_HEIGHT;
        let tempo_h = theme::GLOBAL_TRACK_TEMPO_HEIGHT;
        let sig_h = theme::GLOBAL_TRACK_SIG_HEIGHT;

        let chord_y = shelf_top + header_h;
        let tempo_y = chord_y + chord_h;
        let sig_y = tempo_y + tempo_h;

        // ---- Chord lane background ----
        frame.fill_rectangle(
            Point::new(0.0, chord_y),
            Size::new(width, chord_h),
            theme::GLOBAL_TRACK_BG,
        );
        frame.fill_rectangle(
            Point::new(0.0, chord_y + chord_h - 1.0),
            Size::new(width, 1.0),
            theme::LINE_2,
        );

        // ---- Tempo row background ----
        frame.fill_rectangle(
            Point::new(0.0, tempo_y),
            Size::new(width, tempo_h),
            theme::GLOBAL_TRACK_BG,
        );
        frame.fill_rectangle(
            Point::new(0.0, tempo_y + tempo_h - 1.0),
            Size::new(width, 1.0),
            theme::LINE_2,
        );

        // ---- Time signature row background ----
        frame.fill_rectangle(
            Point::new(0.0, sig_y),
            Size::new(width, sig_h),
            theme::GLOBAL_TRACK_BG,
        );
        frame.fill_rectangle(
            Point::new(0.0, sig_y + sig_h - 1.0),
            Size::new(width, 1.0),
            theme::SEPARATOR,
        );

        // ---- Chord blocks per section ----
        self.draw_chord_lane(frame, width, chord_y, chord_h);

        // ---- Draw tempo line graph ----
        // (Geometry repurposed from the previous implementation; only the
        // row height shrank from 40 → 40, so the math is unchanged.)
        let row_h = tempo_h;
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

            // Draw connecting lines and filled area. Tempo events read in
            // the warm/amber accent matching the rest of the redesign's
            // "playhead / time" semantic; the dim variant softens the
            // filled area underneath the line.
            let line_color = Color {
                a: 0.7,
                ..theme::WARM
            };
            let fill_color = Color {
                a: 0.10,
                ..theme::WARM
            };

            // Build the polyline vertices: every tempo point, plus a
            // horizontal extension out to the right edge from the last
            // point so the fill/line reach the end of the canvas.
            // Previously this geometry was rasterised as hundreds of
            // 1 px-wide `fill_rectangle` calls per segment; one Path
            // fill + one Path stroke does the same work in two draw
            // submissions.
            let mut polyline: Vec<Point> = points
                .iter()
                .map(|&(x, y, _, _)| Point::new(x, y))
                .collect();
            if let Some(&last) = polyline.last() {
                if last.x < width {
                    polyline.push(Point::new(width, last.y));
                }
            }

            if polyline.len() >= 2 {
                // Filled trapezoid under the polyline: trace the
                // polyline left-to-right, then close along the bottom
                // (graph_bot) back to the starting x.
                let fill_path = canvas::Path::new(|b| {
                    let first = polyline[0];
                    b.move_to(Point::new(first.x, graph_bot));
                    for p in &polyline {
                        b.line_to(*p);
                    }
                    let last = polyline[polyline.len() - 1];
                    b.line_to(Point::new(last.x, graph_bot));
                    b.close();
                });
                frame.fill(&fill_path, fill_color);

                // Line itself. Round joins so the 2 px stroke doesn't
                // spike at sharp tempo changes (the previous overlapping
                // 1 px rect stack had no visible miter artifacts).
                let line_path = canvas::Path::new(|b| {
                    b.move_to(polyline[0]);
                    for p in &polyline[1..] {
                        b.line_to(*p);
                    }
                });
                frame.stroke(
                    &line_path,
                    canvas::Stroke::default()
                        .with_width(2.0)
                        .with_color(line_color)
                        .with_line_join(canvas::LineJoin::Round),
                );
            }

            // Draw event points (dots) and BPM labels.
            for (i, &(x, y, bpm, selected)) in points.iter().enumerate() {
                if x > width + 50.0 || x < -50.0 {
                    continue;
                }
                // Dot.
                let dot_r = if selected { 4.0 } else { 3.0 };
                let dot_color = if selected { theme::ACCENT } else { theme::WARM };
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
                        Color {
                            a: 0.30,
                            ..theme::WARM
                        }
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

            // Signature change blocks use the lavender accent at low alpha
            // so they're visible but don't compete with clips for attention.
            let block_color = if is_selected {
                theme::ACCENT_DIM
            } else {
                Color {
                    a: 0.08,
                    ..theme::ACCENT
                }
            };
            frame.fill_rectangle(
                Point::new(x.max(0.0), sig_y + 1.0),
                Size::new(block_w, sig_h - 2.0),
                block_color,
            );

            if x >= 0.0 {
                let marker_color = if is_selected {
                    theme::ACCENT
                } else {
                    theme::TEXT_DIM
                };
                frame.fill_rectangle(Point::new(x, sig_y), Size::new(1.0, sig_h), marker_color);
            }

            // Pill-style label `{n}/{d}`.
            let label_x = x.max(2.0) + 5.0;
            if label_x < width - 10.0 {
                let label = format!("{}/{}", event.numerator, event.denominator);
                let label_y = sig_y + (sig_h - 11.0) * 0.5;
                frame.fill_text(canvas::Text {
                    content: label.clone(),
                    position: Point::new(label_x, label_y),
                    color: if is_selected {
                        theme::ACCENT
                    } else {
                        theme::TEXT_1
                    },
                    size: 10.5.into(),
                    font: theme::MONO_FONT,
                    ..canvas::Text::default()
                });

                // Optional "compound · N eighths" hint for compound meters
                // (numerator divisible by 3 and >= 6, e.g. 6/8, 9/8, 12/8).
                if !is_selected
                    && event.numerator >= 6
                    && event.numerator % 3 == 0
                    && event.denominator == 8
                {
                    let hint_x = label_x + (label.len() as f32) * 6.5 + 10.0;
                    if hint_x < width - 60.0 {
                        frame.fill_text(canvas::Text {
                            content: format!("compound · {} eighths", event.numerator),
                            position: Point::new(hint_x, label_y + 1.0),
                            color: theme::TEXT_3,
                            size: 9.5.into(),
                            font: theme::MONO_FONT,
                            ..canvas::Text::default()
                        });
                    }
                }
            }
        }
    }

    /// Build the one-line "GLOBAL" summary text shown in the always-visible
    /// shelf header strip: `6/8 · 90 BPM · B min · N chords`.
    fn global_shelf_summary(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        let num = self.tempo_map.numerator;
        let den = self.tempo_map.denominator;
        let _ = write!(out, "{}/{}", num, den);

        let bpm = self.tempo_map.bpm;
        let _ = write!(out, "  ·  {} BPM", bpm.round() as u32);

        // Key signature: read from the first section that has a scale.
        if let Some(scale) = self
            .section_definitions
            .iter()
            .find_map(|d| d.scale.as_ref())
        {
            let mode_label = match scale.mode {
                resonance_music_theory::Mode::Major => "maj".to_string(),
                resonance_music_theory::Mode::Minor => "min".to_string(),
                other => other.to_string(),
            };
            let _ = write!(out, "  ·  {} {}", scale.root, mode_label);
        }

        // Chord count: sum of every section's progression.
        let chord_total: usize = self
            .section_definitions
            .iter()
            .map(|d| d.chords.len())
            .sum();
        let _ = write!(out, "  ·  {} chords", chord_total);
        out
    }
}
