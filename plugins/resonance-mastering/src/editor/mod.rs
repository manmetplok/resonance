//! Mastering analyzer editor (Phase 2).
//!
//! Shows a 1/6-octave spectrum, a LUFS M/S/I strip, stereo true-peak
//! bars, correlation, crest/PLR/PSR/LRA readouts, plus rolling
//! LUFS-M and TP history traces. No processing controls yet — those
//! arrive in Phase 3 with the first DSP stages.

use std::sync::Arc;

use resonance_plugin::editor_host::{native_api, EditorOptions, RuntimeEditor, RuntimeEditorHandle};
use resonance_plugin::gui::{EditorFactory, PluginEditor};
use plugin_gui_core::{egui, EditorApp};

use crate::params::MasteringParams;
use crate::viz::MasteringViz;

pub mod controls;
mod correlation;
pub mod header;
mod lufs_history;
mod lufs_meter;
mod readouts;
mod spectrum;
mod theme;
mod tp_history;
mod tp_meter;

use controls::StageTab;

pub const WINDOW_W: u32 = 1200;
pub const WINDOW_H: u32 = 820;

pub struct MasteringEditorFactory {
    params: Arc<MasteringParams>,
    viz: Arc<MasteringViz>,
    presets: Arc<resonance_plugin::presets::PresetSession>,
}

impl MasteringEditorFactory {
    pub fn new(
        params: Arc<MasteringParams>,
        viz: Arc<MasteringViz>,
        presets: Arc<resonance_plugin::presets::PresetSession>,
    ) -> Self {
        Self {
            params,
            viz,
            presets,
        }
    }
}

impl EditorFactory for MasteringEditorFactory {
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
        let app = MasteringEditorApp::new(
            self.params.clone(),
            self.viz.clone(),
            self.presets.clone(),
        );
        let runtime = RuntimeEditor::new(
            app,
            EditorOptions {
                title: "Resonance Mastering".to_string(),
                app_id: "com.resonance.mastering".to_string(),
                initial_size: (WINDOW_W, WINDOW_H),
                min_size: (1000, 620),
                resizable: true,
            },
        )
        .ok()?;
        Some(Box::new(RuntimeEditorHandle::new(runtime)))
    }
}

pub(crate) struct MasteringEditorApp {
    params: Arc<MasteringParams>,
    viz: Arc<MasteringViz>,
    /// This plugin ships no factory presets, so the bank is the user's
    /// own directory alone — which is the whole point of ba todo #1332
    /// for the four plugins that had no preset surface at all.
    bank: resonance_plugin::presets::PresetBank,
    /// Shared with the plugin struct, so what the bar shows is what
    /// `save_state` persists.
    presets: Arc<resonance_plugin::presets::PresetSession>,
    /// Transient bar state (open combo, in-progress rename), editor-only.
    preset_editor: resonance_plugin::presets::PresetEditor,
    current_stage: StageTab,
}

impl MasteringEditorApp {
    pub fn new(
        params: Arc<MasteringParams>,
        viz: Arc<MasteringViz>,
        presets: Arc<resonance_plugin::presets::PresetSession>,
    ) -> Self {
        Self {
            params,
            viz,
            bank: resonance_plugin::presets::PresetBank::new(
                <crate::ResonanceMastering as resonance_plugin::ResonancePlugin>::CLAP_ID,
                <crate::ResonanceMastering as resonance_plugin::ResonancePlugin>::FACTORY_PRESETS,
            ),
            presets,
            preset_editor: resonance_plugin::presets::PresetEditor::default(),
            current_stage: StageTab::default(),
        }
    }
}

impl EditorApp for MasteringEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));

        egui::Panel::top("mastering_header")
            .exact_size(40.0)
                        .show_inside(ui, |ui| {
                header::draw(
                    ui,
                    &self.params,
                    &self.viz,
                    &mut self.preset_editor,
                    &self.bank,
                    &self.presets,
                )
            });

        egui::Panel::top("mastering_tabs")
            .exact_size(32.0)
            .show_inside(ui, |ui| controls::draw_tab_bar(ui, &mut self.current_stage));

        egui::Panel::bottom("mastering_histories")
            .exact_size(150.0)
            .show_inside(ui, |ui| draw_histories(ui, self));

        let controls_h = controls::stage_panel_height(self.current_stage);
        egui::Panel::bottom("mastering_controls")
            .exact_size(controls_h)
            .show_inside(ui, |ui| {
                controls::draw_stage_panel(
                    ui,
                    self.current_stage,
                    &self.params,
                    &self.viz,
                )
            });

        egui::Panel::right("mastering_right")
            .exact_size(320.0)
            .show_inside(ui, |ui| draw_right_panel(ui, self));

        egui::CentralPanel::default().show_inside(ui, |ui| draw_spectrum(ui, self));
    }
}

fn draw_spectrum(ui: &mut egui::Ui, app: &mut MasteringEditorApp) {
    let avail = ui.available_rect_before_wrap();
    let painter = ui.painter_at(avail);
    let handle_guard = app.viz.spectrum.read();
    spectrum::draw(&painter, avail, handle_guard.as_ref());
}

fn draw_right_panel(ui: &mut egui::Ui, app: &mut MasteringEditorApp) {
    let avail = ui.available_rect_before_wrap();
    let painter = ui.painter_at(avail);

    let snap = app.viz.load_snapshot();
    let target = header::reference_line_lufs(&app.params);

    // Top strip: LUFS meter (left) and TP meter (right).
    let top_h = avail.height() * 0.60;
    let top_rect = egui::Rect::from_min_max(
        egui::pos2(avail.left() + 8.0, avail.top() + 8.0),
        egui::pos2(avail.right() - 8.0, avail.top() + 8.0 + top_h),
    );
    let lufs_w = top_rect.width() * 0.56;
    let lufs_rect = egui::Rect::from_min_max(
        top_rect.min,
        egui::pos2(top_rect.left() + lufs_w, top_rect.bottom()),
    );
    let tp_rect = egui::Rect::from_min_max(
        egui::pos2(lufs_rect.right() + 8.0, top_rect.top()),
        top_rect.max,
    );
    lufs_meter::draw(
        &painter,
        lufs_rect,
        snap.momentary_lufs,
        snap.short_term_lufs,
        snap.integrated_lufs,
        target,
    );
    tp_meter::draw(
        &painter,
        tp_rect,
        snap.true_peak_left_dbtp,
        snap.true_peak_right_dbtp,
    );

    // Correlation + readouts stacked below.
    let below_top = top_rect.bottom() + 8.0;
    let corr_rect = egui::Rect::from_min_max(
        egui::pos2(avail.left() + 8.0, below_top),
        egui::pos2(avail.right() - 8.0, below_top + 56.0),
    );
    correlation::draw(&painter, corr_rect, snap.correlation);

    let readouts_rect = egui::Rect::from_min_max(
        egui::pos2(avail.left() + 8.0, corr_rect.bottom() + 8.0),
        egui::pos2(avail.right() - 8.0, avail.bottom() - 8.0),
    );
    readouts::draw(
        &painter,
        readouts_rect,
        &readouts::Readouts {
            plr_db: snap.plr_db,
            psr_db: snap.psr_db,
            crest_db: snap.crest_db,
            lra_lu: snap.lra_lu,
        },
    );
}

fn draw_histories(ui: &mut egui::Ui, app: &mut MasteringEditorApp) {
    let avail = ui.available_rect_before_wrap();
    let painter = ui.painter_at(avail);
    let target = header::reference_line_lufs(&app.params);

    // Split 60/40 between LUFS and TP traces.
    let gap = 8.0;
    let lufs_w = (avail.width() - 3.0 * gap) * 0.60;
    let lufs_rect = egui::Rect::from_min_max(
        egui::pos2(avail.left() + gap, avail.top() + gap),
        egui::pos2(avail.left() + gap + lufs_w, avail.bottom() - gap),
    );
    let tp_rect = egui::Rect::from_min_max(
        egui::pos2(lufs_rect.right() + gap, avail.top() + gap),
        egui::pos2(avail.right() - gap, avail.bottom() - gap),
    );

    lufs_history::draw(&painter, lufs_rect, &app.viz, target);
    tp_history::draw(&painter, tp_rect, &app.viz);
}
