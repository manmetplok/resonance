//! Shared palette for the compressor editor — re-exports the canonical
//! lavender palette from `wayland_plugin_gui::theme` so all plugins feel
//! like they belong to the same product (ba todo #1338), plus the
//! gain-reduction meter colours.

use wayland_plugin_gui::egui;

pub use wayland_plugin_gui::theme::lavender::*;

/// Gain-reduction meter — the shared warm token.
pub const GR: egui::Color32 = WARM;
/// Its glow: the same warm token at alpha 0x40, hand-premultiplied
/// (scale RGB by 64/255).
pub const GR_GLOW: egui::Color32 = egui::Color32::from_rgba_premultiplied(58, 49, 31, 0x40);
