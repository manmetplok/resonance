//! Main Compressor editor app: state struct, `EditorApp` impl, header,
//! center visualisation orchestration, and preset loading.

use std::sync::Arc;

use resonance_plugin::preset_ui::preset_bar;
use resonance_plugin::presets::{PresetBank, PresetEditor, PresetSession};
use resonance_plugin::Param;
use wayland_plugin_gui::{egui, EditorApp};

use crate::params::CompressorParams;
use crate::viz::{CompressorViz, DetectorSource};

use super::{control_strip, curve, history, meters, theme};

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

pub(crate) struct CompressorEditorApp {
    pub(crate) params: Arc<CompressorParams>,
    pub(crate) viz: Arc<CompressorViz>,
    /// Factory bank + this plugin's user preset directory.
    pub(crate) bank: PresetBank,
    /// Shared with the plugin struct, so what the bar shows is what
    /// `save_state` persists.
    pub(crate) presets: Arc<PresetSession>,
    /// Transient bar state (open combo, in-progress rename), editor-only.
    pub(crate) preset_editor: PresetEditor,
}

impl CompressorEditorApp {
    pub fn new(
        params: Arc<CompressorParams>,
        viz: Arc<CompressorViz>,
        presets: Arc<PresetSession>,
    ) -> Self {
        Self {
            params,
            viz,
            bank: PresetBank::new(
                <crate::ResonanceCompressor as resonance_plugin::ResonancePlugin>::CLAP_ID,
                <crate::ResonanceCompressor as resonance_plugin::ResonancePlugin>::FACTORY_PRESETS,
            ),
            presets,
            preset_editor: PresetEditor::default(),
        }
    }
}

impl EditorApp for CompressorEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));

        egui::Panel::top("comp_header")
            .exact_size(40.0)
            .show_inside(ui, |ui| draw_header(ui, self));

        egui::Panel::bottom("comp_strip")
            .exact_size(110.0)
            .show_inside(ui, |ui| control_strip::draw_control_strip(ui, self));

        egui::CentralPanel::default().show_inside(ui, |ui| draw_center(ui, self));
    }
}

fn draw_header(ui: &mut egui::Ui, app: &mut CompressorEditorApp) {
    ui.horizontal_centered(|ui| {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("RESONANCE COMPRESSOR")
                .strong()
                .color(theme::ACCENT),
        );
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        ui.label(egui::RichText::new("Preset").color(theme::TEXT_DIM));
        let params: Vec<&dyn Param> = (0..crate::params::PARAM_COUNT)
            .map(|i| app.params.param_at(i))
            .collect();
        preset_bar(
            ui,
            "comp_preset",
            &mut app.preset_editor,
            &app.bank,
            &app.presets,
            &params,
            "— select —",
        );

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);
        draw_detector_pill(ui, app.viz.detector_source());
    });
}

/// States, in words, which signal the detector is listening to. Without
/// this the only clue that a key is connected is the GR meter moving
/// while the input meter sits still, which reads as a malfunction.
fn draw_detector_pill(ui: &mut egui::Ui, detector: DetectorSource) {
    let (color, dot) = if detector.key_connected() {
        (theme::ACCENT, "\u{25cf}")
    } else {
        (theme::TEXT_DIM, "\u{25cb}")
    };
    ui.label(egui::RichText::new(dot).color(color).size(9.0));
    ui.add_space(3.0);
    let text = egui::RichText::new(detector.header_text()).color(color);
    let text = if detector.key_connected() {
        text.strong()
    } else {
        text
    };
    ui.label(text).on_hover_text(if detector.key_connected() {
        "An external sidechain key is connected: gain reduction follows the key, \
         not this track. The IN meter still shows this track's input."
    } else {
        "No sidechain key is connected: the compressor keys off its own input, \
         which is what the IN/DET meter shows."
    });
}

fn draw_center(ui: &mut egui::Ui, app: &mut CompressorEditorApp) {
    let avail = ui.available_rect_before_wrap();
    let meter_block_width = 150.0f32;
    let gap = 8.0f32;

    // Split the center row horizontally: transfer curve (~40%),
    // GR history (~flex), and three meters on the right.
    let curve_width = avail.width() * 0.36;
    let curve_rect = egui::Rect::from_min_max(
        avail.min,
        egui::pos2(avail.min.x + curve_width, avail.max.y),
    );
    let history_rect = egui::Rect::from_min_max(
        egui::pos2(curve_rect.max.x + gap, avail.min.y),
        egui::pos2(avail.max.x - meter_block_width - gap, avail.max.y),
    );
    let meters_rect = egui::Rect::from_min_max(
        egui::pos2(avail.max.x - meter_block_width, avail.min.y),
        avail.max,
    );

    let painter = ui.painter_at(avail);

    curve::draw(
        &painter,
        curve_rect,
        curve::CurveParams {
            // The plot window comes off the threshold parameter itself,
            // so the threshold indicator can never leave the plot.
            axis: curve::DbAxis::from_threshold(&app.params.threshold),
            threshold: app.params.threshold.value(),
            ratio: app.params.ratio.value(),
            knee: app.params.knee.value(),
            makeup: app.params.makeup.value(),
            current_gr_db: app.viz.read_gr_db(),
            current_input_db: app.viz.read_input_db(),
        },
    );

    history::draw(&painter, history_rect, &app.viz);

    // Three meters side by side in meters_rect.
    let meter_gap = 6.0f32;
    let meter_w = (meters_rect.width() - 2.0 * meter_gap) / 3.0;
    let in_rect = egui::Rect::from_min_max(
        meters_rect.min,
        egui::pos2(meters_rect.min.x + meter_w, meters_rect.max.y),
    );
    let gr_rect = egui::Rect::from_min_max(
        egui::pos2(in_rect.max.x + meter_gap, meters_rect.min.y),
        egui::pos2(in_rect.max.x + meter_gap + meter_w, meters_rect.max.y),
    );
    let out_rect = egui::Rect::from_min_max(
        egui::pos2(gr_rect.max.x + meter_gap, meters_rect.min.y),
        egui::pos2(gr_rect.max.x + meter_gap + meter_w, meters_rect.max.y),
    );
    meters::draw_input_meter(
        &painter,
        in_rect,
        app.viz.read_input_db(),
        app.viz.detector_source(),
    );
    meters::draw_gr_meter(&painter, gr_rect, app.viz.read_gr_db());
    meters::draw_output_meter(&painter, out_rect, app.viz.read_output_db());
}

