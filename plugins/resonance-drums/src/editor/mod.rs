//! Drums plugin editor: an egui UI hosted in `wayland-plugin-gui`.
//!
//! This is the migration from the old iced-based inline UI onto the same
//! editor infrastructure the wavetable plugin uses. The contents are
//! intentionally minimal placeholder controls — the real layout will be
//! designed in a follow-up. The point of this module is to wire up the
//! `EditorFactory` / `PluginEditor` plumbing so the plugin exposes a
//! floating CLAP editor window.

mod app;
mod download_panel;
mod factory;
mod header;
mod kit_browser;
mod pad_grid;
mod pad_inspector;
mod reload;
mod theme;

pub use factory::DrumsEditorFactory;

// Re-exported so the per-section modules (`pad_inspector`, `kit_browser`)
// can keep their existing `super::reload_kit` import path.
pub(crate) use reload::reload_kit;
