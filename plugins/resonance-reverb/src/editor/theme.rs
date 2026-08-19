//! Shared palette — re-exports the canonical lavender palette from
//! `wayland_plugin_gui::theme` so every Resonance plugin feels like one
//! product (ba todo #1338). The reverb-specific extensions are
//! `TAIL_GLOW`, a translucent accent used to fill the analytic decay
//! polygon in the impulse view, and `ER_SPIKE` for early reflections,
//! which reads as a distinct layer in front of the tail because it is a
//! bright line over a dim fill.
//!
//! The local `apply()` is gone: it only existed to pass `ACCENT_DIM` as
//! the selection fill, which is what the shared `apply()` uses.

use wayland_plugin_gui::egui;

pub use wayland_plugin_gui::theme::lavender::*;

/// Filled-polygon colour for the analytic decay envelope in the impulse
/// view — the shared accent glow (accent at ~25 % alpha).
pub const TAIL_GLOW: egui::Color32 = ACCENT_GLOW;

/// Early-reflection spikes, in front of the tail polygon.
pub const ER_SPIKE: egui::Color32 = ACCENT_SOFT;
