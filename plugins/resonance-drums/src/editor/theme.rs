//! Editor theme — the shared lavender palette from
//! `wayland_plugin_gui::theme` (canonical Resonance tokens, applied once
//! per frame from the top-level `ui()` method) plus drums-local aliases
//! and typography helpers.

use wayland_plugin_gui::egui;

pub use wayland_plugin_gui::theme::lavender::*;

// The `TEXT` / `DANGER` aliases `download_panel` and a few other older
// modules use are the shared palette's own legacy names now (ba todo
// #1338 promoted them when seven more editors arrived needing the same
// two), so this module no longer restates them.

// ---------- Typography ----------
/// Standard body / hint text size used across the editor.
pub const BODY_SIZE: f32 = 11.0;

/// Build a body-text hint with the standard dim color.
pub fn hint_text(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into())
        .size(BODY_SIZE)
        .color(TEXT_3)
}
