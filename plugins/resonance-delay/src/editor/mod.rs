//! Delay plugin editor: an egui UI hosted in `wayland-plugin-gui`.
//!
//! Layout: a top header with title + preset picker + readouts + freeze
//! indicator, a bottom control strip, and a centre echo-view visualisation.

mod app;
pub mod controls;
mod echo_view;
mod factory;
mod theme;

pub use factory::{DelayEditorFactory, MIN_H, MIN_W, WINDOW_H, WINDOW_W};

/// The delay editor driven headless at `size`, with a recording
/// announcer — for `tests/editor_*.rs`. Not plugin API.
#[doc(hidden)]
pub fn headless_editor(
    plugin: &crate::ResonanceDelay,
    size: (f32, f32),
) -> resonance_plugin::editor_widgets::headless::HeadlessEditor {
    let factory = DelayEditorFactory::new(
        plugin.params.clone(),
        plugin.viz.clone(),
        plugin.presets.clone(),
        resonance_plugin::EditAnnouncer::new(),
    );
    resonance_plugin::editor_widgets::headless::HeadlessEditor::new(
        Box::new(factory.build_app()),
        size,
    )
}
