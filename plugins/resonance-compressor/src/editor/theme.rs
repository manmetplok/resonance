//! Shared palette for the compressor editor — re-exports the classic
//! palette from `wayland_plugin_gui::theme` so all plugins feel like they
//! belong to the same product, plus the gain-reduction meter colours.

use wayland_plugin_gui::egui;

pub use wayland_plugin_gui::theme::classic::*;

/// Gain-reduction meter (same warm amber as the shared `WARN`).
pub const GR: egui::Color32 = egui::Color32::from_rgb(0xff, 0xb6, 0x4a);
pub const GR_GLOW: egui::Color32 = egui::Color32::from_rgba_premultiplied(0xff, 0xb6, 0x4a, 0x40);
