//! Pure hit-testing helpers shared between audio and MIDI clip lanes on the
//! timeline canvas. The caller computes the clip's pixel rect first (via
//! [`clip_rect`] for audio or the MIDI equivalent with tick→sample
//! conversion) and then asks [`hit_test`] which part of the clip the pointer
//! is over.

use iced::{Point, Rectangle};

use crate::state::{ClipEdge, TrackState};
use resonance_audio::types::TrackId;

/// Outcome of a hit-test against a single clip rectangle.
#[derive(Debug, Clone, Copy)]
pub enum HitKind {
    /// Pointer is on the left or right trim handle.
    Trim(ClipEdge),
    /// Pointer is on the clip body; `grab_offset_x` is the x offset from
    /// the clip's left edge (in canvas pixels).
    Move { grab_offset_x: f32 },
    /// Pointer missed the clip entirely.
    Miss,
}

/// Build the pixel rect for a clip at the given track row.
///
/// `duration_samples` is the already-converted length of the clip. For MIDI
/// clips the caller performs the tick→sample conversion (using BPM and
/// sample rate) before calling this helper.
pub fn clip_rect(
    track_row_y: f32,
    row_height: f32,
    start_sample: u64,
    duration_samples: u64,
    zoom: f32,
    sample_rate: u32,
    scroll_offset: f32,
) -> Rectangle {
    let start_seconds = start_sample as f32 / sample_rate as f32;
    let duration_seconds = duration_samples as f32 / sample_rate as f32;
    let x = start_seconds * zoom - scroll_offset;
    let width = duration_seconds * zoom;
    Rectangle {
        x,
        y: track_row_y + 2.0,
        width,
        height: row_height - 4.0,
    }
}

/// Hit-test a pointer position against a clip rectangle. Returns `Miss` when
/// the pointer is outside the rect. The `trim_threshold` controls the width
/// of the left/right edge handles.
pub fn hit_test(pos: Point, rect: Rectangle, trim_threshold: f32) -> HitKind {
    if pos.x < rect.x
        || pos.x > rect.x + rect.width
        || pos.y < rect.y
        || pos.y > rect.y + rect.height
    {
        return HitKind::Miss;
    }
    if pos.x - rect.x < trim_threshold {
        return HitKind::Trim(ClipEdge::Left);
    }
    if (rect.x + rect.width) - pos.x < trim_threshold {
        return HitKind::Trim(ClipEdge::Right);
    }
    HitKind::Move {
        grab_offset_x: pos.x - rect.x,
    }
}

/// Arrange-view row y (top of the row, in canvas coordinates) for a given
/// track index.
pub fn track_row_y(index: usize, ruler_height: f32, scroll_offset_y: f32, row_h: f32) -> f32 {
    ruler_height + index as f32 * row_h - scroll_offset_y
}

/// Tracks visible in the arrange view, sorted by `order`. Sub-tracks are
/// rendered only in the mixer view, so they are excluded here.
pub fn sorted_arrange_tracks(tracks: &[TrackState]) -> Vec<&TrackState> {
    let mut v: Vec<&TrackState> = tracks.iter().filter(|t| t.sub_track.is_none()).collect();
    v.sort_by_key(|t| t.order);
    v
}

/// Look up a track's row index in the sorted-arrange list by id.
pub fn track_index(sorted: &[&TrackState], id: TrackId) -> Option<usize> {
    sorted.iter().position(|t| t.id == id)
}
