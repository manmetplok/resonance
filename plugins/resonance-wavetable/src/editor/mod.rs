//! Wavetable plugin editor: an egui UI hosted in `wayland-plugin-gui`.
//!
//! Layout: a top tab bar switches between five tabs — OSC, ENV/FLT, LFO,
//! MOD, FX — each of which renders its own controls and canvas-based
//! visualisations. The [`WavetableEditorFactory`] implements
//! [`resonance_plugin::gui::EditorFactory`] and is returned from the
//! plugin's `editor_factory()` hook.
//!
//! The controls themselves come from `wayland_plugin_gui::widgets`; the
//! local `editor/widgets/` copies of the chip, the segmented control and
//! the slider went away with ba todo #1335. What stays local is the
//! *binding* layer in [`tabs`], which is the only place allowed to turn
//! a parameter into a control (ba todo #1285).

mod app;
mod chrome;
mod display_waves;
mod factory;
mod tabs;
mod theme;
mod viz;

pub use factory::WavetableEditorFactory;

// Re-exported so the per-tab modules can keep their existing
// `crate::editor::WavetableEditorApp` import path.
pub(crate) use app::WavetableEditorApp;
