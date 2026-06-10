//! Viewport, scrolling, and zoom. The periodic `Tick` handler lives in
//! `update/tick.rs`.
use iced::Task;

use crate::message::{Message, ViewportMessage};
use crate::theme;
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

pub fn scroll_y_delta(r: &mut Resonance, delta: f32) {
    r.viewport.scroll_offset_y = (r.viewport.scroll_offset_y + delta).max(0.0);
    let max_y = (r.registry.tracks.len() as f32 * theme::TRACK_HEIGHT).max(0.0);
    r.viewport.scroll_offset_y = r.viewport.scroll_offset_y.min(max_y);
}

pub fn scroll_to_x(r: &mut Resonance, x: f32) {
    let max_x = (r.viewport.timeline_content_width - r.viewport.viewport_width).max(0.0);
    r.viewport.scroll_offset = x.clamp(0.0, max_x);
}

pub fn scroll_to_y(r: &mut Resonance, y: f32) {
    r.viewport.scroll_offset_y = y.max(0.0);
    let max_y = (r.registry.tracks.len() as f32 * theme::TRACK_HEIGHT).max(0.0);
    r.viewport.scroll_offset_y = r.viewport.scroll_offset_y.min(max_y);
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
    let max_y = (h - 1.0).max(0.0);
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
