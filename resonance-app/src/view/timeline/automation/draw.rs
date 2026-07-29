//! Automation-lane drawing on the [`TimelineCanvas`]: the cached static
//! lane layer (value axis, envelope segments, breakpoint dots, parameter
//! chip) and the uncached live playhead-value overlay.
//!
//! Draws a track's parameter-automation lane as an overlay band in the
//! lower region of the track row — or, for an expanded track (doc #256,
//! todo #1097), one full band per lane inside dedicated 44 px sub-rows.

use iced::widget::canvas;
use iced::{Color, Point, Rectangle, Size};

use crate::theme;
use crate::view::arrange_layout::ArrangeRowLayout;
use resonance_common::{lane_value_to_real, sample_lane, AutomationLane};

use super::super::TimelineCanvas;
use super::geometry::CHIP_PAD_X;
use super::{
    automation_band, format_real_value, lane_chip_rect, lane_row_band, lane_segments_polyline,
    shown_lane_for_track, target_label, track_lanes_sorted, value_to_y,
};

/// Radius of a breakpoint dot.
const DOT_RADIUS: f32 = 3.5;
/// Radius of the live playhead value indicator.
const LIVE_DOT_RADIUS: f32 = 4.0;

impl TimelineCanvas<'_> {
    /// Draw the static automation layer for every visible track. Called from
    /// the cached `draw_into` pass, after clips, inside the lane clip. Lane
    /// Y / height come from the shared [`ArrangeRowLayout`] (doc #203), so
    /// bands sit correctly under the mixed 60/96 px row pitch and tracks
    /// hidden inside a collapsed group (`track_row_rect` -> `None`) draw
    /// nothing — mirroring clip behaviour.
    pub(in crate::view::timeline) fn draw_automation_lanes(
        &self,
        frame: &mut canvas::Frame,
        layout: &ArrangeRowLayout,
        header_height: f32,
        y_off: f32,
        bounds: Rectangle,
    ) {
        if self.automation.lanes.is_empty() {
            return;
        }
        for track in self.visible_tracks_sorted() {
            let lanes = track_lanes_sorted(self.automation, track);
            if lanes.is_empty() {
                continue;
            }

            // Expanded track (doc #256, todo #1097): every lane draws as a
            // full band inside its own dedicated sub-row, and the in-track
            // overlay band below is suppressed entirely. A track hidden
            // inside a collapsed group emits no lane rows
            // (`automation_row_rect` -> `None`), so nothing draws — the
            // sub-rows vanish with their track.
            if self.automation_expanded_tracks.contains(&track.id) {
                for lane in lanes {
                    let Some((row_y_top, row_height)) =
                        layout.automation_row_rect(track.id, lane.id)
                    else {
                        continue;
                    };
                    let row_y = header_height + row_y_top - y_off;
                    if row_y + row_height < header_height || row_y > bounds.height {
                        continue;
                    }
                    let (band_top, band_height) = lane_row_band(row_y, row_height);
                    // `lane_count` 1: a dedicated row shows exactly one
                    // lane, so it gets the plain label — no clickable
                    // cycle chip, no "N lanes" count.
                    self.draw_one_automation_lane(
                        frame,
                        lane,
                        1,
                        band_top,
                        band_height,
                        bounds.width,
                    );
                }
                continue;
            }

            // Collapsed track: the pre-#1097 overlay band, including the
            // selector chip when several lanes target the track.
            let Some((row_y_top, row_height)) = layout.track_row_rect(track.id) else {
                continue;
            };
            let row_y = header_height + row_y_top - y_off;
            if row_y + row_height < header_height || row_y > bounds.height {
                continue;
            }
            let Some(lane) = shown_lane_for_track(self.automation, track) else {
                continue;
            };
            let (band_top, band_height) = automation_band(row_y, row_height);
            self.draw_one_automation_lane(
                frame,
                lane,
                lanes.len(),
                band_top,
                band_height,
                bounds.width,
            );
        }
    }

    /// Draw a single lane's axis, segments and breakpoints within the
    /// given value band. `lane_count` is how many lanes the band can show:
    /// when more than one, the label chip gets a background (it is
    /// clickable — a click cycles which lane shows, todo #1095) and a
    /// small "N lanes" count so the hidden lanes are discoverable.
    /// Single-lane bands render the plain label. Callers derive the band
    /// from their row rect — [`automation_band`] for the in-track overlay,
    /// [`lane_row_band`] for a dedicated automation sub-row (todo #1097).
    fn draw_one_automation_lane(
        &self,
        frame: &mut canvas::Frame,
        lane: &AutomationLane,
        lane_count: usize,
        band_top: f32,
        band_height: f32,
        width: f32,
    ) {
        // Read-disabled lanes keep their points but use the static value, so
        // dim them to read as "not playing back".
        let dim = if lane.enabled { 1.0 } else { 0.45 };

        // ---- Value axis: faint top / mid / bottom guides ----
        for (frac, alpha) in [(0.0_f32, 0.12_f32), (0.5, 0.16), (1.0, 0.12)] {
            let y = value_to_y(frac, band_top, band_height);
            frame.fill_rectangle(
                Point::new(0.0, y - 0.5),
                Size::new(width, 1.0),
                Color { a: alpha, ..theme::LINE },
            );
        }

        // ---- Parameter label chip (top-left of the band) ----
        let label = target_label(&lane.target, &self.device_param_labels);
        if !label.is_empty() {
            let chip = lane_chip_rect(band_top, &label);
            if lane_count > 1 {
                // The chip is clickable (cycles the shown lane), so give
                // it a faint plate the pointer can visibly target. The
                // fill uses the exact hit-test rect (#732 rule).
                let plate = canvas::Path::rounded_rectangle(
                    Point::new(chip.x, chip.y),
                    Size::new(chip.width, chip.height),
                    3.0.into(),
                );
                frame.fill(
                    &plate,
                    Color {
                        a: 0.35 * dim,
                        ..theme::LINE
                    },
                );
            }
            frame.fill_text(canvas::Text {
                content: label,
                position: Point::new(chip.x + CHIP_PAD_X, band_top - 13.0),
                color: Color {
                    a: 0.85 * dim,
                    ..theme::TEXT_DIM
                },
                size: 9.0.into(),
                ..canvas::Text::default()
            });
            if lane_count > 1 {
                // Hidden-lane count, just right of the chip.
                frame.fill_text(canvas::Text {
                    content: format!("{lane_count} lanes"),
                    position: Point::new(chip.x + chip.width + 4.0, band_top - 13.0),
                    color: Color {
                        a: 0.6 * dim,
                        ..theme::TEXT_DIM
                    },
                    size: 9.0.into(),
                    ..canvas::Text::default()
                });
            }
        }

        let line_color = Color {
            a: dim,
            ..theme::ACCENT_SOFT
        };
        let dot_color = Color {
            a: dim,
            ..theme::ACCENT
        };

        // ---- Connecting segments ----
        // Build one polyline that flat-leads from the left edge, draws each
        // segment (Linear = diagonal to the next point, Stepped = flat hold
        // then a vertical step at the next point's x), and flat-leads out to
        // the right edge — mirroring `sample_lane`'s clamp-at-ends behaviour.
        if !lane.points.is_empty() {
            let poly = lane_segments_polyline(
                &lane.points,
                0.0,
                width,
                band_top,
                band_height,
                |frames| self.sample_to_x(frames),
            );

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
                        .with_color(line_color)
                        .with_line_join(canvas::LineJoin::Round),
                );
            }

            // ---- Breakpoint dots ----
            for p in &lane.points {
                let px = self.sample_to_x(p.time_frames);
                if px < -DOT_RADIUS || px > width + DOT_RADIUS {
                    continue;
                }
                let py = value_to_y(p.value, band_top, band_height);
                let dot = canvas::Path::circle(Point::new(px, py), DOT_RADIUS);
                frame.fill(&dot, dot_color);
            }
        }
    }

    /// Draw the live automated-value indicators for every visible track.
    /// Called from the uncached `draw_overlay_into` pass so the dots track the
    /// playhead every frame without invalidating the cached lane geometry.
    pub(in crate::view::timeline) fn draw_automation_live_values(
        &self,
        frame: &mut canvas::Frame,
        layout: &ArrangeRowLayout,
        header_height: f32,
        y_off: f32,
        bounds: Rectangle,
    ) {
        if self.automation.lanes.is_empty() {
            return;
        }
        let playhead_x = self.sample_to_x(self.playhead);
        if playhead_x < 0.0 || playhead_x > bounds.width {
            return;
        }
        let lane_clip = Rectangle {
            x: 0.0,
            y: header_height,
            width: bounds.width,
            height: (bounds.height - header_height).max(0.0),
        };
        frame.with_clip(lane_clip, |frame| {
            for track in self.visible_tracks_sorted() {
                // Expanded track (todo #1097): one live dot per lane, each
                // riding its own dedicated sub-row; the suppressed in-track
                // overlay draws no dot at all — matching the static pass.
                if self.automation_expanded_tracks.contains(&track.id) {
                    for lane in track_lanes_sorted(self.automation, track) {
                        if lane.points.is_empty() {
                            continue;
                        }
                        let Some((row_y_top, row_height)) =
                            layout.automation_row_rect(track.id, lane.id)
                        else {
                            continue;
                        };
                        let row_y = header_height + row_y_top - y_off;
                        if row_y + row_height < header_height || row_y > bounds.height {
                            continue;
                        }
                        let (band_top, band_height) = lane_row_band(row_y, row_height);
                        self.draw_live_value_dot(frame, lane, playhead_x, band_top, band_height);
                    }
                    continue;
                }

                let Some((row_y_top, row_height)) = layout.track_row_rect(track.id) else {
                    continue;
                };
                let row_y = header_height + row_y_top - y_off;
                if row_y + row_height < header_height || row_y > bounds.height {
                    continue;
                }
                let Some(lane) = shown_lane_for_track(self.automation, track) else {
                    continue;
                };
                if lane.points.is_empty() {
                    continue;
                }
                let (band_top, band_height) = automation_band(row_y, row_height);
                self.draw_live_value_dot(frame, lane, playhead_x, band_top, band_height);
            }
        });
    }

    /// One lane's live playhead-value indicator: the warm dot on the
    /// envelope plus the real-value read-out just right of it. Shared by
    /// the in-track overlay band and the dedicated lane rows (todo #1097).
    fn draw_live_value_dot(
        &self,
        frame: &mut canvas::Frame,
        lane: &AutomationLane,
        playhead_x: f32,
        band_top: f32,
        band_height: f32,
    ) {
        let value = sample_lane(&lane.points, self.playhead);
        let py = value_to_y(value, band_top, band_height);

        // Warm dot tying the read-out to the playhead colour.
        let dot = canvas::Path::circle(Point::new(playhead_x, py), LIVE_DOT_RADIUS);
        frame.fill(&dot, theme::WARM);
        let ring = canvas::Path::circle(Point::new(playhead_x, py), LIVE_DOT_RADIUS);
        frame.stroke(
            &ring,
            canvas::Stroke::default().with_width(1.0).with_color(Color {
                a: 0.9,
                ..theme::BG_1
            }),
        );

        // Real-value read-out (dB / pan / 0-1) just right of the dot.
        let real = lane_value_to_real(&lane.target, value);
        let text = format_real_value(lane.target.clone(), real);
        frame.fill_text(canvas::Text {
            content: text,
            position: Point::new(playhead_x + 7.0, py - 5.0),
            color: theme::WARM,
            size: 9.0.into(),
            ..canvas::Text::default()
        });
    }
}
