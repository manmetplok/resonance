//! Shared palette — re-exports the classic palette from
//! `wayland_plugin_gui::theme` so all Resonance plugins feel like one
//! product. Keeps a local `apply()` that historically sets a reduced set
//! of visuals (and `ACCENT_DIM` selection); preserved verbatim so the
//! editor doesn't shift visually.

use wayland_plugin_gui::egui;

pub use wayland_plugin_gui::theme::classic::*;

pub const GOOD: egui::Color32 = egui::Color32::from_rgb(0x6a, 0xe6, 0x8a);

pub fn apply(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.window_fill = PANEL;
    visuals.panel_fill = BG;
    visuals.override_text_color = Some(TEXT);
    visuals.faint_bg_color = PANEL;
    visuals.extreme_bg_color = PANEL;
    visuals.widgets.noninteractive.bg_fill = PANEL;
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT_DIM);
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    visuals.widgets.inactive.bg_fill = PANEL_LIGHT;
    visuals.widgets.inactive.weak_bg_fill = PANEL_LIGHT;
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.5, ACCENT);
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.5, ACCENT);
    visuals.selection.bg_fill = ACCENT_DIM;
    visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT);
    ctx.set_visuals(visuals);
}
