//! Editor-local theme constants — re-exports the shared classic palette
//! from `wayland_plugin_gui::theme`, plus the IR waveform / response
//! plot colours.

use wayland_plugin_gui::egui;

pub use wayland_plugin_gui::theme::classic::*;

/// Waveform trace — left channel (bright).
pub const WAVE_L: egui::Color32 = egui::Color32::from_rgb(0xa8, 0xe1, 0xff);
/// Waveform trace — right channel (mirrored, dimmer so the overlay reads).
pub const WAVE_R: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x5a, 0xc8, 0xfa, 0xa0);
/// Fill under the waveform envelope.
pub const WAVE_FILL: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x16, 0x37, 0x45, 0x60);
/// Frequency-response line.
pub const RESPONSE_LINE: egui::Color32 = egui::Color32::from_rgb(0xa8, 0xe1, 0xff);
pub const RESPONSE_FILL: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0x16, 0x37, 0x45, 0x50);
