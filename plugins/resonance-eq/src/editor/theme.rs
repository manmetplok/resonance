//! Editor-local theme constants — re-exports the shared classic palette
//! from `wayland_plugin_gui::theme` so all plugins feel like they belong
//! to the same product.

use wayland_plugin_gui::egui;

pub use wayland_plugin_gui::theme::classic::*;

pub const GOOD: egui::Color32 = egui::Color32::from_rgb(0x7f, 0xdd, 0x7f);
