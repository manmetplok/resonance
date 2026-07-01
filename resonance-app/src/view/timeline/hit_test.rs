//! Pure hit-testing helpers shared between audio and MIDI clip lanes on the
//! timeline canvas. The caller computes the clip's pixel rect first (via
//! [`clip_rect`] for audio or the MIDI equivalent with tick→sample
//! conversion) and then asks [`hit_test`] which part of the clip the pointer
//! is over.

use iced::{Point, Rectangle};

use crate::state::{ClipEdge, TrackState};
use resonance_audio::types::TrackId;

/// Outcome of a hit-test against a single clip rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HitKind {
    /// Pointer is on the left or right trim handle.
    Trim(ClipEdge),
    /// Pointer is on the fade-in handle bead (top-left corner, riding
    /// inward as the fade grows). Audio clips only.
    FadeIn,
    /// Pointer is on the fade-out handle bead (top-right corner). Audio
    /// clips only.
    FadeOut,
    /// Pointer is on the clip-gain bead (top-centre). Audio clips only.
    Gain,
    /// Pointer is on the clip body; `grab_offset_x` is the x offset from
    /// the clip's left edge (in canvas pixels).
    Move { grab_offset_x: f32 },
    /// Pointer missed the clip entirely.
    Miss,
}

/// Hit radius (px) of the circular fade/gain handle beads. Deliberately
/// larger than [`crate::theme::CLIP_EDGE_THRESHOLD`] (the 6px trim band) so
/// fades win over trim at the very top corners while trim keeps the rest of
/// the vertical edge — see design doc #153.
pub const FADE_HANDLE_RADIUS: f32 = 10.0;
/// Hit radius (px) of the clip-gain bead at the clip's top-centre.
pub const GAIN_HANDLE_RADIUS: f32 = 10.0;

/// Pixel geometry of an audio clip's fade/gain handle beads. The bead
/// centres sit on the clip's top edge; [`hit_test_audio`] tests the pointer
/// against them.
#[derive(Debug, Clone, Copy)]
pub struct ClipHandles {
    /// x of the fade-in bead centre (rides inward from the left edge as the
    /// fade length grows; equals the left edge when the fade is 0).
    pub fade_in_x: f32,
    /// x of the fade-out bead centre (rides inward from the right edge).
    pub fade_out_x: f32,
    /// x of the clip-gain bead centre (clip top-centre).
    pub gain_x: f32,
    /// y of all three bead centres (the clip's top edge).
    pub top_y: f32,
    /// Whether fade handles are exposed. Frozen / rendered / no-source
    /// clips set this `false` so they expose no fade hits; gain still
    /// applies (design doc #153).
    pub fadeable: bool,
}

/// Compute the fade/gain handle geometry for an audio clip rectangle.
///
/// `fade_in_seconds` / `fade_out_seconds` are the clip's fade ramp lengths;
/// the handle x equals the ramp end (`handle x = ramp end`). Beads are
/// clamped within the clip so a fade longer than the clip can't push a
/// handle past the opposite edge.
pub fn audio_clip_handles(
    rect: Rectangle,
    fade_in_seconds: f32,
    fade_out_seconds: f32,
    zoom: f32,
    fadeable: bool,
) -> ClipHandles {
    let right = rect.x + rect.width;
    let fade_in_x = (rect.x + fade_in_seconds * zoom).clamp(rect.x, right);
    let fade_out_x = (right - fade_out_seconds * zoom).clamp(rect.x, right);
    ClipHandles {
        fade_in_x,
        fade_out_x,
        gain_x: rect.x + rect.width / 2.0,
        top_y: rect.y,
        fadeable,
    }
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

/// Hit-test a pointer against an audio clip, including its fade/gain handle
/// beads. The beads sit on the clip's top edge and take priority over trim
/// at the top corners (their larger radius disambiguates fade-vs-trim per
/// design doc #153). Gain is available even on non-fadeable clips; fade-in /
/// fade-out hits are suppressed when `handles.fadeable` is `false`.
///
/// Falls back to [`hit_test`] (trim edges / body move / miss) when the
/// pointer is on none of the beads.
pub fn hit_test_audio(
    pos: Point,
    rect: Rectangle,
    trim_threshold: f32,
    handles: &ClipHandles,
) -> HitKind {
    let near = |bx: f32, r: f32| {
        let dx = pos.x - bx;
        let dy = pos.y - handles.top_y;
        // Constrain to the clip's x span so a bead can't be hit from far
        // off the side of the clip.
        pos.x >= rect.x && pos.x <= rect.x + rect.width && dx * dx + dy * dy <= r * r
    };
    if handles.fadeable {
        if near(handles.fade_in_x, FADE_HANDLE_RADIUS) {
            return HitKind::FadeIn;
        }
        if near(handles.fade_out_x, FADE_HANDLE_RADIUS) {
            return HitKind::FadeOut;
        }
    }
    if near(handles.gain_x, GAIN_HANDLE_RADIUS) {
        return HitKind::Gain;
    }
    hit_test(pos, rect, trim_threshold)
}

/// Which part of an arrangement-marker's ruler geometry the pointer is
/// over. Returned by [`marker_hit`] and consumed by the ruler input
/// handlers to pick the drag / select behaviour (todo #369).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerHit {
    /// The start pole / flag — a click selects, a drag moves the marker
    /// start (`MoveStart`). Shared by point and region markers.
    Flag,
    /// A region marker's end edge — a drag resizes it (`SetRegionEnd`).
    EndEdge,
}

/// Flag pennant width in px. Must match the `FLAG_W` constant used by
/// [`super::draw::TimelineCanvas::draw_markers`] so the grab zone lines up
/// with the drawn flag.
pub const MARKER_FLAG_W: f32 = 11.0;
/// Padding (px) either side of the flag/pole for the start grab zone, so
/// the thin 1px pole is comfortably clickable.
pub const MARKER_FLAG_PAD: f32 = 4.0;
/// Half-width (px) of the region end-edge resize band.
pub const MARKER_EDGE_THRESHOLD: f32 = 5.0;

/// Hit-test a pointer x against a single marker's ruler geometry.
///
/// `start_x` is the marker's start-pole pixel x; `end_x` is `Some` for a
/// ranged region (its end-edge pixel x). The caller is expected to have
/// already confirmed the pointer is within the ruler band vertically.
///
/// A region's end edge wins over its start when the pointer sits right on
/// it, so a narrow region can still be resized; otherwise the flag grab
/// zone — `[start_x - PAD, start_x + FLAG_W + PAD]` — selects / moves. The
/// translucent region *body* is intentionally not a grab target so ruler
/// seeking still works underneath a wide section region.
pub fn marker_hit(pos_x: f32, start_x: f32, end_x: Option<f32>) -> Option<MarkerHit> {
    if let Some(ex) = end_x {
        if (pos_x - ex).abs() <= MARKER_EDGE_THRESHOLD {
            return Some(MarkerHit::EndEdge);
        }
    }
    if pos_x >= start_x - MARKER_FLAG_PAD && pos_x <= start_x + MARKER_FLAG_W + MARKER_FLAG_PAD {
        return Some(MarkerHit::Flag);
    }
    None
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
