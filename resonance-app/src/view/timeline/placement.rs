//! Pure geometry for the drag-to-timeline placement gesture (doc #175,
//! todo #605).
//!
//! Resolving a cursor position over the arrangement into a concrete drop
//! target — which lane, which grid-snapped sample, and a human bar label —
//! is the one piece of the drag that has to agree exactly with how the
//! canvas lays clips out. Keeping it here as a pure function lets the
//! timeline canvas call it on every pointer move *and* lets the tests
//! assert the mapping directly, without rendering.
//!
//! Coordinates are **canvas content coordinates**: the same space
//! [`TimelineCanvas::sample_to_x`](super::TimelineCanvas::sample_to_x) works
//! in, where x=0 is sample 0 (the outer `Scrollable` owns horizontal
//! scrolling, so the canvas itself renders from sample-zero and its
//! `scroll_offset` is pinned to 0). Vertical scrolling *is* internal, so
//! `scroll_offset_y` is folded in here.

use iced::Point;
use resonance_audio::types::TempoMap;

use super::snap::snap_sample_to_grid_tempo;
use crate::message::DropTarget;
use crate::state::DropResolution;
use crate::view::arrange_layout::ArrangeRowLayout;

/// The timeline layout constants a drop needs to map pixels ↔ (lane,
/// sample). Snapshotted from the canvas / viewport at the moment of the
/// pointer move so [`resolve_drop`] stays a pure function.
#[derive(Debug, Clone, Copy)]
pub struct PlacementGeometry {
    /// Y where regular track rows begin (ruler + section band + global
    /// tracks) — `TimelineCanvas::fixed_header_height()`.
    pub header_height: f32,
    /// Internal vertical scroll offset.
    pub scroll_offset_y: f32,
    /// Horizontal zoom in pixels per second.
    pub zoom: f32,
    pub sample_rate: u32,
    pub bpm: f32,
    pub time_sig_num: u8,
}

/// Convert a content-space x to a raw (un-snapped) sample position.
fn x_to_sample(x: f32, zoom: f32, sample_rate: u32) -> u64 {
    if zoom <= 0.0 {
        return 0;
    }
    let seconds = (x.max(0.0) / zoom) as f64;
    (seconds * sample_rate as f64).max(0.0) as u64
}

/// Where a content-space y lands for a drop.
enum LaneHit {
    /// Over the clip lane of the arrange row at this layout index.
    Row(usize),
    /// Below the last row: the new-audio-track drop zone.
    NewTrack,
    /// Over a row that is not a clip lane (group header, automation or
    /// take sub-row): not a drop target.
    None,
}

/// Resolve y against the shared [`ArrangeRowLayout`] — the same rows the
/// canvas draws, with their mixed pitches (group headers, expanded
/// automation and take sub-rows, collapsed groups). A y above the first
/// row clamps to the first track row so a drag drifting up into the ruler
/// still targets the top track rather than "new track".
fn lane_at(y: f32, geo: &PlacementGeometry, layout: &ArrangeRowLayout) -> LaneHit {
    let rows = layout.rows();
    let rel = y + geo.scroll_offset_y - geo.header_height;
    if rel >= layout.total_height() {
        return LaneHit::NewTrack;
    }
    let index = if rel < 0.0 {
        rows.iter().position(|r| r.track_id().is_some())
    } else {
        rows.iter().position(|r| rel >= r.y_top && rel < r.y_bottom())
    };
    match index {
        Some(i) if rows[i].track_id().is_some() => LaneHit::Row(i),
        _ => LaneHit::None,
    }
}

/// A `"Bar b.beat"` label (both 1-based) for a snapped sample, e.g.
/// `"Bar 5.1"`. Public so the tooltip / tests share one formatting.
pub fn bar_label(sample: u64, sample_rate: u32, tempo_map: &TempoMap) -> String {
    let (bar, frac) = tempo_map.sample_to_bar(sample, sample_rate);
    let numerator = tempo_map.numerator_at_bar(bar).max(1);
    let beat = (frac * numerator as f64).floor() as u32 + 1;
    format!("Bar {}.{}", bar + 1, beat)
}

/// Resolve a cursor point (canvas content coords) into a drop target.
///
/// `layout` is the canvas's arrange-row layout. The returned
/// [`DropResolution`] carries the grid-snapped [`DropTarget`], the targeted
/// row's index in `layout.rows()` (or `None` for the new-track zone), and a
/// bar label for the tooltip. `None` when the cursor is over a row that
/// accepts no clip (group header, automation or take sub-row).
pub fn resolve_drop(
    geo: &PlacementGeometry,
    tempo_map: &TempoMap,
    layout: &ArrangeRowLayout,
    cursor: Point,
) -> Option<DropResolution> {
    let raw_sample = x_to_sample(cursor.x, geo.zoom, geo.sample_rate);
    let start_sample = snap_sample_to_grid_tempo(
        raw_sample,
        geo.bpm,
        geo.time_sig_num,
        geo.sample_rate,
        geo.zoom,
        tempo_map,
    );

    let (target, lane_index) = match lane_at(cursor.y, geo, layout) {
        LaneHit::Row(i) => (
            DropTarget::ExistingTrack {
                track_id: layout.rows()[i].track_id()?,
                start_sample,
            },
            Some(i),
        ),
        LaneHit::NewTrack => (DropTarget::NewTrack { start_sample }, None),
        LaneHit::None => return None,
    };

    Some(DropResolution {
        target,
        lane_index,
        bar_label: bar_label(start_sample, geo.sample_rate, tempo_map),
    })
}
