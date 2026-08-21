//! Editor factory + `PluginEditor` bridge for the delay plugin.
//!
//! `DelayEditorFactory` implements
//! [`resonance_plugin::gui::EditorFactory`] and constructs a
//! [`RuntimeEditor`] (the platform GUI runtime's editor) hosting the egui [`DelayEditorApp`].
//! `RuntimeEditorHandle` adapts that runtime editor to the
//! [`PluginEditor`] trait the plugin host expects.

use std::sync::Arc;

use resonance_plugin::editor_host::{native_api, EditorOptions, RuntimeEditor, RuntimeEditorHandle};
use resonance_plugin::gui::{EditorFactory, PluginEditor};

use crate::params::DelayParams;
use crate::viz::DelayViz;

use super::app::DelayEditorApp;

const WINDOW_W: u32 = 1200;
const WINDOW_H: u32 = 600;

pub struct DelayEditorFactory {
    params: Arc<DelayParams>,
    viz: Arc<DelayViz>,
    presets: Arc<resonance_plugin::presets::PresetSession>,
}

impl DelayEditorFactory {
    pub fn new(
        params: Arc<DelayParams>,
        viz: Arc<DelayViz>,
        presets: Arc<resonance_plugin::presets::PresetSession>,
    ) -> Self {
        Self {
            params,
            viz,
            presets,
        }
    }
}

impl EditorFactory for DelayEditorFactory {
    fn supports(&self, api_name: &str, is_floating: bool) -> bool {
        is_floating && api_name == native_api()
    }
    fn preferred(&self) -> Option<(&'static str, bool)> {
        Some((native_api(), true))
    }
    fn preferred_size(&self) -> (u32, u32) {
        (WINDOW_W, WINDOW_H)
    }
    fn create(&self, api_name: &str, is_floating: bool) -> Option<Box<dyn PluginEditor>> {
        if !self.supports(api_name, is_floating) {
            return None;
        }
        let app = DelayEditorApp::new(
            self.params.clone(),
            self.viz.clone(),
            self.presets.clone(),
        );
        let runtime = RuntimeEditor::new(
            app,
            EditorOptions {
                title: "Resonance Delay".to_string(),
                app_id: "com.resonance.delay".to_string(),
                initial_size: (WINDOW_W, WINDOW_H),
                min_size: (900, 480),
                resizable: true,
            },
        )
        .ok()?;
        Some(Box::new(RuntimeEditorHandle::new(runtime)))
    }
}
