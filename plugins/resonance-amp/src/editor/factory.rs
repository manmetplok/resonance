//! Editor factory + `PluginEditor` bridge for the amp plugin.
//!
//! `AmpEditorFactory` implements
//! [`resonance_plugin::gui::EditorFactory`] and constructs a
//! `wayland_plugin_gui::Editor` hosting the egui [`AmpEditorApp`].
//! `RuntimeEditorHandle` adapts that runtime editor to the
//! [`PluginEditor`] trait the plugin host expects.

use std::sync::atomic::AtomicI32;
use std::sync::Arc;

use parking_lot::Mutex;
use resonance_plugin::editor_host::RuntimeEditorHandle;
use resonance_plugin::gui::{EditorFactory, PluginEditor};
use wayland_plugin_gui::{Editor as RuntimeEditor, EditorOptions};

use crate::params::AmpParams;
use crate::tone3000::worker::WorkerHandle;
use crate::viz::AmpViz;

use super::app::AmpEditorApp;
use super::tone3000_panel::Tone3000PanelState;

const INITIAL_SIZE: (u32, u32) = (960, 620);
const MIN_SIZE: (u32, u32) = (760, 520);

// ---------------------------------------------------------------------------
// Factory — produced by ResonanceAmp::editor_factory().
// ---------------------------------------------------------------------------

pub struct AmpEditorFactory {
    params: Arc<AmpParams>,
    model_name: Arc<Mutex<String>>,
    load_request: Arc<AtomicI32>,
    viz: Arc<AmpViz>,
    tone3000: Arc<WorkerHandle>,
    presets: Arc<resonance_plugin::presets::PresetSession>,
}

impl AmpEditorFactory {
    pub(crate) fn new(
        params: Arc<AmpParams>,
        model_name: Arc<Mutex<String>>,
        load_request: Arc<AtomicI32>,
        viz: Arc<AmpViz>,
        tone3000: Arc<WorkerHandle>,
        presets: Arc<resonance_plugin::presets::PresetSession>,
    ) -> Self {
        Self {
            params,
            model_name,
            load_request,
            viz,
            tone3000,
            presets,
        }
    }
}

impl EditorFactory for AmpEditorFactory {
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
        let app = AmpEditorApp {
            params: self.params.clone(),
            model_name: self.model_name.clone(),
            load_request: self.load_request.clone(),
            viz: self.viz.clone(),
            tone3000: self.tone3000.clone(),
            tone3000_panel: Tone3000PanelState::default(),
            bank: resonance_plugin::presets::PresetBank::new(
                <crate::ResonanceAmp as resonance_plugin::ResonancePlugin>::CLAP_ID,
                <crate::ResonanceAmp as resonance_plugin::ResonancePlugin>::FACTORY_PRESETS,
            ),
            presets: self.presets.clone(),
            preset_editor: resonance_plugin::presets::PresetEditor::default(),
        };
        let runtime = RuntimeEditor::new(
            app,
            EditorOptions {
                title: "Resonance Amp".to_string(),
                app_id: "com.resonance.amp".to_string(),
                initial_size: INITIAL_SIZE,
                min_size: MIN_SIZE,
                resizable: true,
            },
        )
        .ok()?;
        Some(Box::new(RuntimeEditorHandle::new(runtime)))
    }
}
