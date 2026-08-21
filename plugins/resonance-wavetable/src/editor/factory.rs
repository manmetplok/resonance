//! Editor factory + `PluginEditor` bridge.
//!
//! `WavetableEditorFactory` implements
//! [`resonance_plugin::gui::EditorFactory`] and constructs a
//! [`RuntimeEditor`] (the platform GUI runtime's editor) hosting the egui [`WavetableEditorApp`].
//! `RuntimeEditorHandle` adapts that runtime editor to the
//! [`PluginEditor`] trait the plugin host expects.

use std::sync::Arc;

use resonance_plugin::editor_host::{native_api, EditorOptions, RuntimeEditor, RuntimeEditorHandle};
use resonance_plugin::gui::{EditorFactory, PluginEditor};

use crate::params::WavetableParams;
use crate::viz::WavetableVizState;

use super::app::WavetableEditorApp;

// ---------------------------------------------------------------------------
// Factory — produced by ResonanceWavetable::editor_factory().
// ---------------------------------------------------------------------------

pub struct WavetableEditorFactory {
    params: Arc<WavetableParams>,
    viz: Arc<WavetableVizState>,
    presets: Arc<resonance_plugin::presets::PresetSession>,
}

impl WavetableEditorFactory {
    pub fn new(
        params: Arc<WavetableParams>,
        viz: Arc<WavetableVizState>,
        presets: Arc<resonance_plugin::presets::PresetSession>,
    ) -> Self {
        Self {
            params,
            viz,
            presets,
        }
    }
}

impl EditorFactory for WavetableEditorFactory {
    fn supports(&self, api_name: &str, is_floating: bool) -> bool {
        is_floating && api_name == native_api()
    }

    fn preferred(&self) -> Option<(&'static str, bool)> {
        Some((native_api(), true))
    }

    fn preferred_size(&self) -> (u32, u32) {
        (960, 560)
    }

    fn create(&self, api_name: &str, is_floating: bool) -> Option<Box<dyn PluginEditor>> {
        if !self.supports(api_name, is_floating) {
            return None;
        }
        let app = WavetableEditorApp::new(
            self.params.clone(),
            self.viz.clone(),
            self.presets.clone(),
        );
        let runtime = RuntimeEditor::new(
            app,
            EditorOptions {
                title: "Resonance Wavetable".to_string(),
                app_id: "com.resonance.wavetable".to_string(),
                initial_size: (960, 560),
                min_size: (720, 480),
                resizable: true,
            },
        )
        .ok()?;
        Some(Box::new(RuntimeEditorHandle::new(runtime)))
    }
}
