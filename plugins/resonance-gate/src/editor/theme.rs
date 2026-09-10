//! Editor palette — the canonical lavender design-system tokens from
//! `wayland_plugin_gui::theme` (ba todo #1275, finding P6).
//!
//! The gate was the one plugin that never installed any `egui::Visuals`
//! at all, so its window opened in stock egui grey beside every other
//! Resonance editor. It joins the canonical lavender palette directly
//! rather than the legacy classic one the older effect editors are
//! migrating off — see the migration note in `wayland_plugin_gui::theme`.
//! Only design-system v1 tokens; no per-plugin raw colours.

pub use plugin_gui_core::theme::lavender::*;
