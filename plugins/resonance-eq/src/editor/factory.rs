//! Editor factory and runtime handle for the Resonance EQ plugin.

use std::sync::Arc;

use resonance_plugin::editor_host::RuntimeEditorHandle;
use resonance_plugin::gui::{EditorFactory, PluginEditor};
use wayland_plugin_gui::{Editor as RuntimeEditor, EditorOptions};

use resonance_plugin::presets::PresetSession;

use crate::analyzer::AnalyzerState;
use crate::params::EqParams;

use super::app::EqEditorApp;

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

pub struct EqEditorFactory {
    params: Arc<EqParams>,
    analyzer: Arc<AnalyzerState>,
    presets: Arc<PresetSession>,
}

impl EqEditorFactory {
    pub fn new(
        params: Arc<EqParams>,
        analyzer: Arc<AnalyzerState>,
        presets: Arc<PresetSession>,
    ) -> Self {
        Self {
            params,
            analyzer,
            presets,
        }
    }
}

impl EditorFactory for EqEditorFactory {
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
        let app = EqEditorApp::new(
            self.params.clone(),
            self.analyzer.clone(),
            self.presets.clone(),
        );
        let runtime = RuntimeEditor::new(
            app,
            EditorOptions {
                title: "Resonance EQ".to_string(),
                app_id: "com.resonance.eq".to_string(),
                initial_size: (960, 540),
                min_size: (720, 420),
                resizable: true,
            },
        )
        .ok()?;
        Some(Box::new(RuntimeEditorHandle::new(runtime)))
    }
}
