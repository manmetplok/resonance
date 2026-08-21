//! Editor factory + `PluginEditor` bridge (pattern:
//! `plugins/resonance-delay/src/editor/factory.rs`).
//!
//! `GranularEditorFactory` implements
//! [`resonance_plugin::gui::EditorFactory`] and constructs a
//! [`RuntimeEditor`] (the platform GUI runtime's editor) hosting the egui
//! [`GranularEditorApp`]; `RuntimeEditorHandle` adapts it to the
//! [`PluginEditor`] trait the plugin host expects.

use std::sync::Arc;

use resonance_plugin::editor_host::{native_api, EditorOptions, RuntimeEditor, RuntimeEditorHandle};
use resonance_plugin::gui::{EditorFactory, PluginEditor};

use crate::params::GranularDelayParams;
use crate::viz::GranularViz;

use super::app::GranularEditorApp;

// 1320×700 (ba todo #1136, design doc #264): in family with reverb's
// 1320×660; the extra height carries the ~400 px hero buffer view.
const WINDOW_W: u32 = 1320;
const WINDOW_H: u32 = 700;

pub struct GranularEditorFactory {
    params: Arc<GranularDelayParams>,
    viz: Arc<GranularViz>,
    presets: Arc<resonance_plugin::presets::PresetSession>,
}

impl GranularEditorFactory {
    pub fn new(
        params: Arc<GranularDelayParams>,
        viz: Arc<GranularViz>,
        presets: Arc<resonance_plugin::presets::PresetSession>,
    ) -> Self {
        Self {
            params,
            viz,
            presets,
        }
    }
}

impl EditorFactory for GranularEditorFactory {
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
        let app = GranularEditorApp::new(
            self.params.clone(),
            self.viz.clone(),
            self.presets.clone(),
        );
        let runtime = RuntimeEditor::new(
            app,
            EditorOptions {
                title: "Resonance Granular Delay".to_string(),
                app_id: "com.resonance.granular-delay".to_string(),
                initial_size: (WINDOW_W, WINDOW_H),
                min_size: (1000, 560),
                resizable: true,
            },
        )
        .ok()?;
        Some(Box::new(RuntimeEditorHandle::new(runtime)))
    }
}
