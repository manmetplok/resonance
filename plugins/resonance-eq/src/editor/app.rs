//! Main EQ editor app: state struct, `EditorApp` impl, header drawing,
//! and preset loading.

use std::sync::Arc;

use wayland_plugin_gui::{egui, EditorApp};

use crate::analyzer::AnalyzerState;
use crate::params::EqParams;
use crate::presets::PRESETS;

use super::{control_strip, nodes, response, theme};

/// Which side of the EQ chain to display in the spectrum analyzer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AnalyzerMode {
    Off,
    Pre,
    Post,
}

// ---------------------------------------------------------------------------
// App — runs on the editor thread.
// ---------------------------------------------------------------------------

pub(crate) struct EqEditorApp {
    pub(crate) params: Arc<EqParams>,
    pub(crate) analyzer: Arc<AnalyzerState>,
    pub(crate) analyzer_mode: AnalyzerMode,
    /// Which band is highlighted in the curve/strip. None = no selection.
    pub(crate) selected_band: Option<usize>,
    /// Currently-dragged band, if any.
    pub(crate) drag_state: Option<nodes::DragState>,
}

impl EqEditorApp {
    pub fn new(params: Arc<EqParams>, analyzer: Arc<AnalyzerState>) -> Self {
        Self {
            params,
            analyzer,
            analyzer_mode: AnalyzerMode::Post,
            selected_band: None,
            drag_state: None,
        }
    }
}

impl EditorApp for EqEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply(ui.ctx());
        // Continuous repaint so response curve follows slider movement.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));

        egui::Panel::top("eq_header")
            .exact_size(40.0)
            .show_inside(ui, |ui| draw_header(ui, self));

        egui::Panel::bottom("eq_strip")
            .exact_size(160.0)
            .show_inside(ui, |ui| control_strip::draw_band_strip(ui, self));

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let rect = ui.available_rect_before_wrap();
            response::draw(ui, rect, self);
        });
    }
}

fn draw_header(ui: &mut egui::Ui, app: &mut EqEditorApp) {
    ui.horizontal_centered(|ui| {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("RESONANCE EQ")
                .strong()
                .color(theme::ACCENT),
        );
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        ui.label(egui::RichText::new("Preset").color(theme::TEXT_DIM));
        egui::ComboBox::from_id_salt("eq_preset_combo")
            .width(180.0)
            .selected_text("— select —")
            .show_ui(ui, |ui| {
                for entry in PRESETS {
                    if ui.selectable_label(false, entry.name).clicked() {
                        load_preset(&app.params, entry.json);
                    }
                }
            });

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        // Analyzer toggle: three-way segmented control between Off, Pre,
        // and Post. Selecting Pre or Post enables the background spectrum
        // drawn behind the EQ response curve.
        ui.label(egui::RichText::new("Analyzer").color(theme::TEXT_DIM));
        analyzer_segment(ui, &mut app.analyzer_mode, AnalyzerMode::Off, "Off");
        analyzer_segment(ui, &mut app.analyzer_mode, AnalyzerMode::Pre, "Pre");
        analyzer_segment(ui, &mut app.analyzer_mode, AnalyzerMode::Post, "Post");

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        ui.label(egui::RichText::new("Output").color(theme::TEXT_DIM));
        let mut gain = app.params.output_gain.value();
        if ui
            .add(
                egui::Slider::new(&mut gain, -24.0..=24.0)
                    .suffix(" dB")
                    .fixed_decimals(1)
                    .show_value(true),
            )
            .changed()
        {
            app.params.output_gain.set_value(gain);
        }
    });
}

/// One button of the Off/Pre/Post analyzer toggle. Renders as a
/// borderless text button that highlights when selected.
fn analyzer_segment(
    ui: &mut egui::Ui,
    current: &mut AnalyzerMode,
    this: AnalyzerMode,
    label: &str,
) {
    let selected = *current == this;
    let color = if selected {
        theme::ACCENT
    } else {
        theme::TEXT_DIM
    };
    let button = egui::Button::new(egui::RichText::new(label).color(color).strong()).frame(false);
    if ui.add(button).clicked() {
        *current = this;
    }
}

fn load_preset(params: &EqParams, json: &str) {
    resonance_plugin::presets::load(json, crate::params::PARAM_COUNT, |i| params.param_at(i));
}
