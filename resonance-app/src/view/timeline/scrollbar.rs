//! Scrollbar rect + drag-grab math for the timeline canvas's vertical
//! scrollbar. (Horizontal scroll is handled by the outer `Scrollable`
//! that wraps the canvas.)

use iced::Rectangle;

/// Thickness of the scrollbar strips drawn inside the timeline canvas.
pub const THICKNESS: f32 = 10.0;
/// Minimum thumb size in pixels so the thumb stays clickable at any zoom.
pub const MIN_THUMB: f32 = 24.0;

/// Computed rects for one axis of the scrollbar. `travel` is the number of
/// pixels the thumb can move, and `max_scroll` is the caller's scroll range.
#[derive(Debug, Clone, Copy)]
pub struct ScrollbarRects {
    pub track: Rectangle,
    pub thumb: Rectangle,
    pub travel: f32,
    pub max_scroll: f32,
}

/// Vertical scrollbar rects (track area only, excludes the ruler at the top).
/// Returns `None` when the track area fits the viewport.
pub fn v_rects(
    bounds: Rectangle,
    content_height: f32,
    scroll_offset_y: f32,
    ruler_height: f32,
    show_h_bar: bool,
) -> Option<ScrollbarRects> {
    let viewport_height = bounds.height - ruler_height - if show_h_bar { THICKNESS } else { 0.0 };
    let track_content_h = content_height - ruler_height;
    if viewport_height <= 0.0 || track_content_h <= viewport_height + 0.5 {
        return None;
    }
    let track = Rectangle {
        x: bounds.width - THICKNESS,
        y: ruler_height,
        width: THICKNESS,
        height: viewport_height,
    };
    let ratio_visible = (viewport_height / track_content_h).clamp(0.0, 1.0);
    let thumb_h = (viewport_height * ratio_visible).max(MIN_THUMB);
    let max_scroll = (track_content_h - viewport_height).max(1.0);
    let travel = (viewport_height - thumb_h).max(0.0);
    let thumb_y = ruler_height + (scroll_offset_y / max_scroll).clamp(0.0, 1.0) * travel;
    let thumb = Rectangle {
        x: track.x,
        y: thumb_y,
        width: THICKNESS,
        height: thumb_h,
    };
    Some(ScrollbarRects {
        track,
        thumb,
        travel,
        max_scroll,
    })
}

/// Convert a thumb-relative offset (pointer pixel minus clamp origin)
/// into a scroll position. Both press-time page-jumps and move-time
/// drags use this: the axis-specific caller computes `thumb_rel`
/// (typically `pointer − origin − grab` for drag or
/// `pointer − thumb_size/2` for page-jump) and the helper handles the
/// clamp + ratio.
pub fn scroll_from_thumb_pos(thumb_rel: f32, travel: f32, max_scroll: f32) -> f32 {
    let clamped = thumb_rel.clamp(0.0, travel);
    if travel > 0.0 {
        clamped / travel * max_scroll
    } else {
        0.0
    }
}
