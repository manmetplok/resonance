//! Editor theme — the shared lavender palette from
//! `wayland_plugin_gui::theme` (canonical Resonance tokens, applied once
//! per frame from the top-level `ui()` method) plus drums-local aliases
//! and typography helpers.

use wayland_plugin_gui::egui;

pub use wayland_plugin_gui::theme::lavender::*;

// ---------- Backwards-compatible aliases ----------
// `download_panel` and a few other older modules still reference these names.
pub const TEXT: egui::Color32 = TEXT_1;
pub const DANGER: egui::Color32 = BAD;

// ---------- Typography ----------
/// Standard body / hint text size used across the editor.
pub const BODY_SIZE: f32 = 11.0;

/// Build a body-text hint with the standard dim color.
pub fn hint_text(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into())
        .size(BODY_SIZE)
        .color(TEXT_3)
}
