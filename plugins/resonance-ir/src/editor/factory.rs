//! Editor factory + `PluginEditor` bridge for the IR plugin.
//!
//! `IrEditorFactory` implements
//! [`resonance_plugin::gui::EditorFactory`] and constructs a
//! `wayland_plugin_gui::Editor` hosting the egui [`IrEditorApp`].
//! `RuntimeEditorHandle` adapts that runtime editor to the
//! [`PluginEditor`] trait the plugin host expects.

use std::sync::atomic::AtomicI32;
use std::sync::Arc;

use parking_lot::Mutex;
use resonance_plugin::editor_host::RuntimeEditorHandle;
use resonance_plugin::gui::{EditorFactory, PluginEditor};
use wayland_plugin_gui::{Editor as RuntimeEditor, EditorOptions};

use crate::params::IrParams;
use crate::viz::IrViz;

use super::app::IrEditorApp;

const INITIAL_SIZE: (u32, u32) = (880, 540);
const MIN_SIZE: (u32, u32) = (680, 440);

// ---------------------------------------------------------------------------
// Factory — produced by ResonanceIr::editor_factory().
// ---------------------------------------------------------------------------

pub struct IrEditorFactory {
    params: Arc<IrParams>,
    ir_name: Arc<Mutex<String>>,
    ir_info: Arc<Mutex<String>>,
    load_request: Arc<AtomicI32>,
    viz: Arc<IrViz>,
    presets: Arc<resonance_plugin::presets::PresetSession>,
}

impl IrEditorFactory {
    pub(crate) fn new(
        params: Arc<IrParams>,
        ir_name: Arc<Mutex<String>>,
        ir_info: Arc<Mutex<String>>,
        load_request: Arc<AtomicI32>,
        viz: Arc<IrViz>,
        presets: Arc<resonance_plugin::presets::PresetSession>,
    ) -> Self {
        Self {
            params,
            ir_name,
            ir_info,
            load_request,
            viz,
            presets,
        }
    }
}

impl EditorFactory for IrEditorFactory {
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
        let app = IrEditorApp {
            params: self.params.clone(),
            ir_name: self.ir_name.clone(),
            ir_info: self.ir_info.clone(),
            load_request: self.load_request.clone(),
            viz: self.viz.clone(),
            bank: resonance_plugin::presets::PresetBank::new(
                <crate::ResonanceIr as resonance_plugin::ResonancePlugin>::CLAP_ID,
                <crate::ResonanceIr as resonance_plugin::ResonancePlugin>::FACTORY_PRESETS,
            ),
            presets: self.presets.clone(),
            preset_editor: resonance_plugin::presets::PresetEditor::default(),
        };
        let runtime = RuntimeEditor::new(
            app,
            EditorOptions {
                title: "Resonance IR".to_string(),
                app_id: "com.resonance.ir".to_string(),
                initial_size: INITIAL_SIZE,
                min_size: MIN_SIZE,
                resizable: true,
            },
        )
        .ok()?;
        Some(Box::new(RuntimeEditorHandle::new(runtime)))
    }
}
