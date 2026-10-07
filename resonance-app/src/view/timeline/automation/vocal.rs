//! Drawing a vocal track's expression-curve rows (`Vocal: Dynamics |
//! Tension | Breathiness | Pitch bend`, vocal-mcp-control-and-quality.md
//! §10) on the [`TimelineCanvas`].
//!
//! A curve belongs to a section, so the row is only live at the
//! placements of sections with a vocal lane on the track: each placement
//! gets a raised band carrying that section's curve, and the row outside
//! every placement is dimmed (and inert — the input side refuses it, see
//! `input.rs`). A placement whose voicebank does not render the curve is
//! greyed and labelled "not supported by <bank>". The hit-testing reads
//! the same [`VocalLane`] windows, so what is drawn live is exactly what
//! takes a click (the #732 rule).

use iced::widget::canvas;
use iced::{Color, Point, Size};

use crate::compose::VocalLane;
use crate::theme;

use super::super::TimelineCanvas;
use super::geometry::CHIP_PAD_X;
use super::{lane_chip_rect, lane_segments_polyline, target_label, value_to_y};

/// Radius of a curve point.
const DOT_RADIUS: f32 = 3.5;

impl TimelineCanvas<'_> {
    /// Draw one vocal curve row inside its band.
    pub(super) fn draw_vocal_curve_row(
        &self,
        frame: &mut canvas::Frame,
        vocal: &VocalLane,
        row_y: f32,
        row_height: f32,
        band_top: f32,
        band_height: f32,
        width: f32,
    ) {
        // Dim the whole row; placements are lifted out of it below.
        frame.fill_rectangle(
            Point::new(0.0, row_y),
            Size::new(width, row_height - 1.0),
            Color {
                a: 0.6,
                ..theme::BG_1
            },
        );

        for window in &vocal.windows {
            let x0 = self.sample_to_x(window.start_frame).max(0.0);
            let x1 = self.sample_to_x(window.end_frame).min(width);
            if x1 <= x0 {
                continue;
            }
            let fill = if window.supported {
                theme::BG_3
            } else {
                theme::BG_2
            };
            frame.fill_rectangle(
                Point::new(x0, row_y),
                Size::new(x1 - x0, row_height - 1.0),
                fill,
            );
            // Placement edges.
            for x in [x0, x1 - 1.0] {
                frame.fill_rectangle(
                    Point::new(x, row_y),
                    Size::new(1.0, row_height - 1.0),
                    theme::LINE,
                );
            }
            // Mid guide (the neutral value) inside the placement.
            let mid = value_to_y(0.5, band_top, band_height);
            frame.fill_rectangle(
                Point::new(x0, mid - 0.5),
                Size::new(x1 - x0, 1.0),
                Color {
                    a: 0.16,
                    ..theme::LINE
                },
            );
            if !window.supported {
                frame.fill_text(canvas::Text {
                    content: format!("not supported by {}", window.bank),
                    position: Point::new(x0 + 6.0, band_top + band_height / 2.0 - 5.0),
                    color: Color {
                        a: 0.7,
                        ..theme::TEXT_3
                    },
                    size: 9.0.into(),
                    ..canvas::Text::default()
                });
            }

            let points: Vec<resonance_common::Breakpoint> = vocal
                .lane
                .points
                .iter()
                .zip(&vocal.origins)
                .filter(|(_, o)| o.placement_id == window.placement_id)
                .map(|(p, _)| *p)
                .collect();
            if points.is_empty() {
                continue;
            }
            let (line, dot) = if window.supported {
                (theme::ACCENT_SOFT, theme::ACCENT)
            } else {
                (
                    Color {
                        a: 0.45,
                        ..theme::TEXT_3
                    },
                    Color {
                        a: 0.55,
                        ..theme::TEXT_3
                    },
                )
            };
            let poly = lane_segments_polyline(&points, x0, x1, band_top, band_height, |f| {
                self.sample_to_x(f)
            });
            if poly.len() >= 2 {
                let path = canvas::Path::new(|b| {
                    b.move_to(poly[0]);
                    for pt in &poly[1..] {
                        b.line_to(*pt);
                    }
                });
                frame.stroke(
                    &path,
                    canvas::Stroke::default()
                        .with_width(1.5)
                        .with_color(line)
                        .with_line_join(canvas::LineJoin::Round),
                );
            }
            for p in &points {
                let px = self.sample_to_x(p.time_frames);
                if px < -DOT_RADIUS || px > width + DOT_RADIUS {
                    continue;
                }
                let py = value_to_y(p.value, band_top, band_height);
                frame.fill(&canvas::Path::circle(Point::new(px, py), DOT_RADIUS), dot);
            }
        }

        // Row label, top-left, like an engine lane row's.
        let label = target_label(&vocal.lane.target, &self.device_param_labels);
        let chip = lane_chip_rect(band_top, &label);
        frame.fill_text(canvas::Text {
            content: label,
            position: Point::new(chip.x + CHIP_PAD_X, band_top - 13.0),
            color: Color {
                a: 0.85,
                ..theme::TEXT_DIM
            },
            size: 9.0.into(),
            ..canvas::Text::default()
        });
    }
}

/// An order-independent hash of every vocal curve row's points and
/// placement windows, folded into the timeline's `automation_hash` so a
/// curve edit, a moved placement or a bank switch repaints the cached
/// lane layer.
pub fn vocal_rows_hash(rows: &crate::compose::VocalLanes) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut acc = 0_u64;
    for vocal in rows.iter() {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        vocal.lane.id.hash(&mut h);
        for (p, o) in vocal.lane.points.iter().zip(&vocal.origins) {
            (p.time_frames, p.value.to_bits(), p.curve, o.placement_id).hash(&mut h);
        }
        for w in &vocal.windows {
            (w.start_frame, w.end_frame, w.supported, &w.bank).hash(&mut h);
        }
        acc ^= h.finish();
    }
    acc
}
