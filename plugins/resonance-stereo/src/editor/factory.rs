//! Editor factory + `PluginEditor` bridge for the stereo plugin.

use std::sync::Arc;

use resonance_plugin::editor_host::{native_api, EditorOptions, RuntimeEditor, RuntimeEditorHandle};
use resonance_plugin::gui::{EditorFactory, PluginEditor};

use crate::params::StereoParams;
use crate::viz::StereoViz;

use super::StereoEditorApp;

const WINDOW_W: u32 = 900;
const WINDOW_H: u32 = 420;

pub struct StereoEditorFactory {
    params: Arc<StereoParams>,
    viz: Arc<StereoViz>,
    presets: Arc<resonance_plugin::presets::PresetSession>,
    announcer: resonance_plugin::EditAnnouncer,
}

impl StereoEditorFactory {
    pub fn new(
        params: Arc<StereoParams>,
        viz: Arc<StereoViz>,
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

impl EditorFactory for StereoEditorFactory {
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
        let app = StereoEditorApp::new(
            self.params.clone(),
            self.viz.clone(),
            self.presets.clone(),
        );
        let runtime = RuntimeEditor::new(
            resonance_plugin::editor_host::with_announcer(app, self.announcer.clone()),
            EditorOptions {
                title: "Resonance Stereo".to_string(),
                app_id: resonance_plugin::first_party::STEREO.to_string(),
                initial_size: (WINDOW_W, WINDOW_H),
                min_size: (700, 340),
                resizable: true,
            },
        )
        .ok()?;
        Some(Box::new(RuntimeEditorHandle::new(runtime)))
    }
}
