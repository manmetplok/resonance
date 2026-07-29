//! Breakpoint hit-testing and edit-geometry for the timeline canvas
//! (todo #382 / A4). These translate a pointer position into the lane,
//! breakpoint and value the input handlers in [`super::super::input`]
//! act on. Band resolution goes through the same
//! [`crate::view::arrange_layout::ArrangeRowLayout`] the draw pass
//! consumes, so the pointer hits exactly what is drawn (the #732 rule).

use iced::Point;

use crate::state::TrackState;
use crate::view::arrange_layout::ArrangeRowKind;
use resonance_common::{AutomationLane, AutomationTarget, CurveKind};

use super::super::snap::snap_sample_to_grid_tempo;
use super::super::TimelineCanvas;
use super::{
    automation_band, lane_chip_rect, lane_row_band, nearest_breakpoint, shown_lane_for_track,
    target_belongs_to_track, target_label, track_lanes_sorted, value_from_y,
};

/// Pixel pick radius for clicking a breakpoint dot (todo #382). A touch
/// larger than the drawn dot radius so the small dots are easy to grab.
pub const BREAKPOINT_HIT_RADIUS: f32 = 7.0;

/// A pointer hit on a breakpoint dot: which lane drives it and the index of
/// the hit point within the lane's time-sorted point list.
#[derive(Debug, Clone)]
pub(crate) struct BreakpointHit {
    pub target: AutomationTarget,
    pub index: usize,
}

/// The automation band the pointer's y lands in (see
/// [`TimelineCanvas::automation_row_at`]): the owning track, the lane the
/// band edits, the band's screen-space geometry, and whether the band is a
/// dedicated `AutomationLane` sub-row (todo #1097) rather than the
/// collapsed in-track overlay.
struct AutomationBandHit<'l> {
    track: &'l TrackState,
    lane: &'l AutomationLane,
    band_top: f32,
    band_height: f32,
    /// `true` for a dedicated 44 px lane sub-row; `false` for the
    /// in-track overlay band of a collapsed (non-expanded) track.
    dedicated_row: bool,
}

impl TimelineCanvas<'_> {
    /// Sample position (frames) under pixel `x`, floored at 0.
    pub(in crate::view::timeline) fn x_to_frames(&self, x: f32) -> u64 {
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

    /// The automation band under `y`, resolved through the same
    /// [`crate::view::arrange_layout::ArrangeRowLayout`] the draw pass
    /// consumes so the pointer hits exactly the band the user sees drawn
    /// under the mixed 44/60/96 px row pitch (the #732 rule). Two kinds of
    /// row carry a band:
    ///
    /// * a **dedicated `AutomationLane` sub-row** (doc #256, todo #1097):
    ///   the lane is resolved directly from the row kind — every lane of
    ///   an expanded track is editable in its own row;
    /// * a collapsed **track row**: the *shown* lane (chip-cycle selection
    ///   or priority default) in the in-track overlay band, as before.
    ///
    /// A track in `automation_expanded_tracks` returns `None` for its
    /// track row — the overlay band is suppressed while its lanes show as
    /// sub-rows, so no overlay gesture (chip, dot, band-add) can land on
    /// the clip lane. Group-header rows and hidden collapsed-group members
    /// resolve to `None` as always.
    fn automation_row_at(&self, y: f32) -> Option<AutomationBandHit<'_>> {
        if self.automation.lanes.is_empty() {
            return None;
        }
        let header_height = self.fixed_header_height();
        let y_off = self.scroll_offset_y;
        let layout = self.arrange_layout();
        let row = layout.row_at_y(y - header_height + y_off)?;
        let row_y = header_height + row.y_top - y_off;
        match row.kind {
            ArrangeRowKind::Track(track_id) => {
                // Expanded: the in-track overlay is suppressed — its lanes
                // live in the sub-rows below (drawn there, hit there).
                if self.automation_expanded_tracks.contains(&track_id) {
                    return None;
                }
                let track = self
                    .visible_tracks_sorted()
                    .into_iter()
                    .find(|t| t.id == track_id)?;
                let lane = shown_lane_for_track(self.automation, track)?;
                let (band_top, band_height) = automation_band(row_y, row.height);
                Some(AutomationBandHit {
                    track,
                    lane,
                    band_top,
                    band_height,
                    dedicated_row: false,
                })
            }
            ArrangeRowKind::AutomationLane { track, lane } => {
                let track = self
                    .visible_tracks_sorted()
                    .into_iter()
                    .find(|t| t.id == track)?;
                let lane = self
                    .automation
                    .lanes
                    .values()
                    .find(|l| l.id == lane && target_belongs_to_track(&l.target, track))?;
                let (band_top, band_height) = lane_row_band(row_y, row.height);
                Some(AutomationBandHit {
                    track,
                    lane,
                    band_top,
                    band_height,
                    dedicated_row: true,
                })
            }
            ArrangeRowKind::GroupHeader(_) => None,
        }
    }

    /// The track whose clickable parameter chip is under `pos`, or `None`.
    /// The chip only exists (and is only clickable) when more than one lane
    /// targets the track — single-lane tracks keep their plain label and
    /// every pre-#1095 click behavior. Dedicated lane rows (todo #1097)
    /// have no cycle chip either: every lane is already visible, so their
    /// plain label never captures a click. Geometry comes from the same
    /// [`lane_chip_rect`] the draw pass fills (#732 rule).
    pub(in crate::view::timeline) fn lane_chip_hit(
        &self,
        pos: Point,
    ) -> Option<resonance_common::TrackId> {
        let hit = self.automation_row_at(pos.y)?;
        if hit.dedicated_row {
            return None;
        }
        if track_lanes_sorted(self.automation, hit.track).len() < 2 {
            return None;
        }
        let label = target_label(&hit.lane.target, &self.device_param_labels);
        if label.is_empty() {
            return None;
        }
        lane_chip_rect(hit.band_top, &label)
            .contains(pos)
            .then_some(hit.track.id)
    }

    /// Value-band geometry (`band_top`, `band_height`) for the band owning
    /// `target`'s lane. Used mid-drag, where the pointer y may leave the
    /// band but the value still maps against the band that owns the point.
    /// For an expanded track the band is the lane's own dedicated sub-row
    /// (todo #1097); for a collapsed track it is the in-track overlay —
    /// but only while the lane is the *shown* one, matching what's drawn.
    /// `None` when the owning track has no visible band (e.g. it folded
    /// into a collapsed group mid-gesture).
    fn target_band(&self, target: &AutomationTarget) -> Option<(f32, f32)> {
        let header_height = self.fixed_header_height();
        let y_off = self.scroll_offset_y;
        let layout = self.arrange_layout();
        for track in self.visible_tracks_sorted() {
            if !target_belongs_to_track(target, track) {
                continue;
            }
            if self.automation_expanded_tracks.contains(&track.id) {
                let lane = self.automation.lanes.get(target)?;
                let (row_y_top, row_height) = layout.automation_row_rect(track.id, lane.id)?;
                let row_y = header_height + row_y_top - y_off;
                return Some(lane_row_band(row_y, row_height));
            }
            let shown =
                shown_lane_for_track(self.automation, track).map(|l| &l.target) == Some(target);
            if shown {
                let (row_y_top, row_height) = layout.track_row_rect(track.id)?;
                let row_y = header_height + row_y_top - y_off;
                return Some(automation_band(row_y, row_height));
            }
        }
        None
    }

    /// The breakpoint dot under the pointer, if any. Used to start a
    /// drag / curve-toggle (left) or delete (right). Resolves through
    /// [`Self::automation_row_at`], so on an expanded track each lane's
    /// dots are hit inside that lane's own sub-row (todo #1097).
    pub(crate) fn breakpoint_hit(&self, pos: Point) -> Option<BreakpointHit> {
        let hit = self.automation_row_at(pos.y)?;
        let idx = nearest_breakpoint(
            &hit.lane.points,
            pos,
            hit.band_top,
            hit.band_height,
            |frames| self.sample_to_x(frames),
            BREAKPOINT_HIT_RADIUS,
        )?;
        Some(BreakpointHit {
            target: hit.lane.target.clone(),
            index: idx,
        })
    }

    /// An "add a breakpoint here" target for a press on empty band space:
    /// the lane target, the grid-snapped frame, and the value mapped from
    /// the pointer y. `None` unless the pointer is inside an automated
    /// band — the overlay band of a collapsed track (the row's top/bottom
    /// insets stay free for clip grabbing), or a dedicated lane row's band
    /// on an expanded track, which adds to exactly that row's lane.
    pub(crate) fn band_add_at(&self, pos: Point) -> Option<(AutomationTarget, u64, f32)> {
        let hit = self.automation_row_at(pos.y)?;
        if pos.y < hit.band_top - BREAKPOINT_HIT_RADIUS
            || pos.y > hit.band_top + hit.band_height + BREAKPOINT_HIT_RADIUS
        {
            return None;
        }
        let value = value_from_y(pos.y, hit.band_top, hit.band_height);
        Some((
            hit.lane.target.clone(),
            self.x_to_frames_snapped(pos.x),
            value,
        ))
    }

    /// New `(frame, value)` for a breakpoint drag to `pos`: value from the
    /// pointer y mapped into the lane's band, frame from the pointer x
    /// clamped between the point's time-neighbors so the drag can't reorder
    /// the lane — which keeps the dragged index stable for the whole
    /// gesture (the update handler re-sorts, but a clamped move is a no-op
    /// for the sort).
    pub(in crate::view::timeline) fn breakpoint_drag_to(
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
    pub(in crate::view::timeline) fn toggled_breakpoint_curve(
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
