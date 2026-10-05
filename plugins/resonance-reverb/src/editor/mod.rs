//! Reverb editor — egui UI hosted by the platform GUI runtime.
//!
//! Layout (top-down):
//! - Header: plugin name, algorithm selector, preset dropdown, live
//!   readouts, freeze indicator.
//! - Central: impulse tail hero visualisation on the left, per-algorithm tank
//!   on the right, stereo peak meters along the bottom of the central area.
//! - Bottom: control strip — the room (two rows) and the return channel
//!   (return EQ, ducking, ER/tail depth).

use std::sync::Arc;

use resonance_plugin::editor_host::{native_api, EditorOptions, RuntimeEditor, RuntimeEditorHandle};
use resonance_plugin::gui::{EditorFactory, PluginEditor};
use resonance_plugin::presets::{PresetBank, PresetEditor, PresetSession};
use resonance_plugin::preset_ui::preset_bar;
use resonance_plugin::{editor_widgets, Param};
use plugin_gui_core::{egui, widgets, EditorApp};

use crate::params::{ReverbParams, PARAM_COUNT};
use crate::viz::ReverbViz;

mod center;
mod controls;
mod impulse_view;
mod meters;
mod tank_view;
mod theme;

const WINDOW_W: u32 = 1320;
const WINDOW_H: u32 = 780;

/// The reverb editor driven headless at `size`, with a recording
/// announcer — for the plugin's tests. Not plugin API.
#[doc(hidden)]
pub fn headless_editor(
    plugin: &crate::ResonanceReverb,
    size: (f32, f32),
) -> resonance_plugin::editor_widgets::headless::HeadlessEditor {
    let app = ReverbEditorApp::new(
        plugin.params.clone(),
        plugin.viz.clone(),
        plugin.presets.clone(),
    );
    resonance_plugin::editor_widgets::headless::HeadlessEditor::new(Box::new(app), size)
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

pub struct ReverbEditorFactory {
    params: Arc<ReverbParams>,
    viz: Arc<ReverbViz>,
    presets: Arc<PresetSession>,
    announcer: resonance_plugin::EditAnnouncer,
}

impl ReverbEditorFactory {
    pub fn new(params: Arc<ReverbParams>, viz: Arc<ReverbViz>, presets: Arc<PresetSession>,
        announcer: resonance_plugin::EditAnnouncer,
    ) -> Self {
        Self {
            announcer,
            params,
            viz,
            presets,
        }
    }
}

impl EditorFactory for ReverbEditorFactory {
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
        let app = ReverbEditorApp::new(
            self.params.clone(),
            self.viz.clone(),
            self.presets.clone(),
        );
        let runtime = RuntimeEditor::new(
            resonance_plugin::editor_host::with_announcer(app, self.announcer.clone()),
            EditorOptions {
                title: "Resonance Reverb".to_string(),
                app_id: resonance_plugin::first_party::REVERB.to_string(),
                initial_size: (WINDOW_W, WINDOW_H),
                min_size: (800, 700),
                resizable: true,
            },
        )
        .ok()?;
        Some(Box::new(RuntimeEditorHandle::new(runtime)))
    }
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

pub(crate) struct ReverbEditorApp {
    pub(crate) params: Arc<ReverbParams>,
    pub(crate) viz: Arc<ReverbViz>,
    /// Factory bank + this plugin's user preset directory.
    pub(crate) bank: PresetBank,
    /// Shared with the plugin struct, so what the bar shows is what
    /// `save_state` persists.
    pub(crate) presets: Arc<PresetSession>,
    /// Transient bar state (open combo, in-progress rename), editor-only.
    pub(crate) preset_editor: PresetEditor,
}

impl ReverbEditorApp {
    pub fn new(params: Arc<ReverbParams>, viz: Arc<ReverbViz>, presets: Arc<PresetSession>) -> Self {
        Self {
            params,
            viz,
            bank: PresetBank::for_plugin::<crate::ResonanceReverb>(),
            presets,
            preset_editor: PresetEditor::default(),
        }
    }
}

impl EditorApp for ReverbEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply_once(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));

        egui::Panel::top("reverb_header")
            .exact_size(42.0)
            .show_inside(ui, |ui| draw_header(ui, self));

        egui::Panel::bottom("reverb_strip")
            .exact_size(356.0)
            .show_inside(ui, |ui| controls::draw(ui, &self.params, &self.viz));

        egui::CentralPanel::default().show_inside(ui, |ui| center::draw(ui, self));
    }
}

fn draw_header(ui: &mut egui::Ui, app: &mut ReverbEditorApp) {
    ui.horizontal_centered(|ui| {
        ui.add_space(12.0);
        ui.label(
            egui::RichText::new("RESONANCE REVERB")
                .strong()
                .color(theme::ACCENT),
        );
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        // The algorithm first: it decides which controls below apply.
        editor_widgets::choice_segmented(
            ui,
            &app.params.algorithm,
            &widgets::SegmentedStyle::LAVENDER,
        );

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);

        ui.label(egui::RichText::new("Preset").color(theme::TEXT_DIM));
        let params: Vec<&dyn Param> = (0..PARAM_COUNT)
            .map(|i| app.params.param_at(i))
            .collect();
        preset_bar(
            ui,
            "reverb_preset",
            &mut app.preset_editor,
            &app.bank,
            &app.presets,
            &params,
            "— select —",
        );

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        let decay = app
            .viz
            .synced_decay_s()
            .unwrap_or_else(|| app.params.decay.value());
        let size = app.params.size.value();
        let diff = app.params.diffusion.value();
        ui.label(egui::RichText::new(format!("RT60 {decay:>4.1} s")).color(theme::TEXT));
        ui.add_space(12.0);
        ui.label(
            egui::RichText::new(format!("Size {:>3.0}%", size * 100.0)).color(theme::TEXT_DIM),
        );
        ui.add_space(12.0);
        ui.label(
            egui::RichText::new(format!("Diffusion {:>3.0}%", diff * 100.0)).color(theme::TEXT_DIM),
        );

        // Freeze indicator pinned to the right.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(12.0);
            let frozen = app.params.freeze.value();
            let (dot_color, text_color, label) = if frozen {
                (theme::ACCENT, theme::ACCENT, "FREEZE")
            } else {
                (theme::BORDER, theme::TEXT_DIM, "freeze")
            };
            ui.label(egui::RichText::new(label).strong().color(text_color));
            ui.add_space(4.0);
            // Painted dot.
            let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 5.0, dot_color);
        });
    });
}

