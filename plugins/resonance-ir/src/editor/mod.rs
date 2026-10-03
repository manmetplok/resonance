//! IR plugin editor: an egui UI hosted in `wayland-plugin-gui`.
//!
//! Mirrors the amp / reverb editors: a Factory that produces a
//! `RuntimeEditorHandle`, which wraps the `wayland_plugin_gui::Editor`
//! and drives an `EditorApp` implementation on the editor thread.
//!
//! Layout (top → bottom):
//!
//! - Top strip: header with title, "Load IR…" button, Prev/Next and
//!   the current filename + position counter.
//! - Centre: waveform view (left) + frequency-response view (right)
//!   drawn from the `IrSnapshot` published by the loader thread, plus
//!   a stereo IN/OUT meter strip along the bottom.
//! - Bottom: the dry/wet and output-gain control strip, plus the
//!   latency-mode picker and its readout (ba todo #1300).

mod app;
mod controls;
mod factory;
mod header;
mod latency;
mod meters;
mod missing_banner;
mod response_view;
mod theme;
mod waveform_view;

pub use factory::IrEditorFactory;

// Re-exported so the per-section modules (e.g. `header.rs`) can keep
// their existing `super::IrEditorApp` import path.
pub(crate) use app::IrEditorApp;

/// The IR editor driven headless at `size`, for `tests/*.rs`. Not
/// plugin API. Builds the same `IrEditorApp` the factory does
/// (`factory.rs::create`), without a host announcer or a real GUI
/// runtime.
#[doc(hidden)]
pub fn headless_editor(
    plugin: &crate::ResonanceIr,
    size: (f32, f32),
) -> resonance_plugin::editor_widgets::headless::HeadlessEditor {
    let app = IrEditorApp {
        params: plugin.params.clone(),
        ir_name: plugin.ir_name.clone(),
        ir_info: plugin.ir_info.clone(),
        load_request: plugin.load_request.clone(),
        viz: plugin.viz.clone(),
        bank: resonance_plugin::presets::PresetBank::for_plugin::<crate::ResonanceIr>(),
        presets: plugin.presets.clone(),
        preset_editor: resonance_plugin::presets::PresetEditor::default(),
        #[cfg(not(target_os = "macos"))]
        ir_picker: parking_lot::Mutex::new(resonance_plugin::file_picker::FilePicker::default()),
    };
    resonance_plugin::editor_widgets::headless::HeadlessEditor::new(Box::new(app), size)
}
