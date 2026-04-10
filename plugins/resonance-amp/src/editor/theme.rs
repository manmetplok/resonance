//! Editor-local theme constants. Applied once per frame from the top-level
//! `ui()` method so the UI has a consistent dark look. Mirrors the drums /
//! wavetable / IR editor themes — the four will likely consolidate later
//! into a shared `resonance-editor-theme` crate.

use wayland_plugin_gui::egui;

pub const BG: egui::Color32 = egui::Color32::from_rgb(0x14, 0x14, 0x18);
pub const PANEL: egui::Color32 = egui::Color32::from_rgb(0x1b, 0x1b, 0x22);
pub const PANEL_LIGHT: egui::Color32 = egui::Color32::from_rgb(0x25, 0x25, 0x2e);
pub const BORDER: egui::Color32 = egui::Color32::from_rgb(0x33, 0x33, 0x3e);
pub const TEXT: egui::Color32 = egui::Color32::from_rgb(0xe0, 0xe0, 0xe0);
pub const TEXT_DIM: egui::Color32 = egui::Color32::from_rgb(0x80, 0x80, 0x88);
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x5a, 0xc8, 0xfa);
pub const ACCENT_GLOW: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0x5a, 0xc8, 0xfa, 0x40);

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
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    visuals.widgets.hovered.bg_fill = PANEL_LIGHT;
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.5, ACCENT);
    visuals.widgets.active.bg_fill = PANEL_LIGHT;
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.5, ACCENT);
    visuals.widgets.open.bg_fill = PANEL_LIGHT;
    visuals.selection.bg_fill = ACCENT_GLOW;
    visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT);
    ctx.set_visuals(visuals);
}
