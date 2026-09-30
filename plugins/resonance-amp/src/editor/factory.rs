//! Editor factory + `PluginEditor` bridge for the amp plugin.
//!
//! `AmpEditorFactory` implements
//! [`resonance_plugin::gui::EditorFactory`] and constructs a
//! [`RuntimeEditor`] (the platform GUI runtime's editor) hosting the egui [`AmpEditorApp`].
//! `RuntimeEditorHandle` adapts that runtime editor to the
//! [`PluginEditor`] trait the plugin host expects.

use std::sync::atomic::AtomicI32;
use std::sync::Arc;

use parking_lot::Mutex;
use resonance_plugin::editor_host::{native_api, EditorOptions, RuntimeEditor, RuntimeEditorHandle};
use resonance_plugin::gui::{EditorFactory, PluginEditor};

use crate::params::AmpParams;
use crate::tone3000::worker::WorkerHandle;
use crate::viz::AmpViz;

use super::app::AmpEditorApp;

const INITIAL_SIZE: (u32, u32) = (960, 620);
const MIN_SIZE: (u32, u32) = (760, 520);

// ---------------------------------------------------------------------------
// Factory — produced by ResonanceAmp::editor_factory().
// ---------------------------------------------------------------------------

pub struct AmpEditorFactory {
    params: Arc<AmpParams>,
    load_request: Arc<AtomicI32>,
    viz: Arc<AmpViz>,
    presets: Arc<resonance_plugin::presets::PresetSession>,
    /// The process's one Tone3000 worker, taken on the first editor open
    /// and held for this plugin's lifetime (no worker at all for an amp
    /// whose editor is never opened).
    tone3000: Mutex<Option<Arc<WorkerHandle>>>,
}

impl AmpEditorFactory {
    pub(crate) fn new(
        params: Arc<AmpParams>,
        load_request: Arc<AtomicI32>,
        viz: Arc<AmpViz>,
        presets: Arc<resonance_plugin::presets::PresetSession>,
    ) -> Self {
        Self {
            params,
            load_request,
            viz,
            presets,
            tone3000: Mutex::new(None),
        }
    }
}

impl EditorFactory for AmpEditorFactory {
    fn supports(&self, api_name: &str, is_floating: bool) -> bool {
        is_floating && api_name == native_api()
    }

    fn preferred(&self) -> Option<(&'static str, bool)> {
        Some((native_api(), true))
    }

    fn preferred_size(&self) -> (u32, u32) {
        INITIAL_SIZE
    }

    fn create(&self, api_name: &str, is_floating: bool) -> Option<Box<dyn PluginEditor>> {
        if !self.supports(api_name, is_floating) {
            return None;
        }
        // The Library panel and the Tone3000 tab's "Installed" labels read
        // the index.
        self.params.library.ensure_scanned();
        let tone3000 = self
            .tone3000
            .lock()
            .get_or_insert_with(|| crate::tone3000::worker::shared(self.params.library.clone()))
            .clone();
        let app = AmpEditorApp::new(
            self.params.clone(),
            self.load_request.clone(),
            self.viz.clone(),
            tone3000,
            self.presets.clone(),
        );
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
