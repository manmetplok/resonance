//! Arrange-view viewport (scroll + zoom + reported size) and the
//! top-level [`ViewMode`] tab enum.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Arrange,
    Mixer,
    Compose,
    /// Full-screen, distraction-free live chord teleprompter. Entered and
    /// exited manually only (button + `F` / `Esc`); never auto-opens on
    /// record-arm. Switching to/from it preserves transport state.
    Performance,
}

/// Widget id of the arrange view's outer horizontal `Scrollable`, the
/// target of playhead follow's `scroll_to` (review FU-D1).
pub const ARRANGE_SCROLL_ID: iced::widget::Id = iced::widget::Id::new("arrange-timeline-scroll");

/// Horizontal and vertical scroll position of the arrange-view timeline.
/// `viewport_width` / `timeline_content_width` / `_height` are reported back
/// from the canvas after layout.
#[derive(Debug, Clone)]
pub struct ArrangeViewport {
    /// Horizontal zoom in pixels per second.
    pub zoom: f32,
    /// Live horizontal scroll offset (content px) of the outer arrange
    /// `Scrollable` ([`ARRANGE_SCROLL_ID`]), mirrored from its `on_scroll`
    /// viewport — and set optimistically when playhead follow issues a
    /// `scroll_to`. The `Scrollable` owns the scroll; this is only its
    /// read-back. The canvas works in content coordinates, so pointer →
    /// sample conversions must never add this (review VIEW-10).
    pub scroll_offset: f32,
    /// Visible width (px) of the arrange `Scrollable`, from `on_scroll`.
    /// `0.0` until the first report.
    pub visible_width: f32,
    /// Content width (px) of the arrange `Scrollable`, from `on_scroll`
    /// (the fixed canvas width) — the clamp for follow's `scroll_to`.
    pub scroll_content_width: f32,
    /// Playhead follow is paused because the user scrolled the arrange
    /// view by hand during playback. Cleared whenever the transport is
    /// stopped, so the next playback follows again.
    pub follow_paused: bool,
    /// The offset follow last asked the `Scrollable` for, until its
    /// `on_scroll` echo arrives — so that echo isn't mistaken for a
    /// manual scroll.
    pub follow_pending_x: Option<f32>,
    pub scroll_offset_y: f32,
    pub viewport_width: f32,
    /// On-screen height (in pixels) of the timeline canvas viewport.
    /// Reported by `TimelineCanvas::report_viewport`; used by the
    /// track-header column to bottom-side virtualize the manual lane
    /// list (rows below `scroll_offset_y + viewport_height` are skipped
    /// during `view_track_headers`).
    pub viewport_height: f32,
    pub timeline_content_width: f32,
    pub timeline_content_height: f32,
    /// Whether the global tracks area (tempo, time signature) is expanded.
    pub global_tracks_expanded: bool,
}

impl Default for ArrangeViewport {
    fn default() -> Self {
        Self {
            zoom: 100.0,
            scroll_offset: 0.0,
            visible_width: 0.0,
            scroll_content_width: 0.0,
            follow_paused: false,
            follow_pending_x: None,
            scroll_offset_y: 0.0,
            viewport_width: 1000.0,
            viewport_height: 0.0,
            timeline_content_width: 1000.0,
            timeline_content_height: 0.0,
            global_tracks_expanded: false,
        }
    }
}
