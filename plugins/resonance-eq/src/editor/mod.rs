//! Custom EQ editor: a frequency-response curve with draggable band nodes,
//! a per-band control strip underneath, and a factory-preset dropdown in
//! the header. Runs on an egui UI hosted by `wayland-plugin-gui`.

mod app;
mod control_strip;
mod factory;
mod nodes;
mod response;
mod theme;

pub use factory::EqEditorFactory;

/// The EQ editor driven headless at `size`, with a recording announcer
/// — for `tests/editor_*.rs`. Not plugin API.
#[doc(hidden)]
pub fn headless_editor(
    plugin: &crate::ResonanceEq,
    size: (f32, f32),
) -> resonance_plugin::editor_widgets::headless::HeadlessEditor {
    let app = EqEditorApp::new(
        plugin.params.clone(),
        plugin.analyzer_state.clone(),
        plugin.presets.clone(),
    );
    resonance_plugin::editor_widgets::headless::HeadlessEditor::new(Box::new(app), size)
}

pub(crate) use app::{AnalyzerMode, EqEditorApp};
