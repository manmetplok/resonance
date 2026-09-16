//! Editor palette — the canonical lavender palette from
//! `plugin_gui_core::theme` so every Resonance plugin reads as part of
//! one product (ba todo #1338; the amp was on the older blue `classic`
//! palette, which is gone).
//!
//! Amp-specific additions: `SCOPE_IN`/`SCOPE_OUT` for the two oscilloscope
//! traces, `CURVE_LINE` for the transfer-curve plot, and `TUNE_OK`/`TUNE_OFF`
//! for the tuner's in-tune/out-of-tune colour zones. Each is shaped from a
//! canonical token rather than picked freehand — they were all tints of the
//! old blue accent, and a scope trace left electric blue over a lavender
//! window is exactly what "two themes" looked like.

use plugin_gui_core::egui;

pub use plugin_gui_core::theme::lavender::*;

/// Oscilloscope: dim trace for the dry input signal — the accent at
/// alpha 0x70, hand-premultiplied (scale RGB by 112/255).
pub const SCOPE_IN: egui::Color32 = egui::Color32::from_rgba_premultiplied(61, 48, 112, 0x70);
/// Oscilloscope: bright trace for the post-model output signal.
pub const SCOPE_OUT: egui::Color32 = ACCENT_SOFT;

/// Stroke colour for the static transfer curve.
pub const CURVE_LINE: egui::Color32 = ACCENT_SOFT;
/// Fill under the transfer curve — the shared accent glow.
pub const CURVE_FILL: egui::Color32 = ACCENT_GLOW;

/// Tuner "in tune" (within a few cents of perfect).
pub const TUNE_OK: egui::Color32 = GOOD;
/// Tuner "close" (within ~15 cents).
pub const TUNE_NEAR: egui::Color32 = WARM;
/// Tuner "off" — dim, outside the close zone.
pub const TUNE_OFF: egui::Color32 = TEXT_3;
