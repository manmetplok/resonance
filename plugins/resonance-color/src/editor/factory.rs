//! Editor factory and runtime handle for the Resonance Color plugin.

use std::sync::Arc;

use resonance_plugin::editor_host::{native_api, EditorOptions, RuntimeEditor, RuntimeEditorHandle};
use resonance_plugin::gui::{EditorFactory, PluginEditor};

use crate::params::ColorParams;
use crate::viz::ColorViz;

use super::app::ColorEditorApp;

const SIZE: (u32, u32) = (900, 520);

pub struct ColorEditorFactory {
    params: Arc<ColorParams>,
    viz: Arc<ColorViz>,
    presets: Arc<resonance_plugin::presets::PresetSession>,
    announcer: resonance_plugin::EditAnnouncer,
}

impl ColorEditorFactory {
    pub fn new(
        params: Arc<ColorParams>,
        viz: Arc<ColorViz>,
        presets: Arc<resonance_plugin::presets::PresetSession>,
        announcer: resonance_plugin::EditAnnouncer,
    ) -> Self {
        Self {
            announcer,
            params,
            viz,
            presets,
        }
    }
}

impl EditorFactory for ColorEditorFactory {
    fn supports(&self, api_name: &str, is_floating: bool) -> bool {
        is_floating && api_name == native_api()
    }
    fn preferred(&self) -> Option<(&'static str, bool)> {
        Some((native_api(), true))
    }
    fn preferred_size(&self) -> (u32, u32) {
        SIZE
    }
    fn create(&self, api_name: &str, is_floating: bool) -> Option<Box<dyn PluginEditor>> {
        if !self.supports(api_name, is_floating) {
            return None;
        }
        let app = ColorEditorApp::new(
            self.params.clone(),
            self.viz.clone(),
            self.presets.clone(),
        );
        let runtime = RuntimeEditor::new(
            resonance_plugin::editor_host::with_announcer(app, self.announcer.clone()),
            EditorOptions {
                title: "Resonance Color".to_string(),
                app_id: resonance_plugin::first_party::COLOR.to_string(),
                initial_size: SIZE,
                min_size: (720, 440),
                resizable: true,
            },
        )
        .ok()?;
        Some(Box::new(RuntimeEditorHandle::new(runtime)))
    }
}
