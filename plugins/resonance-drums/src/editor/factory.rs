//! Editor factory + `PluginEditor` bridge for the drums plugin.
//!
//! `DrumsEditorFactory` implements
//! [`resonance_plugin::gui::EditorFactory`] and constructs a
//! [`RuntimeEditor`] (the platform GUI runtime's editor) hosting the egui [`DrumsEditorApp`].
//! `RuntimeEditorHandle` adapts that runtime editor to the
//! [`PluginEditor`] trait the plugin host expects.

use std::sync::Arc;

use resonance_plugin::editor_host::{native_api, EditorOptions, RuntimeEditor, RuntimeEditorHandle};
use resonance_plugin::gui::{EditorFactory, PluginEditor};

use crate::params::DrumParams;
use crate::KitBridge;

use super::app::DrumsEditorApp;

// Matches the amp (drums-plugin-rework.md §6.1): at the old 720×440 the
// body needed about 640px of height and got 302, so the GLOBAL card was
// permanently off-screen. 960×640 gives the two-column pad body room to
// breathe; 780×520 is the floor the layout (`app.rs`) is built to survive
// without losing the KIT/GLOBAL row.
const INITIAL_SIZE: (u32, u32) = (960, 640);
const MIN_SIZE: (u32, u32) = (780, 520);

// ---------------------------------------------------------------------------
// Factory — produced by ResonanceDrums::editor_factory().
// ---------------------------------------------------------------------------

pub struct DrumsEditorFactory {
    params: Arc<DrumParams>,
    bridge: KitBridge,
    presets: Arc<resonance_plugin::presets::PresetSession>,
}

impl DrumsEditorFactory {
    pub(crate) fn new(
        params: Arc<DrumParams>,
        bridge: KitBridge,
        presets: Arc<resonance_plugin::presets::PresetSession>,
    ) -> Self {
        Self {
            params,
            bridge,
            presets,
        }
    }
}

impl EditorFactory for DrumsEditorFactory {
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
        let app = DrumsEditorApp::new(
            self.params.clone(),
            self.bridge.clone(),
            // The process-wide kit library (and its download worker), the
            // one `kit_select` resolves against: opened by whichever comes
            // first, this or the parameter, and held by the instance from
            // then on — so the library and an in-flight download outlive
            // a closed editor window, and the plugin going away drops it.
            self.params.selection.library.shared(),
            self.presets.clone(),
        );
        let runtime = RuntimeEditor::new(
            app,
            EditorOptions {
                title: "Resonance Drums".to_string(),
                app_id: resonance_plugin::first_party::DRUMS.to_string(),
                initial_size: INITIAL_SIZE,
                min_size: MIN_SIZE,
                resizable: true,
            },
        )
        .ok()?;
        Some(Box::new(RuntimeEditorHandle::new(runtime)))
    }
}
