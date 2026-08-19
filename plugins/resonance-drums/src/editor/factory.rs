//! Editor factory + `PluginEditor` bridge for the drums plugin.
//!
//! `DrumsEditorFactory` implements
//! [`resonance_plugin::gui::EditorFactory`] and constructs a
//! `wayland_plugin_gui::Editor` hosting the egui [`DrumsEditorApp`].
//! `RuntimeEditorHandle` adapts that runtime editor to the
//! [`PluginEditor`] trait the plugin host expects.

use std::sync::Arc;

use resonance_plugin::editor_host::RuntimeEditorHandle;
use resonance_plugin::gui::{EditorFactory, PluginEditor};
use wayland_plugin_gui::{Editor as RuntimeEditor, EditorOptions};

use crate::download::WorkerHandle;
use crate::params::DrumParams;
use crate::KitBridge;

use super::app::DrumsEditorApp;

const INITIAL_SIZE: (u32, u32) = (720, 440);
const MIN_SIZE: (u32, u32) = (560, 360);

// ---------------------------------------------------------------------------
// Factory — produced by ResonanceDrums::editor_factory().
// ---------------------------------------------------------------------------

pub struct DrumsEditorFactory {
    params: Arc<DrumParams>,
    bridge: KitBridge,
    download_worker: Arc<WorkerHandle>,
    presets: Arc<resonance_plugin::presets::PresetSession>,
}

impl DrumsEditorFactory {
    pub(crate) fn new(
        params: Arc<DrumParams>,
        bridge: KitBridge,
        download_worker: Arc<WorkerHandle>,
        presets: Arc<resonance_plugin::presets::PresetSession>,
    ) -> Self {
        Self {
            params,
            bridge,
            download_worker,
            presets,
        }
    }
}

impl EditorFactory for DrumsEditorFactory {
    fn supports(&self, api_name: &str, is_floating: bool) -> bool {
        is_floating && api_name == "wayland"
    }

    fn preferred(&self) -> Option<(&'static str, bool)> {
        Some(("wayland", true))
    }

    fn preferred_size(&self) -> (u32, u32) {
        INITIAL_SIZE
    }

    fn create(&self, api_name: &str, is_floating: bool) -> Option<Box<dyn PluginEditor>> {
        if !self.supports(api_name, is_floating) {
            return None;
        }
        let app = DrumsEditorApp::new(
            self.params.clone(),
            self.bridge.clone(),
            self.download_worker.clone(),
            self.presets.clone(),
        );
        let runtime = RuntimeEditor::new(
            app,
            EditorOptions {
                title: "Resonance Drums".to_string(),
                app_id: "com.resonance.drums".to_string(),
                initial_size: INITIAL_SIZE,
                min_size: MIN_SIZE,
                resizable: true,
            },
        )
        .ok()?;
        Some(Box::new(RuntimeEditorHandle::new(runtime)))
    }
}
