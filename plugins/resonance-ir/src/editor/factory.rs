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
        Some(Box::new(RuntimeEditorHandle {
            runtime: Some(runtime),
            size: INITIAL_SIZE,
        }))
    }
}

// ---------------------------------------------------------------------------
// RuntimeEditorHandle — bridges `PluginEditor` to `wayland_plugin_gui::Editor`.
// ---------------------------------------------------------------------------

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

    fn set_title(&mut self, _title: &str) {}
}

impl Drop for RuntimeEditorHandle {
    fn drop(&mut self) {
        if let Some(r) = self.runtime.take() {
            r.destroy();
        }
    }
}
