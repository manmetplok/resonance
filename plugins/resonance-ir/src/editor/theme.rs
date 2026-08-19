//! Editor-local theme constants — re-exports the canonical lavender
//! palette from `wayland_plugin_gui::theme` (ba todo #1338), plus the IR
//! waveform / response plot colours.
//!
//! The plot colours were all tints of the old blue accent; they are the
//! same tints of the canonical accent now. The alphas are unchanged, so
//! the left/right and line/fill relationships they encode are too. Tints
//! are hand-premultiplied: scale the accent's RGB by (alpha / 255).

use wayland_plugin_gui::egui;

pub use wayland_plugin_gui::theme::lavender::*;

/// Waveform trace — left channel (bright).
pub const WAVE_L: egui::Color32 = ACCENT_SOFT;
/// Waveform trace — right channel (mirrored, dimmer so the overlay reads).
pub const WAVE_R: egui::Color32 = egui::Color32::from_rgba_premultiplied(87, 69, 160, 0xa0);
/// Fill under the waveform envelope.
pub const WAVE_FILL: egui::Color32 = egui::Color32::from_rgba_premultiplied(52, 41, 96, 0x60);
/// Frequency-response line.
pub const RESPONSE_LINE: egui::Color32 = ACCENT_SOFT;
pub const RESPONSE_FILL: egui::Color32 = egui::Color32::from_rgba_premultiplied(44, 34, 80, 0x50);
