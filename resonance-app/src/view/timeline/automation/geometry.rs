//! Pure automation-lane geometry and lane-selection helpers: band rects,
//! value↔y mapping, envelope polylines, target labels/priorities and the
//! shown-lane/cycle logic. No canvas, no `TimelineCanvas` — everything
//! here is a pure function, unit-testable without a live canvas.

use std::collections::HashMap;

use iced::{Point, Rectangle};

use crate::state::TrackState;
use resonance_common::{AutomationLane, AutomationTarget, Breakpoint, CurveKind};

/// Vertical inset of the value band from the track row's top/bottom edges.
/// Larger than [`crate::theme::CLIP_LANE_INSET`] so the envelope sits clear
/// of the clip name label that hugs the top of the row.
const BAND_INSET: f32 = 18.0;
/// Top inset of the value band inside a dedicated 44 px automation
/// sub-row (doc #256, todo #1097). Leaves exactly enough headroom for the
/// parameter-label chip, which rises [`CHIP_RISE`] px above the band top:
/// `LANE_ROW_BAND_TOP_INSET - CHIP_RISE = 1`, so the chip's top edge sits
/// 1 px inside the row instead of bleeding into the row above.
const LANE_ROW_BAND_TOP_INSET: f32 = 16.0;
/// Bottom inset of the value band inside a dedicated automation sub-row.
/// Slimmer than the top inset (no chip below the band) — together they
/// leave a 22 px editable band in the 44 px row.
const LANE_ROW_BAND_BOTTOM_INSET: f32 = 6.0;

/// The value-band rect (top-left + height) inside a track row whose top edge
/// is `row_y` and whose height is `row_height`. Normalized lane value `1.0`
/// maps to `band_top`, `0.0` to the band bottom (axis grows downward like the
/// rest of the canvas). The band is derived from the row's *actual* height —
/// rows come from the shared [`crate::view::arrange_layout::ArrangeRowLayout`]
/// (doc #203), not a fixed `TRACK_HEIGHT` pitch.
pub fn automation_band(row_y: f32, row_height: f32) -> (f32, f32) {
    let band_top = row_y + BAND_INSET;
    let band_height = (row_height - 2.0 * BAND_INSET).max(1.0);
    (band_top, band_height)
}

/// The value-band rect inside a dedicated 44 px automation sub-row (doc
/// #256, todo #1097). Same contract as [`automation_band`], but with the
/// slimmer lane-row insets: the label chip fills the strip above the band
/// (inside the row) and only a hairline margin remains below. Shared by the
/// row draw pass and the pointer hit-testing so gestures land exactly on
/// the band the user sees (the #732 rule).
pub fn lane_row_band(row_y: f32, row_height: f32) -> (f32, f32) {
    let band_top = row_y + LANE_ROW_BAND_TOP_INSET;
    let band_height =
        (row_height - LANE_ROW_BAND_TOP_INSET - LANE_ROW_BAND_BOTTOM_INSET).max(1.0);
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
/// flat-leads out to `right` at the last value, mirroring
/// [`resonance_common::sample_lane`]'s clamp-at-ends behaviour. Returns an
/// empty vec when there are no points.
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

/// The lane-ordering rules — [`target_priority`], [`target_belongs_to_track`]
/// and [`track_lanes_sorted`] — live in `state::automation` (ARCH2-05), shared
/// with the arrange-row layout; re-exported here so the overlay's callers
/// keep their path.
pub use crate::state::automation::{target_belongs_to_track, target_priority, track_lanes_sorted};

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

/// The lane drawn for `track` by default, if any — the highest-priority
/// target that belongs to the track, with ties (several device-param lanes
/// share one priority tier) broken by the lexicographically smallest
/// `param_id` so the pick is deterministic. `None` hides the lane (the
/// common case: tracks have no automation). The chip click can override
/// this via [`shown_lane_for_track`].
pub fn primary_lane_for_track<'l>(
    automation: &'l crate::state::AutomationState,
    track: &TrackState,
) -> Option<&'l AutomationLane> {
    track_lanes_sorted(automation, track).into_iter().next()
}

/// The lane the Arrange overlay actually shows for `track`: the transient
/// chip-cycle selection (`AutomationState::lane_selection`, todo #1095)
/// when it still resolves to one of the track's lanes, else the
/// priority-based [`primary_lane_for_track`] default. A stale selection —
/// the lane was removed, or the id belongs to another track's lane — falls
/// back to the default silently.
pub fn shown_lane_for_track<'l>(
    automation: &'l crate::state::AutomationState,
    track: &TrackState,
) -> Option<&'l AutomationLane> {
    if let Some(selected) = automation.lane_selection.get(&track.id) {
        let lane = automation
            .lanes
            .values()
            .find(|l| l.id == *selected && target_belongs_to_track(&l.target, track));
        if lane.is_some() {
            return lane;
        }
    }
    primary_lane_for_track(automation, track)
}

/// The lane id a chip click should switch `track` to: the lane after the
/// currently shown one in [`track_lanes_sorted`] order, wrapping past the
/// end. `None` when the track has no lanes at all. With a single lane this
/// wraps straight back to it (a no-op cycle).
pub fn next_lane_id_for_track(
    automation: &crate::state::AutomationState,
    track: &TrackState,
) -> Option<resonance_common::LaneId> {
    let lanes = track_lanes_sorted(automation, track);
    let shown = shown_lane_for_track(automation, track)?.id;
    // `shown` always comes from `lanes` (both filter by
    // `target_belongs_to_track`), so the position lookup can only miss if
    // the two ever diverge — fall back to cycling from the front.
    let idx = lanes.iter().position(|l| l.id == shown).unwrap_or(0);
    Some(lanes[(idx + 1) % lanes.len()].id)
}

/// Left edge of the parameter-label chip.
const CHIP_X: f32 = 2.0;
/// Horizontal padding between the chip edge and the label text (the text
/// itself is drawn at `CHIP_X + CHIP_PAD_X`).
pub(super) const CHIP_PAD_X: f32 = 2.0;
/// Estimated advance width of one label character at the 9 px chip font.
/// Canvas text has no measure API, so the chip width is estimated from the
/// character count — slightly generous so the drawn label always sits
/// inside the clickable rect.
const CHIP_CHAR_WIDTH: f32 = 5.6;
/// Chip height; covers the 9 px label plus a little breathing room.
const CHIP_HEIGHT: f32 = 12.0;
/// Vertical offset of the chip's top edge above the value band's top.
const CHIP_RISE: f32 = 15.0;

/// The parameter-label chip rect for a band whose top edge is `band_top`.
/// This is the single source of chip geometry: the draw pass fills it (and
/// positions the label / "N lanes" count relative to it) and the pointer
/// hit-test checks it, so what you click is exactly what you see (the #732
/// draw/hit-test rule).
pub fn lane_chip_rect(band_top: f32, label: &str) -> Rectangle {
    Rectangle {
        x: CHIP_X,
        y: band_top - CHIP_RISE,
        width: CHIP_PAD_X * 2.0 + label.chars().count() as f32 * CHIP_CHAR_WIDTH,
        height: CHIP_HEIGHT,
    }
}

/// Format a target's real value for the live read-out: dB for gain, a signed
/// pan position, On/Off for mute, two decimals for a plugin param.
pub fn format_real_value(target: AutomationTarget, real: f32) -> String {
    match target {
        AutomationTarget::TrackGain(_)
        | AutomationTarget::BusGain(_)
        | AutomationTarget::MasterGain => crate::util::format_db_signed(real, true),
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
