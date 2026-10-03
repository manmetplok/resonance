//! Editor factory + `PluginEditor` bridge for the delay plugin.
//!
//! `DelayEditorFactory` implements
//! [`resonance_plugin::gui::EditorFactory`] and constructs a
//! [`RuntimeEditor`] (the platform GUI runtime's editor) hosting the egui [`DelayEditorApp`].
//! `RuntimeEditorHandle` adapts that runtime editor to the
//! [`PluginEditor`] trait the plugin host expects.

use std::sync::Arc;

use resonance_plugin::editor_host::{
    native_api, with_announcer, EditorOptions, RuntimeEditor, RuntimeEditorHandle,
};
use resonance_plugin::EditAnnouncer;
use resonance_plugin::gui::{EditorFactory, PluginEditor};

use crate::params::DelayParams;
use crate::viz::DelayViz;

use super::app::DelayEditorApp;

/// Default window size.
pub const WINDOW_W: u32 = 1200;
pub const WINDOW_H: u32 = 600;
/// Minimum window size: the strip still fits it, three rows deep
/// (`tests/editor_layout.rs`).
pub const MIN_W: u32 = 900;
pub const MIN_H: u32 = 480;

pub struct DelayEditorFactory {
    params: Arc<DelayParams>,
    viz: Arc<DelayViz>,
    presets: Arc<resonance_plugin::presets::PresetSession>,
    announcer: EditAnnouncer,
}

impl DelayEditorFactory {
    pub fn new(
        params: Arc<DelayParams>,
        viz: Arc<DelayViz>,
        presets: Arc<resonance_plugin::presets::PresetSession>,
        announcer: EditAnnouncer,
    ) -> Self {
        Self {
            params,
            viz,
            presets,
            announcer,
        }
    }

    /// The editor app, unwrapped (the headless test hook drives it with
    /// its own announcer).
    pub(crate) fn build_app(&self) -> DelayEditorApp {
        DelayEditorApp::new(self.params.clone(), self.viz.clone(), self.presets.clone())
    }
}

impl EditorFactory for DelayEditorFactory {
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
        let app = with_announcer(self.build_app(), self.announcer.clone());
        let runtime = RuntimeEditor::new(
            app,
            EditorOptions {
                title: "Resonance Delay".to_string(),
                app_id: "com.resonance.delay".to_string(),
                initial_size: (WINDOW_W, WINDOW_H),
                min_size: (MIN_W, MIN_H),
                resizable: true,
            },
        )
        .ok()?;
        Some(Box::new(RuntimeEditorHandle::new(runtime)))
    }
}
