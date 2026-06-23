//! Timeline automation-lane rendering (architecture doc #162 §3, epic #14,
//! todo #381 / A3).
//!
//! Draws a track's parameter-automation lane as an overlay band in the lower
//! region of the track row: a value axis, the breakpoints, the connecting
//! segments (straight for `Linear`, right-angle holds for `Stepped`), and —
//! in the uncached overlay pass — a live indicator of the automated value
//! under the playhead.
//!
//! This file owns only **rendering**. Breakpoint hit-testing / editing is
//! todo #382 (A4); the parameter picker, Read toggle and fader/knob tint are
//! todo #383 (A5). The lane "shows" whenever the track has a lane in
//! [`AutomationState`] and "hides" when it has none — no extra UI state, so a
//! lane added via the picker simply appears and a cleared lane disappears.
//!
//! View-performance rules (MEMORY ui-work §11): the static lane layer (axis,
//! segments, dots) is drawn inside the cached geometry pass and only repaints
//! when [`super::TimelineFingerprint::automation_hash`] changes (an edit). The
//! live playhead value rides the uncached overlay frame so it follows the
//! playhead without invalidating the rest of the timeline.

use iced::widget::canvas;
use iced::{Color, Point, Rectangle, Size};

use crate::state::TrackState;
use crate::theme;
use resonance_common::{
    lane_value_to_real, sample_lane, AutomationLane, AutomationTarget, Breakpoint, CurveKind,
};

use super::TimelineCanvas;

/// Vertical inset of the value band from the track row's top/bottom edges.
/// Larger than [`theme::CLIP_LANE_INSET`] so the envelope sits clear of the
/// clip name label that hugs the top of the row.
const BAND_INSET: f32 = 18.0;
/// Radius of a breakpoint dot.
const DOT_RADIUS: f32 = 3.5;
/// Radius of the live playhead value indicator.
const LIVE_DOT_RADIUS: f32 = 4.0;

/// The value-band rect (top-left + height) inside a track row whose top edge is
/// `row_y`. Normalized lane value `1.0` maps to `band_top`, `0.0` to the band
/// bottom (axis grows downward like the rest of the canvas).
pub fn automation_band(row_y: f32) -> (f32, f32) {
    let band_top = row_y + BAND_INSET;
    let band_height = (theme::TRACK_HEIGHT - 2.0 * BAND_INSET).max(1.0);
    (band_top, band_height)
}

/// Map a normalized lane value (`0.0..=1.0`) to a y pixel inside the band.
pub fn value_to_y(value: f32, band_top: f32, band_height: f32) -> f32 {
    band_top + (1.0 - value.clamp(0.0, 1.0)) * band_height
}

/// Build the envelope polyline for a sorted breakpoint list within a value
/// band of `[left, right]` x range. The line flat-leads in from `left` at the
/// first value, draws each segment — `Linear` as a diagonal to the next point,
/// `Stepped` as a flat hold then a vertical step at the next point's x — and
/// flat-leads out to `right` at the last value, mirroring [`sample_lane`]'s
/// clamp-at-ends behaviour. Returns an empty vec when there are no points.
///
/// `x_of` maps a breakpoint's `time_frames` to a pixel x (the canvas's
/// `sample_to_x`). Pure so the Linear/Stepped geometry is unit-testable.
pub fn lane_segments_polyline(
    points: &[Breakpoint],
    left: f32,
    right: f32,
    band_top: f32,
    band_height: f32,
    x_of: impl Fn(u64) -> f32,
) -> Vec<Point> {
    if points.is_empty() {
        return Vec::new();
    }
    let y_of = |v: f32| value_to_y(v, band_top, band_height);
    let mut poly: Vec<Point> = Vec::with_capacity(points.len() * 2 + 2);
    poly.push(Point::new(left, y_of(points[0].value)));
    for (idx, p) in points.iter().enumerate() {
        poly.push(Point::new(x_of(p.time_frames), y_of(p.value)));
        if idx + 1 < points.len() && p.curve == CurveKind::Stepped {
            // Hold this value flat to the next breakpoint's x; the next push
            // then draws the vertical step up/down.
            poly.push(Point::new(x_of(points[idx + 1].time_frames), y_of(p.value)));
        }
    }
    let last = &points[points.len() - 1];
    poly.push(Point::new(right, y_of(last.value)));
    poly
}

/// Priority used to pick the single lane drawn for a track when several
/// targets it (gain, then pan, then mute, then the lowest plugin-param id).
/// The parameter picker (#383) will let the user override which lane shows;
/// until then this gives a stable, predictable choice.
pub fn target_priority(target: &AutomationTarget) -> u32 {
    match target {
        AutomationTarget::TrackGain(_) => 0,
        AutomationTarget::TrackPan(_) => 1,
        AutomationTarget::TrackMute(_) => 2,
        AutomationTarget::PluginParam { param_id, .. } => 10u32.saturating_add(*param_id),
        // Bus/master targets never belong to an arrange track row.
        _ => u32::MAX,
    }
}

/// Short human label for a lane's target, shown as a chip on the band.
fn target_label(target: &AutomationTarget) -> String {
    match target {
        AutomationTarget::TrackGain(_) => "Volume".to_string(),
        AutomationTarget::TrackPan(_) => "Pan".to_string(),
        AutomationTarget::TrackMute(_) => "Mute".to_string(),
        AutomationTarget::PluginParam { param_id, .. } => format!("Param {param_id}"),
        _ => String::new(),
    }
}

impl TimelineCanvas<'_> {
    /// Whether `target` drives one of `track`'s parameters (its own gain/pan/
    /// mute, or a CLAP param on a plugin instance hosted by the track).
    fn target_belongs_to_track(&self, target: &AutomationTarget, track: &TrackState) -> bool {
        match target {
            AutomationTarget::TrackGain(id)
            | AutomationTarget::TrackPan(id)
            | AutomationTarget::TrackMute(id) => *id == track.id,
            AutomationTarget::PluginParam { instance, .. } => {
                track.plugins.iter().any(|p| p.instance_id == *instance)
            }
            _ => false,
        }
    }

    /// The lane drawn for `track`, if any — the highest-priority target that
    /// belongs to the track. `None` hides the lane (the common case: tracks
    /// have no automation).
    pub(super) fn primary_lane_for_track<'l>(
        &self,
        automation: &'l crate::state::AutomationState,
        track: &TrackState,
    ) -> Option<&'l AutomationLane> {
        automation
            .lanes
            .values()
            .filter(|lane| self.target_belongs_to_track(&lane.target, track))
            .min_by_key(|lane| target_priority(&lane.target))
    }

    /// Draw the static automation layer for every visible track. Called from
    /// the cached `draw_into` pass, after clips, inside the lane clip.
    pub(super) fn draw_automation_lanes(
        &self,
        frame: &mut canvas::Frame,
        sorted_tracks: &[&TrackState],
        header_height: f32,
        y_off: f32,
        bounds: Rectangle,
    ) {
        if self.automation.lanes.is_empty() {
            return;
        }
        for (i, track) in sorted_tracks.iter().enumerate() {
            let row_y = header_height + i as f32 * theme::TRACK_HEIGHT - y_off;
            if row_y + theme::TRACK_HEIGHT < header_height || row_y > bounds.height {
                continue;
            }
            let Some(lane) = self.primary_lane_for_track(self.automation, track) else {
                continue;
            };
            self.draw_one_automation_lane(frame, lane, row_y, bounds.width);
        }
    }

    /// Draw a single lane's axis, segments and breakpoints within its row.
    fn draw_one_automation_lane(
        &self,
        frame: &mut canvas::Frame,
        lane: &AutomationLane,
        row_y: f32,
        width: f32,
    ) {
        let (band_top, band_height) = automation_band(row_y);
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
        let label = target_label(&lane.target);
        if !label.is_empty() {
            frame.fill_text(canvas::Text {
                content: label,
                position: Point::new(4.0, band_top - 13.0),
                color: Color {
                    a: 0.85 * dim,
                    ..theme::TEXT_DIM
                },
                size: 9.0.into(),
                ..canvas::Text::default()
            });
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
    pub(super) fn draw_automation_live_values(
        &self,
        frame: &mut canvas::Frame,
        sorted_tracks: &[&TrackState],
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
            for (i, track) in sorted_tracks.iter().enumerate() {
                let row_y = header_height + i as f32 * theme::TRACK_HEIGHT - y_off;
                if row_y + theme::TRACK_HEIGHT < header_height || row_y > bounds.height {
                    continue;
                }
                let Some(lane) = self.primary_lane_for_track(self.automation, track) else {
                    continue;
                };
                if lane.points.is_empty() {
                    continue;
                }
                let (band_top, band_height) = automation_band(row_y);
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
                let text = format_real_value(&lane.target, real);
                frame.fill_text(canvas::Text {
                    content: text,
                    position: Point::new(playhead_x + 7.0, py - 5.0),
                    color: theme::WARM,
                    size: 9.0.into(),
                    ..canvas::Text::default()
                });
            }
        });
    }
}

/// Format a target's real value for the live read-out: dB for gain, a signed
/// pan position, On/Off for mute, two decimals for a plugin param.
pub fn format_real_value(target: &AutomationTarget, real: f32) -> String {
    match target {
        AutomationTarget::TrackGain(_)
        | AutomationTarget::BusGain(_)
        | AutomationTarget::MasterGain => format!("{real:+.1} dB"),
        AutomationTarget::TrackPan(_) | AutomationTarget::BusPan(_) => {
            if real.abs() < 0.01 {
                "C".to_string()
            } else if real < 0.0 {
                format!("L{:.0}", real.abs() * 100.0)
            } else {
                format!("R{:.0}", real * 100.0)
            }
        }
        AutomationTarget::TrackMute(_) | AutomationTarget::BusMute(_) => {
            if real >= 0.5 {
                "On".to_string()
            } else {
                "Off".to_string()
            }
        }
        AutomationTarget::PluginParam { .. } | AutomationTarget::DeviceParam { .. } => {
            format!("{real:.2}")
        }
    }
}
