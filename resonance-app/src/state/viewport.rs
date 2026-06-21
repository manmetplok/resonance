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

/// Horizontal and vertical scroll position of the arrange-view timeline.
/// `viewport_width` / `timeline_content_width` / `_height` are reported back
/// from the canvas after layout.
#[derive(Debug, Clone)]
pub struct ArrangeViewport {
    /// Horizontal zoom in pixels per second.
    pub zoom: f32,
    pub scroll_offset: f32,
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
            scroll_offset_y: 0.0,
            viewport_width: 1000.0,
            viewport_height: 0.0,
            timeline_content_width: 1000.0,
            timeline_content_height: 0.0,
            global_tracks_expanded: false,
        }
    }
}
