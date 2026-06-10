//! Editor palette — the shared classic palette from
//! `wayland_plugin_gui::theme` so every Resonance plugin reads as part of
//! one product.
//!
//! Amp-specific additions: `SCOPE_IN`/`SCOPE_OUT` for the two oscilloscope
//! traces, `CURVE_LINE` for the transfer-curve plot, and `TUNE_OK`/`TUNE_OFF`
//! for the tuner's in-tune/out-of-tune colour zones.

use wayland_plugin_gui::egui;

pub use wayland_plugin_gui::theme::classic::*;

/// Oscilloscope: dim trace for the dry input signal.
pub const SCOPE_IN: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x5a, 0xc8, 0xfa, 0x70);
/// Oscilloscope: bright trace for the post-model output signal.
pub const SCOPE_OUT: egui::Color32 = egui::Color32::from_rgb(0xa8, 0xe1, 0xff);

/// Stroke colour for the static transfer curve.
pub const CURVE_LINE: egui::Color32 = egui::Color32::from_rgb(0xa8, 0xe1, 0xff);
pub const CURVE_FILL: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0x16, 0x37, 0x45, 0x40);

/// Tuner "in tune" green (within a few cents of perfect).
pub const TUNE_OK: egui::Color32 = egui::Color32::from_rgb(0x7a, 0xdc, 0x8c);
/// Tuner "close" yellow (within ~15 cents).
pub const TUNE_NEAR: egui::Color32 = egui::Color32::from_rgb(0xf5, 0xd4, 0x5c);
/// Tuner "off" dim grey (outside the close zone).
pub const TUNE_OFF: egui::Color32 = egui::Color32::from_rgb(0x6b, 0x6b, 0x76);
