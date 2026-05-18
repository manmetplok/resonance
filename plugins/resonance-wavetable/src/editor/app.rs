//! The actual egui app: state, tab enum, and update/view orchestration.
//!
//! `WavetableEditorApp` is the `EditorApp` the runtime drives each frame.
//! It refreshes the audio→UI snapshot, paints the chrome panels (delegated
//! to [`super::chrome`]), and dispatches the body to the selected tab.

use std::sync::Arc;

use wayland_plugin_gui::{egui, EditorApp};

use crate::params::{WavetableParams, PARAM_COUNT};
use crate::viz::{VizSnapshot, WavetableVizState};

use super::{chrome, tabs, theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WtTab {
    Osc,
    EnvFilter,
    Lfo,
    Mod,
    Fx,
}

pub(crate) struct WavetableEditorApp {
    pub(crate) params: Arc<WavetableParams>,
    pub(crate) viz: Arc<WavetableVizState>,
    pub(crate) selected_tab: WtTab,
    pub(crate) selected_osc: usize,
    pub(crate) selected_lfo: usize,
    #[allow(dead_code)] // reserved for future "highlight selected mod slot" feature
    pub(crate) selected_mod_slot: usize,
    pub(crate) preset_idx: usize,
    /// Most recent audio→UI viz snapshot, refreshed each frame.
    pub(crate) snapshot: VizSnapshot,
}

impl WavetableEditorApp {
    pub fn new(params: Arc<WavetableParams>, viz: Arc<WavetableVizState>) -> Self {
        let snapshot = viz.read_snapshot();
        // Standalone editor harness can pick an initial tab via env var so
        // each tab can be screenshotted without manual clicking.
        let selected_tab = match std::env::var("WT_TAB").as_deref() {
            Ok("env") => WtTab::EnvFilter,
            Ok("lfo") => WtTab::Lfo,
            Ok("mod") => WtTab::Mod,
            Ok("fx") => WtTab::Fx,
            _ => WtTab::Osc,
        };
        Self {
            params,
            viz,
            selected_tab,
            selected_osc: 0,
            selected_lfo: 0,
            selected_mod_slot: 0,
            preset_idx: 0,
            snapshot,
        }
    }
}

impl EditorApp for WavetableEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        // Refresh live audio state for this frame.
        self.snapshot = self.viz.read_snapshot();
        // Drive continuous ~60 Hz repaint so live views animate.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));

        theme::apply(ui.ctx());

        // Chrome (brand + chrome icons).
        egui::Panel::top("wt_chrome")
            .exact_size(38.0)
            .frame(
                egui::Frame::default()
                    .fill(theme::BG_1)
                    .inner_margin(egui::Margin::symmetric(14, 6))
                    .stroke(egui::Stroke::new(1.0, theme::LINE_2)),
            )
            .show_inside(ui, |ui| chrome::draw_chrome(ui, self));

        // Tab bar (module nav + preset pill + voices badge).
        egui::Panel::top("wt_tabs")
            .exact_size(48.0)
            .frame(
                egui::Frame::default()
                    .fill(theme::BG_1)
                    .inner_margin(egui::Margin::symmetric(14, 6))
                    .stroke(egui::Stroke::new(1.0, theme::LINE_2)),
            )
            .show_inside(ui, |ui| chrome::draw_tab_bar(ui, self));

        // Status bar.
        egui::Panel::bottom("wt_status")
            .exact_size(28.0)
            .frame(
                egui::Frame::default()
                    .fill(theme::BG_1)
                    .inner_margin(egui::Margin::symmetric(16, 6))
                    .stroke(egui::Stroke::new(1.0, theme::LINE_2)),
            )
            .show_inside(ui, |ui| chrome::draw_status_bar(ui, self));

        // Body.
        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(theme::BG_0)
                    .inner_margin(egui::Margin::same(12)),
            )
            .show_inside(ui, |ui| match self.selected_tab {
                WtTab::Osc => tabs::osc::draw(ui, self),
                WtTab::EnvFilter => tabs::env_filter::draw(ui, self),
                WtTab::Lfo => tabs::lfo::draw(ui, self),
                WtTab::Mod => tabs::mod_matrix::draw(ui, self),
                WtTab::Fx => tabs::fx::draw(ui, self),
            });
    }
}

/// Apply a factory preset: walk every param and call `set_plain` for any
/// id that matches a key in the preset's `params` object. Missing keys
/// are ignored so older presets still load after a param is added.
pub(super) fn load_preset(params: &WavetableParams, json: &str) {
    resonance_plugin::presets::load(json, PARAM_COUNT, |i| params.param_at(i));
}

pub(super) fn peak_of(scope: &[f32]) -> f32 {
    let mut p = 0.0f32;
    for s in scope.iter() {
        let a = s.abs();
        if a > p {
            p = a;
        }
    }
    p.clamp(0.0, 1.0)
}
