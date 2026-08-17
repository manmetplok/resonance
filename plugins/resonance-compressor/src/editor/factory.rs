//! Editor factory and runtime handle for the Resonance Compressor plugin.

use std::sync::Arc;

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
        // The compositor owns the window size, so report what the
        // runtime last applied rather than what was last requested:
        // that is what lets the host persist and restore the size the
        // user actually left the window at (ba todo #1337). The cached
        // field is only a fallback for a handle whose runtime is gone.
        self.runtime
            .as_ref()
            .map(|r| r.get_size())
            .unwrap_or(self.size)
    }
    fn set_size(&mut self, width: u32, height: u32) -> bool {
        if let Some(r) = &mut self.runtime {
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
