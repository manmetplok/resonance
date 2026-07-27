//! Editor palette — the shared classic palette from
//! `wayland_plugin_gui::theme`, matching the other effect editors
//! (ux-guidelines.md: no per-plugin design pass).

use wayland_plugin_gui::egui;

pub use wayland_plugin_gui::theme::classic::*;

pub fn apply(ctx: &egui::Context) {
    apply_with_selection(ctx, ACCENT_DIM);
}
