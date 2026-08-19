//! Editor factory and runtime handle for the Resonance Compressor plugin.

use std::sync::Arc;

use resonance_plugin::editor_host::RuntimeEditorHandle;
use resonance_plugin::gui::{EditorFactory, PluginEditor};
use wayland_plugin_gui::{Editor as RuntimeEditor, EditorOptions};

use crate::params::CompressorParams;
use crate::viz::CompressorViz;

use super::app::CompressorEditorApp;

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

pub struct CompressorEditorFactory {
    params: Arc<CompressorParams>,
    viz: Arc<CompressorViz>,
    presets: Arc<resonance_plugin::presets::PresetSession>,
}

impl CompressorEditorFactory {
    pub fn new(
        params: Arc<CompressorParams>,
        viz: Arc<CompressorViz>,
        presets: Arc<resonance_plugin::presets::PresetSession>,
    ) -> Self {
        Self {
            params,
            viz,
            presets,
        }
    }
}

impl EditorFactory for CompressorEditorFactory {
    fn supports(&self, api_name: &str, is_floating: bool) -> bool {
        is_floating && api_name == "wayland"
    }
    fn preferred(&self) -> Option<(&'static str, bool)> {
        Some(("wayland", true))
    }
    fn preferred_size(&self) -> (u32, u32) {
        (960, 540)
    }
    fn create(&self, api_name: &str, is_floating: bool) -> Option<Box<dyn PluginEditor>> {
        if !self.supports(api_name, is_floating) {
            return None;
        }
        let app = CompressorEditorApp::new(
            self.params.clone(),
            self.viz.clone(),
            self.presets.clone(),
        );
        let runtime = RuntimeEditor::new(
            app,
            EditorOptions {
                title: "Resonance Compressor".to_string(),
                app_id: "com.resonance.compressor".to_string(),
                initial_size: (960, 540),
                min_size: (680, 400),
                resizable: true,
            },
        )
        .ok()?;
        Some(Box::new(RuntimeEditorHandle::new(runtime)))
    }
}
