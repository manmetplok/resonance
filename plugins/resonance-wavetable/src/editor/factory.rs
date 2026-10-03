//! Editor factory + `PluginEditor` bridge.
//!
//! `WavetableEditorFactory` implements
//! [`resonance_plugin::gui::EditorFactory`] and constructs a
//! [`RuntimeEditor`] (the platform GUI runtime's editor) hosting the egui [`WavetableEditorApp`].
//! `RuntimeEditorHandle` adapts that runtime editor to the
//! [`PluginEditor`] trait the plugin host expects.

use std::sync::Arc;

use resonance_plugin::editor_host::{native_api, EditorOptions, RuntimeEditor, RuntimeEditorHandle};
use resonance_plugin::gui::{EditorFactory, PluginEditor};

use crate::params::WavetableParams;
use crate::user_wavetable::UserWavetables;
use crate::viz::WavetableVizState;

use super::app::WavetableEditorApp;

// ---------------------------------------------------------------------------
// Factory — produced by ResonanceWavetable::editor_factory().
// ---------------------------------------------------------------------------

pub struct WavetableEditorFactory {
    params: Arc<WavetableParams>,
    viz: Arc<WavetableVizState>,
    presets: Arc<resonance_plugin::presets::PresetSession>,
    user_tables: Arc<UserWavetables>,
    announcer: resonance_plugin::EditAnnouncer,
}

impl WavetableEditorFactory {
    pub fn new(
        params: Arc<WavetableParams>,
        viz: Arc<WavetableVizState>,
        presets: Arc<resonance_plugin::presets::PresetSession>,
        user_tables: Arc<UserWavetables>,
        announcer: resonance_plugin::EditAnnouncer,
    ) -> Self {
        Self {
            announcer,
            params,
            viz,
            presets,
            user_tables,
        }
    }

    /// The editor app, unwrapped (the headless test hook drives it with
    /// its own announcer).
    pub(crate) fn build_app(&self) -> WavetableEditorApp {
        WavetableEditorApp::new(
            self.params.clone(),
            self.viz.clone(),
            self.presets.clone(),
            self.user_tables.clone(),
        )
    }
}

impl EditorFactory for WavetableEditorFactory {
    fn supports(&self, api_name: &str, is_floating: bool) -> bool {
        is_floating && api_name == native_api()
    }

    fn preferred(&self) -> Option<(&'static str, bool)> {
        Some((native_api(), true))
    }

    fn preferred_size(&self) -> (u32, u32) {
        (960, 560)
    }

    fn create(&self, api_name: &str, is_floating: bool) -> Option<Box<dyn PluginEditor>> {
        if !self.supports(api_name, is_floating) {
            return None;
        }
        let app = self.build_app();
        let runtime = RuntimeEditor::new(
            resonance_plugin::editor_host::with_announcer(app, self.announcer.clone()),
            EditorOptions {
                title: "Resonance Wavetable".to_string(),
                app_id: "com.resonance.wavetable".to_string(),
                initial_size: (960, 560),
                min_size: (720, 480),
                resizable: true,
            },
        )
        .ok()?;
        Some(Box::new(RuntimeEditorHandle::new(runtime)))
    }
}
