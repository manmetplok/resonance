//! Wavetable plugin editor: an egui UI hosted in `wayland-plugin-gui`.
//!
//! Layout: a top tab bar switches between five tabs — OSC, ENV/FLT, LFO,
//! MOD, FX — each of which renders its own controls and canvas-based
//! visualisations. The [`WavetableEditorFactory`] implements
//! [`resonance_plugin::gui::EditorFactory`] and is returned from the
//! plugin's `editor_factory()` hook.

mod app;
mod chrome;
mod display_waves;
mod factory;
mod tabs;
mod theme;
mod viz;
mod widgets;

pub use factory::WavetableEditorFactory;

// Re-exported so the per-tab modules can keep their existing
// `crate::editor::WavetableEditorApp` import path.
pub(crate) use app::WavetableEditorApp;
