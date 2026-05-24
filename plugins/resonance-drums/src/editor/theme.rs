//! Editor-local theme constants. Applied once per frame from the top-level
//! `ui()` method so the UI has a consistent dark look.
//!
//! Palette and shape tokens mirror the `Resonance Drums` design handoff:
//! a lavender accent (`#8b6dff`) for primary controls, warm amber
//! (`#e8c47b`) for modulation/balance indicators, plus state LEDs
//! (`#6dd6a3` good / `#e87b8b` bad).

use wayland_plugin_gui::egui;

// ---------- Surfaces ----------
pub const BG_0: egui::Color32 = egui::Color32::from_rgb(0x0a, 0x0b, 0x0e);
pub const BG_1: egui::Color32 = egui::Color32::from_rgb(0x15, 0x16, 0x1b);
pub const BG_2: egui::Color32 = egui::Color32::from_rgb(0x1b, 0x1d, 0x23);
pub const BG_3: egui::Color32 = egui::Color32::from_rgb(0x23, 0x26, 0x2e);

pub const LINE: egui::Color32 = egui::Color32::from_rgb(0x27, 0x2a, 0x31);
pub const LINE_2: egui::Color32 = egui::Color32::from_rgb(0x1f, 0x22, 0x29);

// ---------- Text ----------
pub const TEXT_1: egui::Color32 = egui::Color32::from_rgb(0xe8, 0xe7, 0xe3);
pub const TEXT_2: egui::Color32 = egui::Color32::from_rgb(0x9a, 0xa0, 0xac);
pub const TEXT_3: egui::Color32 = egui::Color32::from_rgb(0x5d, 0x62, 0x6d);
pub const TEXT_4: egui::Color32 = egui::Color32::from_rgb(0x3f, 0x43, 0x4c);

// ---------- Accents ----------
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x8b, 0x6d, 0xff);
pub const ACCENT_SOFT: egui::Color32 = egui::Color32::from_rgb(0xa8, 0x92, 0xff);
// Hand-premultiplied: scale RGB by (alpha / 255).
// ACCENT_DIM: alpha 0x28 = 40 → 40/255 ≈ 0.157 → (22, 17, 40, 40).
pub const ACCENT_DIM: egui::Color32 = egui::Color32::from_rgba_premultiplied(22, 17, 40, 40);

pub const WARM: egui::Color32 = egui::Color32::from_rgb(0xe8, 0xc4, 0x7b);

pub const GOOD: egui::Color32 = egui::Color32::from_rgb(0x6d, 0xd6, 0xa3);
pub const BAD: egui::Color32 = egui::Color32::from_rgb(0xe8, 0x7b, 0x8b);

// ---------- Backwards-compatible aliases ----------
// `download_panel` and a few other older modules still reference these names.
pub const PANEL: egui::Color32 = BG_2;
pub const BORDER: egui::Color32 = LINE;
pub const TEXT: egui::Color32 = TEXT_1;
pub const TEXT_DIM: egui::Color32 = TEXT_3;
pub const DANGER: egui::Color32 = BAD;

// ---------- Shape tokens ----------
pub const RADIUS_PANEL: f32 = 9.0;
pub const RADIUS_CHIP: f32 = 5.0;

// ---------- Typography ----------
/// Standard body / hint text size used across the editor.
pub const BODY_SIZE: f32 = 11.0;

/// Build a body-text hint with the standard dim color.
pub fn hint_text(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into())
        .size(BODY_SIZE)
        .color(TEXT_3)
}

pub fn apply(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.window_fill = BG_2;
    visuals.panel_fill = BG_0;
    visuals.override_text_color = Some(TEXT_1);
    visuals.faint_bg_color = BG_2;
    visuals.extreme_bg_color = BG_1;
    visuals.widgets.noninteractive.bg_fill = BG_2;
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT_3);
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, LINE_2);
    visuals.widgets.inactive.bg_fill = BG_3;
    visuals.widgets.inactive.weak_bg_fill = BG_3;
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT_1);
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, LINE);
    visuals.widgets.hovered.bg_fill = BG_3;
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, ACCENT);
    visuals.widgets.active.bg_fill = BG_3;
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.5, ACCENT);
    visuals.widgets.open.bg_fill = BG_3;
    visuals.selection.bg_fill = ACCENT_DIM;
    visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT);
    ctx.set_visuals(visuals);
}
