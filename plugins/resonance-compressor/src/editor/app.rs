//! Main Compressor editor app: state struct, `EditorApp` impl, header,
//! center visualisation orchestration, and preset loading.

use std::sync::Arc;

use resonance_plugin::preset_ui::preset_bar;
use resonance_plugin::presets::{PresetBank, PresetEditor, PresetSession};
use resonance_plugin::Param;
use plugin_gui_core::{egui, EditorApp};

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
            bank: PresetBank::for_plugin::<crate::ResonanceCompressor>(),
            presets,
            preset_editor: PresetEditor::default(),
        }
    }
}

impl EditorApp for CompressorEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply_once(ui.ctx());
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
         not this track. The IN meter still shows this track's input; the KEY \
         meter shows the level the detector is working from, against the threshold."
    } else {
        "No sidechain key is connected: the compressor keys off its own input, \
         which is what the IN/DET meter shows."
    });
}

/// Width of one vertical meter, and the gap between two of them. Three
/// meters at these numbers is exactly the 150 px block the layout used
/// before the key meter existed, so connecting a key widens the block
/// instead of squeezing the bars that were already there.
const METER_W: f32 = 46.0;
const METER_GAP: f32 = 6.0;

fn meter_block_width(count: usize) -> f32 {
    count as f32 * METER_W + (count as f32 - 1.0) * METER_GAP
}

fn draw_center(ui: &mut egui::Ui, app: &mut CompressorEditorApp) {
    let avail = ui.available_rect_before_wrap();
    let gap = 8.0f32;

    let detector = app.viz.detector_source();
    let threshold = app.params.threshold.value();
    // `None` unless the host has a key connected, which is also what
    // decides whether the key meter exists at all — so the row cannot
    // show a key bar for a key that is not there.
    let key_db = app.viz.read_key_db();
    let meter_block_width = meter_block_width(if key_db.is_some() { 4 } else { 3 });

    // Split the center row horizontally: transfer curve (~40%),
    // GR history (~flex), and the meters on the right.
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
            threshold,
            ratio: app.params.ratio.value(),
            knee: app.params.knee.value(),
            makeup: app.params.makeup.value(),
            current_gr_db: app.viz.read_gr_db(),
            current_input_db: app.viz.read_input_db(),
        },
    );

    history::draw(&painter, history_rect, &app.viz);

    // Meters side by side in meters_rect, in signal order: what comes
    // in, what the detector hears if that is something else, what the
    // compressor did about it, what came out.
    let mut left = meters_rect.min.x;
    let mut next_meter = || {
        let rect = egui::Rect::from_min_max(
            egui::pos2(left, meters_rect.min.y),
            egui::pos2(left + METER_W, meters_rect.max.y),
        );
        left += METER_W + METER_GAP;
        rect
    };

    // The threshold is compared against the detector's source and
    // nothing else, so its marker belongs on that meter: the input's bar
    // when the compressor keys off itself, the key's when a key is
    // connected.
    meters::draw_input_meter(
        &painter,
        next_meter(),
        app.viz.read_input_db(),
        detector,
        (!detector.key_connected()).then_some(threshold),
    );
    if let Some(key_db) = key_db {
        meters::draw_key_meter(&painter, next_meter(), key_db, threshold);
    }
    meters::draw_gr_meter(&painter, next_meter(), app.viz.read_gr_db());
    meters::draw_output_meter(&painter, next_meter(), app.viz.read_output_db());
}

