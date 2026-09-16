//! Pure geometry of the hero band (ba todo #1265): the plot rect, the
//! time and pitch axis mappings, and the delay-tap grab zone.
//!
//! Nothing here paints or reads a parameter — it is a function of the
//! band rect and the effective delay, which is what makes it testable
//! without an egui context and what lets the draw, cloud and interact
//! layers agree on exactly one mapping.

use plugin_gui_core::egui;

/// Right-hand pitch-ruler gutter width, px.
pub(super) const RULER_W: f32 = 54.0;

/// Half-width of the delay-tap hit zone, px (the interaction layer's
/// grab region; the cursor affordance uses the same rect).
pub const TAP_HIT_HALF_W: f32 = 14.0;

/// Pitch-axis half-range, semitones (±24 st ruler).
pub const PITCH_RANGE_ST: f32 = 24.0;

/// Frame layout + axis mapping of the hero band, rebuilt every frame
/// (pure function of the band rect and the effective delay). Shared by
/// the draw, cloud and interact layers so they cannot drift apart on
/// where a millisecond or a semitone lands.
#[derive(Debug, Clone, Copy)]
pub struct HeroLayout {
    /// The whole hero band.
    pub canvas: egui::Rect,
    /// The plot area (canvas minus the pitch-ruler gutter); the write
    /// head sits at its right edge.
    pub plot: egui::Rect,
    /// Visible buffer window behind the write head, seconds.
    pub window_seconds: f32,
    /// Delay-tap x position, px.
    pub tap_x: f32,
    /// Delay-tap grab zone (full plot height, ±[`TAP_HIT_HALF_W`]).
    pub tap_hit: egui::Rect,
}

impl HeroLayout {
    /// Build the layout for a band rect and the current effective
    /// delay. The visible window scales with the delay (2× the tap,
    /// clamped to 1.5 – 4 s) so the tap always sits mid-view.
    pub fn new(canvas: egui::Rect, delay_ms: f32) -> Self {
        let plot = egui::Rect::from_min_max(
            canvas.min,
            egui::pos2(canvas.right() - RULER_W, canvas.bottom()),
        );
        let window_seconds = (delay_ms * 0.001 * 2.0).clamp(1.5, crate::dsp::MAX_DELAY_SECONDS);
        let mut layout = Self {
            canvas,
            plot,
            window_seconds,
            tap_x: 0.0,
            tap_hit: egui::Rect::NOTHING,
        };
        layout.tap_x = layout.x_of_ms(delay_ms);
        layout.tap_hit = egui::Rect::from_min_max(
            egui::pos2(layout.tap_x - TAP_HIT_HALF_W, plot.top()),
            egui::pos2(layout.tap_x + TAP_HIT_HALF_W, plot.bottom()),
        );
        layout
    }

    /// X coordinate of `ms` behind the write head (head at the right
    /// edge, older content to the left).
    pub fn x_of_ms(&self, ms: f32) -> f32 {
        self.plot.left() + self.plot.width() * (1.0 - ms * 0.001 / self.window_seconds)
    }

    /// Milliseconds behind the write head at `x`.
    pub fn ms_of_x(&self, x: f32) -> f32 {
        (1.0 - (x - self.plot.left()) / self.plot.width()) * self.window_seconds * 1000.0
    }

    /// Y coordinate of a pitch offset in semitones (0 st slightly above
    /// centre, +24 up, −24 down).
    pub fn y_of_st(&self, st: f32) -> f32 {
        self.mid_y() - st * (self.canvas.height() * 0.38) / PITCH_RANGE_ST
    }

    /// Pitch offset (semitones) at `y`.
    pub fn st_of_y(&self, y: f32) -> f32 {
        (self.mid_y() - y) * PITCH_RANGE_ST / (self.canvas.height() * 0.38)
    }

    /// The 0 st axis (backdrop midline).
    pub fn mid_y(&self) -> f32 {
        self.canvas.top() + self.canvas.height() * 0.52
    }
}
