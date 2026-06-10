//! Editor theme — the shared lavender palette from
//! `wayland_plugin_gui::theme` (canonical Resonance tokens, applied once
//! per frame from the top-level `ui()` method) plus wavetable-local
//! extras.

use wayland_plugin_gui::egui;

pub use wayland_plugin_gui::theme::lavender::*;

// Hand-premultiplied: scale RGB by (alpha / 255).
// ACCENT_GLOW: alpha 0x40 = 64 → 64/255 ≈ 0.251 → (35, 27, 64, 64).
pub const ACCENT_GLOW: egui::Color32 = egui::Color32::from_rgba_premultiplied(35, 27, 64, 64);

// ---------- Backwards-compatible aliases ----------
// The viz/* modules reference this older name.
pub const WARN: egui::Color32 = WARM;
