//! Viewport, scrolling, and zoom. The periodic `Tick` handler lives in
//! `update/tick.rs`.
use iced::Task;

use crate::message::{Message, ViewportMessage};
use crate::Resonance;

/// Route a `ViewportMessage` to the appropriate handler.
pub fn handle(r: &mut Resonance, m: ViewportMessage) -> Task<Message> {
    match m {
        ViewportMessage::ZoomIn => zoom_in(r),
        ViewportMessage::ZoomOut => zoom_out(r),
        ViewportMessage::ScrollY(delta) => scroll_y_delta(r, delta),
        ViewportMessage::ScrollToX(x) => scroll_to_x(r, x),
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

pub fn scroll_to_x(r: &mut Resonance, x: f32) {
    let max_x = (r.viewport.timeline_content_width - r.viewport.viewport_width).max(0.0);
    r.viewport.scroll_offset = x.clamp(0.0, max_x);
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
    // Re-clamp scroll offsets if content shrank.
    let max_x = (w - r.viewport.viewport_width).max(0.0);
    if r.viewport.scroll_offset > max_x {
        r.viewport.scroll_offset = max_x;
    }
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
