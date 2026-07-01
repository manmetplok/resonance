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
use resonance_audio::types::{TempoMap, TrackId};

use super::snap::snap_sample_to_grid_tempo;
use crate::message::DropTarget;
use crate::state::DropResolution;

/// The timeline layout constants a drop needs to map pixels ↔ (lane,
/// sample). Snapshotted from the canvas / viewport at the moment of the
/// pointer move so [`resolve_drop`] stays a pure function.
#[derive(Debug, Clone, Copy)]
pub struct PlacementGeometry {
    /// Y where regular track rows begin (ruler + section band + global
    /// tracks) — `TimelineCanvas::fixed_header_height()`.
    pub header_height: f32,
    /// Per-lane row height (`theme::TRACK_HEIGHT`).
    pub track_height: f32,
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

/// The lane index a content-space y falls in, or `None` when it is below
/// the last lane (the new-audio-track drop zone). A y above the first lane
/// clamps to lane 0 so a drag drifting up into the ruler still targets the
/// top track rather than snapping to "new track".
fn lane_at(y: f32, geo: &PlacementGeometry, lane_count: usize) -> Option<usize> {
    if lane_count == 0 {
        return None;
    }
    let rel = y + geo.scroll_offset_y - geo.header_height;
    if rel < 0.0 {
        return Some(0);
    }
    let idx = (rel / geo.track_height).floor() as usize;
    if idx >= lane_count {
        None
    } else {
        Some(idx)
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
/// `track_ids` are the arrange-sorted, arrange-visible track ids (same
/// order the canvas draws lanes in). The returned [`DropResolution`] carries
/// the grid-snapped [`DropTarget`], the targeted lane index (or `None` for
/// the new-track zone), and a bar label for the tooltip.
pub fn resolve_drop(
    geo: &PlacementGeometry,
    tempo_map: &TempoMap,
    track_ids: &[TrackId],
    cursor: Point,
) -> DropResolution {
    let raw_sample = x_to_sample(cursor.x, geo.zoom, geo.sample_rate);
    let start_sample = snap_sample_to_grid_tempo(
        raw_sample,
        geo.bpm,
        geo.time_sig_num,
        geo.sample_rate,
        geo.zoom,
        tempo_map,
    );

    let lane_index = lane_at(cursor.y, geo, track_ids.len());
    let target = match lane_index.and_then(|i| track_ids.get(i)) {
        Some(&track_id) => DropTarget::ExistingTrack {
            track_id,
            start_sample,
        },
        None => DropTarget::NewTrack { start_sample },
    };

    DropResolution {
        target,
        lane_index,
        bar_label: bar_label(start_sample, geo.sample_rate, tempo_map),
    }
}
