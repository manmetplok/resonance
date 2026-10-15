//! Editor factory and runtime handle for the Resonance EQ plugin.

use std::sync::Arc;

use resonance_plugin::editor_host::{native_api, EditorOptions, RuntimeEditor, RuntimeEditorHandle};
use resonance_plugin::gui::{EditorFactory, PluginEditor};

use resonance_plugin::presets::PresetSession;

use crate::analyzer::AnalyzerState;
use crate::params::EqParams;

use super::app::EqEditorApp;

/// The editor's opening size, px. Wide enough for all eight band
/// columns side by side; tall enough for the header, a usable curve and
/// the full band strip (`app::STRIP_H`, raised with it — BUG-41).
pub const INITIAL_SIZE: (u32, u32) = (960, 804);

/// The smallest size the window may be dragged to, px. Narrower than the
/// band row, which then scrolls sideways; the header wraps to two rows.
pub const MIN_SIZE: (u32, u32) = (720, 684);

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

pub struct EqEditorFactory {
    params: Arc<EqParams>,
    analyzer: Arc<AnalyzerState>,
    presets: Arc<PresetSession>,
    announcer: resonance_plugin::EditAnnouncer,
}

impl EqEditorFactory {
    pub fn new(
        params: Arc<EqParams>,
        analyzer: Arc<AnalyzerState>,
        presets: Arc<PresetSession>,
        announcer: resonance_plugin::EditAnnouncer,
    ) -> Self {
        Self {
            announcer,
            params,
            analyzer,
            presets,
        }
    }
}

impl EditorFactory for EqEditorFactory {
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
        let app = EqEditorApp::new(
            self.params.clone(),
            self.analyzer.clone(),
            self.presets.clone(),
        );
        let runtime = RuntimeEditor::new(
            resonance_plugin::editor_host::with_announcer(app, self.announcer.clone()),
            EditorOptions {
                title: "Resonance EQ".to_string(),
                app_id: resonance_plugin::first_party::EQ.to_string(),
                initial_size: INITIAL_SIZE,
                min_size: MIN_SIZE,
                resizable: true,
            },
        )
        .ok()?;
        Some(Box::new(RuntimeEditorHandle::new(runtime)))
    }
}
