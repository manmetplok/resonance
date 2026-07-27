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

use std::collections::HashMap;

use iced::widget::canvas;
use iced::{Color, Point, Rectangle, Size};

use crate::state::TrackState;
use crate::theme;
use crate::view::arrange_layout::ArrangeRowLayout;
use resonance_common::{
    lane_value_to_real, sample_lane, AutomationLane, AutomationTarget, Breakpoint, CurveKind,
};

use super::snap::snap_sample_to_grid_tempo;
use super::TimelineCanvas;

/// Vertical inset of the value band from the track row's top/bottom edges.
/// Larger than [`theme::CLIP_LANE_INSET`] so the envelope sits clear of the
/// clip name label that hugs the top of the row.
const BAND_INSET: f32 = 18.0;
/// Radius of a breakpoint dot.
const DOT_RADIUS: f32 = 3.5;
/// Radius of the live playhead value indicator.
const LIVE_DOT_RADIUS: f32 = 4.0;
/// Pixel pick radius for clicking a breakpoint dot (todo #382). A touch
/// larger than [`DOT_RADIUS`] so the small dots are easy to grab.
pub const BREAKPOINT_HIT_RADIUS: f32 = 7.0;

/// The value-band rect (top-left + height) inside a track row whose top edge
/// is `row_y` and whose height is `row_height`. Normalized lane value `1.0`
/// maps to `band_top`, `0.0` to the band bottom (axis grows downward like the
/// rest of the canvas). The band is derived from the row's *actual* height —
/// rows come from the shared [`ArrangeRowLayout`] (doc #203), not a fixed
/// `TRACK_HEIGHT` pitch.
pub fn automation_band(row_y: f32, row_height: f32) -> (f32, f32) {
    let band_top = row_y + BAND_INSET;
    let band_height = (row_height - 2.0 * BAND_INSET).max(1.0);
    (band_top, band_height)
}

/// Map a normalized lane value (`0.0..=1.0`) to a y pixel inside the band.
pub fn value_to_y(value: f32, band_top: f32, band_height: f32) -> f32 {
    band_top + (1.0 - value.clamp(0.0, 1.0)) * band_height
}

/// Inverse of [`value_to_y`]: map a pixel y inside the band back to a
/// normalized lane value, clamped to `0.0..=1.0`. A zero-height band maps
/// everything to `0.0` so the division is safe.
pub fn value_from_y(y: f32, band_top: f32, band_height: f32) -> f32 {
    if band_height <= 0.0 {
        return 0.0;
    }
    (1.0 - (y - band_top) / band_height).clamp(0.0, 1.0)
}

/// Index of the breakpoint nearest `pos` within `hit_radius` pixels, or
/// `None` on a miss. Pure (the breakpoint→x mapping is passed as `x_of`)
/// so the pick geometry is unit-testable without a live canvas; ties break
/// toward the closest dot.
pub fn nearest_breakpoint(
    points: &[Breakpoint],
    pos: Point,
    band_top: f32,
    band_height: f32,
    x_of: impl Fn(u64) -> f32,
    hit_radius: f32,
) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (idx, p) in points.iter().enumerate() {
        let dx = pos.x - x_of(p.time_frames);
        let dy = pos.y - value_to_y(p.value, band_top, band_height);
        let dist2 = dx * dx + dy * dy;
        if dist2 <= hit_radius * hit_radius && best.is_none_or(|(_, b)| dist2 < b) {
            best = Some((idx, dist2));
        }
    }
    best.map(|(idx, _)| idx)
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
/// targets it (gain, then pan, then mute, then device params, then the lowest
/// plugin-param id). The parameter picker (#383) will let the user override
/// which lane shows; until then this gives a stable, predictable choice.
/// Mirrors the mixer strip's `priority` (view/mixer/automation.rs) so the
/// strip header and the timeline band agree on which lane is "primary".
pub fn target_priority(target: AutomationTarget) -> u32 {
    match target {
        AutomationTarget::TrackGain(_) => 0,
        AutomationTarget::TrackPan(_) => 1,
        AutomationTarget::TrackMute(_) => 2,
        // Device params are what the user automated deliberately on an
        // external-instrument track, so they outrank generic plugin params.
        // Ties between several device lanes are broken deterministically by
        // `param_id` in [`primary_lane_for_track`].
        AutomationTarget::DeviceParam { .. } => 5,
        AutomationTarget::PluginParam { param_id, .. } => 10u32.saturating_add(param_id),
        // Bus/master targets never belong to an arrange track row.
        _ => u32::MAX,
    }
}

/// Short human label for a lane's target, shown as a chip on the band.
/// `DeviceParam` lanes resolve through `device_labels` (built once per view
/// pass by [`device_param_labels`]), falling back to the raw param id when the
/// preset no longer resolves (e.g. it changed while the lane survives).
pub fn target_label(
    target: &AutomationTarget,
    device_labels: &HashMap<AutomationTarget, String>,
) -> String {
    match target {
        AutomationTarget::TrackGain(_) => "Volume".to_string(),
        AutomationTarget::TrackPan(_) => "Pan".to_string(),
        AutomationTarget::TrackMute(_) => "Mute".to_string(),
        AutomationTarget::PluginParam { param_id, .. } => format!("Param {param_id}"),
        AutomationTarget::DeviceParam { param_id, .. } => device_labels
            .get(target)
            .cloned()
            .unwrap_or_else(|| param_id.clone()),
        _ => String::new(),
    }
}

/// Resolved display names for every `DeviceParam` lane in `automation`, keyed
/// by the lane's target: the track's selected external-instrument device
/// preset is looked up in the definition registry and the param id resolved to
/// its named [`resonance_common::DeviceParam::name`] — the same resolution the
/// mixer strip's picker uses (view/mixer/automation.rs). Built at view-model
/// build time (timeline_panel.rs) so the canvas needs no registry access;
/// exposed for reuse by the track-header work (doc #256). Unresolvable lanes
/// are simply absent — [`target_label`] falls back to the raw param id.
pub fn device_param_labels(
    automation: &crate::state::AutomationState,
    external_instruments: &crate::state::ExternalInstrumentMap,
    registry: &resonance_common::DeviceDefinitionRegistry,
) -> HashMap<AutomationTarget, String> {
    automation
        .lanes
        .keys()
        .filter_map(|target| {
            let AutomationTarget::DeviceParam { track, param_id } = target else {
                return None;
            };
            let def_id = external_instruments.get(track)?.device_id.as_deref()?;
            let name = registry.get(def_id)?.param(param_id)?.name.clone();
            Some((target.clone(), name))
        })
        .collect()
}

/// Whether `target` drives one of `track`'s parameters: its own gain/pan/
/// mute, a CLAP param on a plugin instance hosted by the track, or a device
/// param on the track's external instrument.
pub fn target_belongs_to_track(target: &AutomationTarget, track: &TrackState) -> bool {
    match target {
        AutomationTarget::TrackGain(id)
        | AutomationTarget::TrackPan(id)
        | AutomationTarget::TrackMute(id) => *id == track.id,
        AutomationTarget::PluginParam { instance, .. } => {
            track.plugins.iter().any(|p| p.instance_id == *instance)
        }
        AutomationTarget::DeviceParam { track: id, .. } => *id == track.id,
        _ => false,
    }
}

/// The lane drawn for `track`, if any — the highest-priority target that
/// belongs to the track, with ties (several device-param lanes share one
/// priority tier) broken by the lexicographically smallest `param_id` so the
/// pick is deterministic. `None` hides the lane (the common case: tracks have
/// no automation).
pub fn primary_lane_for_track<'l>(
    automation: &'l crate::state::AutomationState,
    track: &TrackState,
) -> Option<&'l AutomationLane> {
    automation
        .lanes
        .values()
        .filter(|lane| target_belongs_to_track(&lane.target, track))
        .min_by_key(|lane: &&'l AutomationLane| {
            // Copy the inner `&'l` ref out so the tie-break `&str` borrows
            // from the lane itself, not the closure-local double reference.
            let lane: &'l AutomationLane = lane;
            let tie = match &lane.target {
                AutomationTarget::DeviceParam { param_id, .. } => param_id.as_str(),
                _ => "",
            };
            (target_priority(lane.target.clone()), tie)
        })
}

impl TimelineCanvas<'_> {
    /// Draw the static automation layer for every visible track. Called from
    /// the cached `draw_into` pass, after clips, inside the lane clip. Lane
    /// Y / height come from the shared [`ArrangeRowLayout`] (doc #203), so
    /// bands sit correctly under the mixed 60/96 px row pitch and tracks
    /// hidden inside a collapsed group (`track_row_rect` -> `None`) draw
    /// nothing — mirroring clip behaviour.
    pub(super) fn draw_automation_lanes(
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
            let Some((row_y_top, row_height)) = layout.track_row_rect(track.id) else {
                continue;
            };
            let row_y = header_height + row_y_top - y_off;
            if row_y + row_height < header_height || row_y > bounds.height {
                continue;
            }
            let Some(lane) = primary_lane_for_track(self.automation, track) else {
                continue;
            };
            self.draw_one_automation_lane(frame, lane, row_y, row_height, bounds.width);
        }
    }

    /// Draw a single lane's axis, segments and breakpoints within its row.
    fn draw_one_automation_lane(
        &self,
        frame: &mut canvas::Frame,
        lane: &AutomationLane,
        row_y: f32,
        row_height: f32,
        width: f32,
    ) {
        let (band_top, band_height) = automation_band(row_y, row_height);
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
                let Some((row_y_top, row_height)) = layout.track_row_rect(track.id) else {
                    continue;
                };
                let row_y = header_height + row_y_top - y_off;
                if row_y + row_height < header_height || row_y > bounds.height {
                    continue;
                }
                let Some(lane) = primary_lane_for_track(self.automation, track) else {
                    continue;
                };
                if lane.points.is_empty() {
                    continue;
                }
                let (band_top, band_height) = automation_band(row_y, row_height);
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
        });
    }
}

/// A pointer hit on a breakpoint dot: which lane drives it and the index of
/// the hit point within the lane's time-sorted point list.
#[derive(Debug, Clone)]
pub(crate) struct BreakpointHit {
    pub target: AutomationTarget,
    pub index: usize,
}

/// Breakpoint hit-testing and edit-geometry for the timeline canvas
/// (todo #382 / A4). These translate a pointer position into the lane,
/// breakpoint and value the input handlers in [`super::input`] act on.
impl TimelineCanvas<'_> {
    /// Sample position (frames) under pixel `x`, floored at 0.
    pub(super) fn x_to_frames(&self, x: f32) -> u64 {
        let seconds = ((x + self.scroll_offset) / self.zoom).max(0.0);
        (seconds as f64 * self.sample_rate as f64) as u64
    }

    /// [`Self::x_to_frames`] snapped to the timeline grid — used when
    /// *adding* a breakpoint so new points land on the beat. Drags stay
    /// unsnapped for fine value/time control.
    fn x_to_frames_snapped(&self, x: f32) -> u64 {
        snap_sample_to_grid_tempo(
            self.x_to_frames(x),
            self.bpm,
            self.time_sig_num,
            self.sample_rate,
            self.zoom,
            self.tempo_map,
        )
    }

    /// The primary lane plus its row's top y / height for the visible track
    /// row whose vertical span contains `y`. `None` when `y` isn't over an
    /// automated track row (no lanes, a group-header band, a hidden
    /// collapsed-group member, or the row under `y` has no lane).
    ///
    /// Resolves the row through the same [`ArrangeRowLayout`] the draw pass
    /// consumes, so the pointer hits exactly the band the user sees drawn
    /// under the mixed 60/96 px row pitch (the #732 rule).
    fn automation_row_at(&self, y: f32) -> Option<(&AutomationLane, f32, f32)> {
        if self.automation.lanes.is_empty() {
            return None;
        }
        let header_height = self.fixed_header_height();
        let y_off = self.scroll_offset_y;
        let layout = self.arrange_layout();
        let row = layout.row_at_y(y - header_height + y_off)?;
        let track_id = row.track_id()?;
        let row_y = header_height + row.y_top - y_off;
        let row_height = row.height;
        let track = self
            .visible_tracks_sorted()
            .into_iter()
            .find(|t| t.id == track_id)?;
        let lane = primary_lane_for_track(self.automation, track)?;
        Some((lane, row_y, row_height))
    }

    /// Value-band geometry (`band_top`, `band_height`) for the row owning
    /// `target`'s lane. Used mid-drag, where the pointer y may leave the
    /// band but the value still maps against the band that owns the point.
    /// `None` when the owning track has no visible row (e.g. it folded
    /// into a collapsed group mid-gesture).
    fn target_band(&self, target: &AutomationTarget) -> Option<(f32, f32)> {
        let header_height = self.fixed_header_height();
        let y_off = self.scroll_offset_y;
        let layout = self.arrange_layout();
        for track in self.visible_tracks_sorted() {
            let owns =
                primary_lane_for_track(self.automation, track).map(|l| &l.target) == Some(target);
            if owns {
                let (row_y_top, row_height) = layout.track_row_rect(track.id)?;
                let row_y = header_height + row_y_top - y_off;
                return Some(automation_band(row_y, row_height));
            }
        }
        None
    }

    /// The breakpoint dot under the pointer, if any. Used to start a
    /// drag / curve-toggle (left) or delete (right).
    pub(super) fn breakpoint_hit(&self, pos: Point) -> Option<BreakpointHit> {
        let (lane, row_y, row_height) = self.automation_row_at(pos.y)?;
        let (band_top, band_height) = automation_band(row_y, row_height);
        let idx = nearest_breakpoint(
            &lane.points,
            pos,
            band_top,
            band_height,
            |frames| self.sample_to_x(frames),
            BREAKPOINT_HIT_RADIUS,
        )?;
        Some(BreakpointHit {
            target: lane.target.clone(),
            index: idx,
        })
    }

    /// An "add a breakpoint here" target for a press on empty band space:
    /// the lane target, the grid-snapped frame, and the value mapped from
    /// the pointer y. `None` unless the pointer is inside an automated
    /// row's value band — the row's top/bottom insets stay free for clip
    /// grabbing on automated tracks.
    pub(super) fn band_add_at(&self, pos: Point) -> Option<(AutomationTarget, u64, f32)> {
        let (lane, row_y, row_height) = self.automation_row_at(pos.y)?;
        let (band_top, band_height) = automation_band(row_y, row_height);
        if pos.y < band_top - BREAKPOINT_HIT_RADIUS
            || pos.y > band_top + band_height + BREAKPOINT_HIT_RADIUS
        {
            return None;
        }
        let value = value_from_y(pos.y, band_top, band_height);
        Some((lane.target.clone(), self.x_to_frames_snapped(pos.x), value))
    }

    /// New `(frame, value)` for a breakpoint drag to `pos`: value from the
    /// pointer y mapped into the lane's band, frame from the pointer x
    /// clamped between the point's time-neighbors so the drag can't reorder
    /// the lane — which keeps the dragged index stable for the whole
    /// gesture (the update handler re-sorts, but a clamped move is a no-op
    /// for the sort).
    pub(super) fn breakpoint_drag_to(
        &self,
        target: AutomationTarget,
        index: usize,
        pos: Point,
    ) -> Option<(u64, f32)> {
        let (band_top, band_height) = self.target_band(&target)?;
        let value = value_from_y(pos.y, band_top, band_height);
        let lane = self.automation.lanes.get(&target)?;
        if index >= lane.points.len() {
            return None;
        }
        let left = if index > 0 {
            lane.points[index - 1].time_frames
        } else {
            0
        };
        let right = if index + 1 < lane.points.len() {
            lane.points[index + 1].time_frames
        } else {
            u64::MAX
        };
        let frames = self.x_to_frames(pos.x).clamp(left, right);
        Some((frames, value))
    }

    /// The curve kind a double-click should set on the breakpoint at
    /// `index`: the *other* of the two kinds (Linear ⇄ Stepped). Defaults
    /// to flipping the default when the point is gone.
    pub(super) fn toggled_breakpoint_curve(
        &self,
        target: AutomationTarget,
        index: usize,
    ) -> CurveKind {
        let current = self
            .automation
            .lanes
            .get(&target)
            .and_then(|l| l.points.get(index))
            .map(|p| p.curve)
            .unwrap_or_default();
        match current {
            CurveKind::Linear => CurveKind::Stepped,
            CurveKind::Stepped => CurveKind::Linear,
        }
    }
}

/// Format a target's real value for the live read-out: dB for gain, a signed
/// pan position, On/Off for mute, two decimals for a plugin param.
pub fn format_real_value(target: AutomationTarget, real: f32) -> String {
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
