//! Editor-local theme constants. Applied once per frame from the top-level
//! `ui()` method so the UI has a consistent dark look. Mirrors the wavetable
//! editor theme — the two will likely consolidate later.

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
pub const DANGER: egui::Color32 = egui::Color32::from_rgb(0xff, 0x6a, 0x6a);

// --- Typography / spacing tokens -------------------------------------------
//
// The editor uses three text sizes consistently. Pulling them into named
// constants keeps section headers (`SECTION_LABEL_SIZE`) in lock-step
// across `kit_browser`, `pad_inspector`, and any future per-section UI.

/// Big title text — the "RESONANCE DRUMS" header banner.
pub const TITLE_SIZE: f32 = 14.0;
/// All-caps section sub-labels — "PADS", "CLOSE MICS", "ARTICULATION".
pub const SECTION_LABEL_SIZE: f32 = 10.0;
/// Body / dropdown / hint text. Matches the standard egui body size in
/// the rest of the editor (was a mix of 11/12 before; consolidated to 11).
pub const BODY_SIZE: f32 = 11.0;
/// Vertical breathing room between sections in the pad inspector.
pub const SECTION_GAP: f32 = 10.0;

/// Build an all-caps section sub-label used to head a control group
/// (e.g. "CLOSE MICS", "OVERHEAD BLEND", "ARTICULATION"). Centralised so
/// every group reads with the same weight + color + size — the previous
/// code repeated the same `RichText::new(...).size(10.0).strong().color(TEXT_DIM)`
/// three times inside `pad_inspector::draw`.
pub fn section_label(text: &str) -> egui::RichText {
    egui::RichText::new(text)
        .size(SECTION_LABEL_SIZE)
        .strong()
        .color(TEXT_DIM)
}

/// Build a body-text hint with the standard dim color.
pub fn hint_text(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into())
        .size(BODY_SIZE)
        .color(TEXT_DIM)
}

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
