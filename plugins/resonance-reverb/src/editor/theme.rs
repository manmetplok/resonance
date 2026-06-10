//! Shared palette — re-exports the classic palette from
//! `wayland_plugin_gui::theme` so every Resonance plugin feels like one
//! product. The reverb-specific extensions are `TAIL_GLOW`, a translucent
//! accent used to fill the analytic decay polygon in the impulse view,
//! and `ER_SPIKE` for early reflections. The reverb editor uses
//! `ACCENT_DIM` for the selection fill.

use wayland_plugin_gui::egui;

pub use wayland_plugin_gui::theme::classic::*;

/// Filled-polygon colour for the analytic decay envelope in the impulse
/// view — accent blue at ~25% alpha, premultiplied.
pub const TAIL_GLOW: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x16, 0x37, 0x45, 0x40);

/// A warmer accent used for early-reflection spikes so they read as a
/// distinct layer in front of the tail polygon.
pub const ER_SPIKE: egui::Color32 = egui::Color32::from_rgb(0xa8, 0xe1, 0xff);

pub fn apply(ctx: &egui::Context) {
    apply_with_selection(ctx, ACCENT_DIM);
}
