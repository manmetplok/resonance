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
}

impl AmpEditorFactory {
    pub(crate) fn new(
        params: Arc<AmpParams>,
        model_name: Arc<Mutex<String>>,
        load_request: Arc<AtomicI32>,
        viz: Arc<AmpViz>,
        tone3000: Arc<WorkerHandle>,
    ) -> Self {
        Self {
            params,
            model_name,
            load_request,
            viz,
            tone3000,
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
        };
        let runtime = RuntimeEditor::new(
            app,
            EditorOptions {
                title: "Resonance Amp".to_string(),
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
        self.size
    }

    fn set_size(&mut self, width: u32, height: u32) -> bool {
        if let Some(r) = &self.runtime {
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
