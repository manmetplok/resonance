//! Editor factory + `PluginEditor` bridge for the gate plugin.
//!
//! `GateEditorFactory` implements
//! [`resonance_plugin::gui::EditorFactory`] and constructs a
//! [`RuntimeEditor`] (the platform GUI runtime's editor) hosting the egui [`GateEditorApp`].
//! `RuntimeEditorHandle` adapts that runtime editor to the
//! [`PluginEditor`] trait the plugin host expects.

use std::sync::Arc;

use resonance_plugin::editor_host::{native_api, EditorOptions, RuntimeEditor, RuntimeEditorHandle};
use resonance_plugin::gui::{EditorFactory, PluginEditor};

use crate::params::GateParams;
use crate::viz::GateViz;

use super::GateEditorApp;

const WINDOW_W: u32 = 820;
const WINDOW_H: u32 = 320;

pub struct GateEditorFactory {
    params: Arc<GateParams>,
    viz: Arc<GateViz>,
    presets: Arc<resonance_plugin::presets::PresetSession>,
    announcer: resonance_plugin::EditAnnouncer,
}

impl GateEditorFactory {
    pub fn new(
        params: Arc<GateParams>,
        viz: Arc<GateViz>,
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

impl EditorFactory for GateEditorFactory {
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
        let app = GateEditorApp::new(
            self.params.clone(),
            self.viz.clone(),
            self.presets.clone(),
        );
        let runtime = RuntimeEditor::new(
            resonance_plugin::editor_host::with_announcer(app, self.announcer.clone()),
            EditorOptions {
                title: "Resonance Gate".to_string(),
                app_id: resonance_plugin::first_party::GATE.to_string(),
                initial_size: (WINDOW_W, WINDOW_H),
                min_size: (640, 260),
                resizable: true,
            },
        )
        .ok()?;
        Some(Box::new(RuntimeEditorHandle::new(runtime)))
    }
}
