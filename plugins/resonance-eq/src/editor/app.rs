//! Main EQ editor app: state struct, `EditorApp` impl, header drawing,
//! and preset loading.

use std::sync::Arc;

use plugin_gui_core::widgets::{slider as slider_widget, HSlider, SliderStyle};
use plugin_gui_core::{egui, EditorApp};

use resonance_plugin::preset_ui::preset_bar;
use resonance_plugin::presets::{PresetBank, PresetEditor, PresetSession};
use resonance_plugin::Param;

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
    /// Factory bank + this plugin's user preset directory.
    pub(crate) bank: PresetBank,
    /// Shared with the plugin struct, so what the bar shows is what
    /// `save_state` persists.
    pub(crate) presets: Arc<PresetSession>,
    /// Transient bar state (open combo, in-progress rename), editor-only.
    pub(crate) preset_editor: PresetEditor,
    pub(crate) analyzer_mode: AnalyzerMode,
    /// Which band is highlighted in the curve/strip. None = no selection.
    pub(crate) selected_band: Option<usize>,
    /// Currently-dragged band, if any.
    pub(crate) drag_state: Option<nodes::DragState>,
}

impl EqEditorApp {
    pub fn new(
        params: Arc<EqParams>,
        analyzer: Arc<AnalyzerState>,
        presets: Arc<PresetSession>,
    ) -> Self {
        Self {
            params,
            analyzer,
            bank: PresetBank::new(<crate::ResonanceEq as resonance_plugin::ResonancePlugin>::CLAP_ID, PRESETS),
            presets,
            preset_editor: PresetEditor::default(),
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
            .exact_size(STRIP_H)
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
        let params: Vec<&dyn Param> = (0..crate::params::PARAM_COUNT)
            .map(|i| app.params.param_at(i))
            .collect();
        preset_bar(
            ui,
            "eq_preset",
            &mut app.preset_editor,
            &app.bank,
            &app.presets,
            &params,
            "— select —",
        );

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

        // Output trim: the shared kit's slider, like the band strip
        // below it (ba todo #1335). Bipolar, so the fill runs out from
        // 0 dB. The raw `egui::Slider` this replaces carried an inline
        // value box you could click and type into; the shared slider has
        // no such box in any editor, so the number moves to a readout
        // beside it and exact entry is gone — arrow keys with the slider
        // focused are the fine adjustment now.
        ui.label(egui::RichText::new("Output").color(theme::TEXT_DIM));
        let mut gain = app.params.output_gain.value();
        let slider = HSlider::new(OUTPUT_SLIDER_W, gain / OUTPUT_GAIN_DB * 0.5 + 0.5)
            .bipolar(true)
            .style(SliderStyle::LAVENDER);
        if let Some(travel) = slider_widget(ui, &slider) {
            gain = (travel * 2.0 - 1.0) * OUTPUT_GAIN_DB;
            app.params.output_gain.set_value(gain);
        }
        ui.label(egui::RichText::new(format!("{:+.1} dB", gain)).color(theme::TEXT_DIM));

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        // Auto-gain: the output trimmed by the curve's static loudness
        // estimate, so a move can be judged at matched level. The trim is
        // a function of the bands alone, so the editor computes the same
        // number the DSP applies.
        let mut auto = app.params.auto_gain.value();
        if ui.checkbox(&mut auto, "Auto gain").changed() {
            app.params.auto_gain.set_value(auto);
        }
        if auto {
            let snaps: [crate::params::BandSnapshot; crate::params::NUM_BANDS] =
                std::array::from_fn(|i| app.params.bands[i].snapshot());
            let trim = crate::dsp::auto_gain_trim_db(&snaps, VIS_SR);
            ui.label(egui::RichText::new(format!("{trim:+.1} dB")).color(theme::TEXT_DIM));
        }
    });
}

/// Height of the band strip, px: header, kind, slope, M/S, and the
/// Freq / Gain / Q sliders with their readouts.
pub(crate) const STRIP_H: f32 = 190.0;
/// Sample rate the header's auto-gain readout is estimated at. The trim
/// is nearly independent of the rate below the top octave.
const VIS_SR: f32 = 48_000.0;

/// Width of the header's Output slider, px — `egui::Slider`'s own
/// default `slider_width`, which is what it was laid out at.
const OUTPUT_SLIDER_W: f32 = 100.0;
/// Half-range of the Output trim, dB (it runs `-24..=24`).
const OUTPUT_GAIN_DB: f32 = 24.0;

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
