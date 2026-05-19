//! Editor factory and runtime handle for the Resonance EQ plugin.

use std::sync::Arc;

use resonance_plugin::gui::{EditorFactory, PluginEditor};
use wayland_plugin_gui::{Editor as RuntimeEditor, EditorOptions};

use crate::analyzer::AnalyzerState;
use crate::params::EqParams;

use super::app::EqEditorApp;

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

pub struct EqEditorFactory {
    params: Arc<EqParams>,
    analyzer: Arc<AnalyzerState>,
}

impl EqEditorFactory {
    pub fn new(params: Arc<EqParams>, analyzer: Arc<AnalyzerState>) -> Self {
        Self { params, analyzer }
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
        let app = EqEditorApp::new(self.params.clone(), self.analyzer.clone());
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
        Some(Box::new(RuntimeEditorHandle {
            runtime: Some(runtime),
            size: (960, 540),
        }))
    }
}

struct RuntimeEditorHandle {
    runtime: Option<RuntimeEditor>,
    size: (u32, u32),
}

impl PluginEditor for RuntimeEditorHandle {
    fn show(&mut self) {
        if let Some(r) = &self.runtime {
            r.show();
        }
    }
    fn hide(&mut self) {
        if let Some(r) = &self.runtime {
            r.hide();
        }
    }
    fn size(&self) -> (u32, u32) {
        self.size
    }
    fn set_size(&mut self, width: u32, height: u32) -> bool {
        if let Some(r) = &self.runtime {
            if r.set_size(width, height).is_ok() {
                self.size = (width, height);
                return true;
            }
        }
        false
    }
    fn can_resize(&self) -> bool {
        self.runtime
            .as_ref()
            .map(|r| r.is_resizable())
            .unwrap_or(false)
    }
}

impl Drop for RuntimeEditorHandle {
    fn drop(&mut self) {
        if let Some(r) = self.runtime.take() {
            r.destroy();
        }
    }
}
