//! Client-side decoration (CSD) fallback frame.
//!
//! When the compositor negotiates [`DecorationMode::Client`] (or never offers
//! server-side decorations at all), the editor window would otherwise have no
//! border, titlebar, or close button — the surface would just be the bare app
//! UI floating with no chrome. This module draws a minimal CSD frame so the
//! window is always usable regardless of the compositor: a one-pixel border, a
//! titlebar with the window title, and a close button.
//!
//! The geometry is computed by [`FrameLayout`], a pure function of the window
//! size, kept separate from any egui/Wayland state so the close-button
//! hit-testing can be exercised in unit tests without a live compositor.
//!
//! The frame is painted in *logical points* (the same coordinate space egui's
//! `screen_rect` uses), so the app UI is laid out in the inset rect returned by
//! [`FrameLayout::content_rect`].

/// Height of the CSD titlebar, in logical points.
pub const TITLEBAR_HEIGHT: f32 = 28.0;
/// Width of the CSD outer border, in logical points.
pub const BORDER_WIDTH: f32 = 1.0;
/// Side of the (square) close button hit area, in logical points.
pub const CLOSE_BUTTON_SIZE: f32 = TITLEBAR_HEIGHT;

/// Pure geometry of the CSD frame for a window of `size` logical points.
///
/// All rects are in the window's logical-point coordinate space with the
/// origin at the top-left, matching egui's `screen_rect`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameLayout {
    /// Full window size in logical points.
    pub width: f32,
    pub height: f32,
}

/// An axis-aligned rectangle in logical points: `(min_x, min_y, max_x, max_y)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameRect {
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

impl FrameRect {
    /// Whether the point `(x, y)` (logical points) is inside this rect.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.min_x && x < self.max_x && y >= self.min_y && y < self.max_y
    }
}

impl FrameLayout {
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    /// The titlebar strip, spanning the full window width below the top
    /// border, `TITLEBAR_HEIGHT` tall.
    pub fn titlebar_rect(&self) -> FrameRect {
        FrameRect {
            min_x: BORDER_WIDTH,
            min_y: BORDER_WIDTH,
            max_x: (self.width - BORDER_WIDTH).max(BORDER_WIDTH),
            max_y: BORDER_WIDTH + TITLEBAR_HEIGHT,
        }
    }

    /// The close-button hit area: a square at the right end of the titlebar.
    pub fn close_button_rect(&self) -> FrameRect {
        let right = (self.width - BORDER_WIDTH).max(BORDER_WIDTH);
        let left = (right - CLOSE_BUTTON_SIZE).max(BORDER_WIDTH);
        FrameRect {
            min_x: left,
            min_y: BORDER_WIDTH,
            max_x: right,
            max_y: BORDER_WIDTH + CLOSE_BUTTON_SIZE,
        }
    }

    /// The inset content rect the app UI is laid out in: inside the border on
    /// the left/right/bottom and below the titlebar on top.
    pub fn content_rect(&self) -> FrameRect {
        FrameRect {
            min_x: BORDER_WIDTH,
            min_y: BORDER_WIDTH + TITLEBAR_HEIGHT,
            max_x: (self.width - BORDER_WIDTH).max(BORDER_WIDTH),
            max_y: (self.height - BORDER_WIDTH).max(BORDER_WIDTH + TITLEBAR_HEIGHT),
        }
    }

    /// Whether a click at logical point `(x, y)` lands on the close button.
    ///
    /// This is the single source of truth the live paint path and the tests
    /// share, so "click the CSD close button" means exactly the same thing in
    /// both.
    pub fn is_close_click(&self, x: f32, y: f32) -> bool {
        self.close_button_rect().contains(x, y)
    }
}
