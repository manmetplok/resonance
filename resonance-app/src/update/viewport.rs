//! Viewport, scrolling, and zoom. The periodic `Tick` handler lives in
//! `update/tick.rs`.
use iced::Task;

use crate::message::Message;
use crate::Resonance;

#[derive(Debug, Clone)]
pub enum ViewportMessage {
    ZoomIn,
    ZoomOut,
    ScrollY(f32),
    /// The arrange view's outer horizontal `Scrollable` moved or resized
    /// (its `on_scroll`): live x offset, visible width and content width,
    /// all in px. Feeds playhead follow (review FU-D1).
    ArrangeScrolled {
        offset_x: f32,
        visible_width: f32,
        content_width: f32,
    },
    ScrollToY(f32),
    ViewportWidth(f32),
    /// Total available height the timeline canvas + track-header column
    /// see for content. Reported by `TimelineCanvas::report_viewport`
    /// whenever `bounds.height` moves more than 1 px. The track-header
    /// column uses this to drop tracks below the viewport during manual
    /// virtualization (see `view/track_header.rs`).
    ViewportHeight(f32),
    TimelineContentSize(f32, f32),
}

/// Route a `ViewportMessage` to the appropriate handler.
pub fn handle(r: &mut Resonance, m: ViewportMessage) -> Task<Message> {
    match m {
        ViewportMessage::ZoomIn => zoom_in(r),
        ViewportMessage::ZoomOut => zoom_out(r),
        ViewportMessage::ScrollY(delta) => scroll_y_delta(r, delta),
        ViewportMessage::ArrangeScrolled {
            offset_x,
            visible_width,
            content_width,
        } => arrange_scrolled(r, offset_x, visible_width, content_width),
        ViewportMessage::ScrollToY(y) => scroll_to_y(r, y),
        ViewportMessage::ViewportWidth(w) => viewport_width(r, w),
        ViewportMessage::ViewportHeight(h) => viewport_height(r, h),
        ViewportMessage::TimelineContentSize(w, h) => timeline_content_size(r, w, h),
    }
    Task::none()
}

/// Largest vertical scroll offset: arrange content height minus the
/// canvas height. The content height comes from the shared
/// `ArrangeRowLayout` (group headers, automation and take sub-rows
/// included), so a lane expanded since the canvas last reported its
/// content size is reachable at once; a larger reported height (a stale
/// report is re-clamped by `timeline_content_size`) is honoured too.
fn max_scroll_y(r: &Resonance) -> f32 {
    let layout_h = r.arrange_header_offset() + r.arrange_row_layout().total_height();
    let content_h = layout_h.max(r.viewport.timeline_content_height);
    (content_h - r.viewport.viewport_height).max(0.0)
}

pub fn scroll_y_delta(r: &mut Resonance, delta: f32) {
    let y = r.viewport.scroll_offset_y + delta;
    scroll_to_y(r, y);
}

/// Mirror the arrange `Scrollable`'s viewport into state. An offset
/// change that isn't the echo of follow's own `scroll_to` is the user
/// scrolling by hand: during playback that pauses follow until the
/// transport next stops (see `update::tick::follow_playhead`).
pub fn arrange_scrolled(r: &mut Resonance, offset_x: f32, visible_width: f32, content_width: f32) {
    let vp = &mut r.viewport;
    let moved = (offset_x - vp.scroll_offset).abs() > 0.5;
    let is_echo = vp
        .follow_pending_x
        .is_some_and(|target| (offset_x - target).abs() <= 1.0);
    if is_echo {
        vp.follow_pending_x = None;
    } else if moved && r.transport.playing {
        vp.follow_paused = true;
        vp.follow_pending_x = None;
    }
    vp.scroll_offset = offset_x;
    vp.visible_width = visible_width;
    vp.scroll_content_width = content_width;
}

pub fn scroll_to_y(r: &mut Resonance, y: f32) {
    r.viewport.scroll_offset_y = y.clamp(0.0, max_scroll_y(r));
}

pub fn viewport_width(r: &mut Resonance, w: f32) {
    r.viewport.viewport_width = w;
}

pub fn viewport_height(r: &mut Resonance, h: f32) {
    r.viewport.viewport_height = h;
}

pub fn timeline_content_size(r: &mut Resonance, w: f32, h: f32) {
    r.viewport.timeline_content_width = w;
    r.viewport.timeline_content_height = h;
    // Re-clamp the vertical offset if content shrank. (Horizontal scroll
    // is owned — and clamped — by the outer `Scrollable`.)
    let max_y = max_scroll_y(r);
    if r.viewport.scroll_offset_y > max_y {
        r.viewport.scroll_offset_y = max_y;
    }
}

pub fn zoom_in(r: &mut Resonance) {
    r.viewport.zoom = (r.viewport.zoom * 1.5).min(1000.0);
}

pub fn zoom_out(r: &mut Resonance) {
    r.viewport.zoom = (r.viewport.zoom / 1.5).max(10.0);
}
