//! Editor palette — the shared classic palette from
//! `wayland_plugin_gui::theme`, plus the per-channel echo colours.
//! The delay editor uses `ACCENT_DIM` for the selection fill.

use wayland_plugin_gui::egui;

pub use wayland_plugin_gui::theme::classic::*;

pub const ECHO_L: egui::Color32 = egui::Color32::from_rgb(0x50, 0xb4, 0xff);
pub const ECHO_R: egui::Color32 = egui::Color32::from_rgb(0xff, 0x82, 0x50);

pub fn apply(ctx: &egui::Context) {
    apply_with_selection(ctx, ACCENT_DIM);
}
