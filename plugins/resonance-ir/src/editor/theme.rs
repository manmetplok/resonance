//! Editor-local theme constants. Applied once per frame from the top-level
//! `ui()` method so the UI has a consistent dark look. Mirrors the amp /
//! reverb / drums editor themes — the set will likely consolidate later
//! into a shared `resonance-editor-theme` crate.

use wayland_plugin_gui::egui;

pub const BG: egui::Color32 = egui::Color32::from_rgb(0x14, 0x14, 0x18);
pub const PANEL: egui::Color32 = egui::Color32::from_rgb(0x1b, 0x1b, 0x22);
pub const PANEL_LIGHT: egui::Color32 = egui::Color32::from_rgb(0x25, 0x25, 0x2e);
pub const BORDER: egui::Color32 = egui::Color32::from_rgb(0x33, 0x33, 0x3e);
pub const TEXT: egui::Color32 = egui::Color32::from_rgb(0xe0, 0xe0, 0xe0);
pub const TEXT_DIM: egui::Color32 = egui::Color32::from_rgb(0x80, 0x80, 0x88);
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x5a, 0xc8, 0xfa);
pub const ACCENT_DIM: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0x5a, 0xc8, 0xfa, 0x60);
pub const ACCENT_GLOW: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0x5a, 0xc8, 0xfa, 0x40);
pub const WARN: egui::Color32 = egui::Color32::from_rgb(0xff, 0xb6, 0x4a);
pub const DANGER: egui::Color32 = egui::Color32::from_rgb(0xff, 0x6a, 0x6a);

/// Waveform trace — left channel (bright).
pub const WAVE_L: egui::Color32 = egui::Color32::from_rgb(0xa8, 0xe1, 0xff);
/// Waveform trace — right channel (mirrored, dimmer so the overlay reads).
pub const WAVE_R: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0x5a, 0xc8, 0xfa, 0xa0);
/// Fill under the waveform envelope.
pub const WAVE_FILL: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0x16, 0x37, 0x45, 0x60);
/// Frequency-response line.
pub const RESPONSE_LINE: egui::Color32 = egui::Color32::from_rgb(0xa8, 0xe1, 0xff);
pub const RESPONSE_FILL: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0x16, 0x37, 0x45, 0x50);

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
