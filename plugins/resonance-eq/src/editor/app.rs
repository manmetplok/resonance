//! Main EQ editor app: state struct, `EditorApp` impl, header drawing,
//! and preset loading.

use std::sync::Arc;

use plugin_gui_core::{egui, EditorApp};
use resonance_plugin::editor_widgets::{self, ParamSlider};

use resonance_plugin::preset_ui::preset_bar;
use resonance_plugin::presets::{PresetBank, PresetEditor, PresetSession};
use resonance_plugin::Param;

use crate::analyzer::AnalyzerState;
use crate::params::EqParams;

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
    /// The band whose Q a scroll over its node is adjusting, while the
    /// scroll runs: one announced edit per scroll, not per frame.
    pub(crate) q_scroll: Option<usize>,
    /// The drawn band curve and the band state it was computed from, so
    /// the response is re-evaluated only when a band moved, not on every
    /// 16 ms repaint (ui-work.md §11).
    pub(crate) curve_cache: Option<(
        [crate::params::BandSnapshot; crate::params::NUM_BANDS],
        crate::response_curve::ResponseCurves,
    )>,
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
            bank: PresetBank::for_plugin::<crate::ResonanceEq>(),
            presets,
            preset_editor: PresetEditor::default(),
            analyzer_mode: AnalyzerMode::Post,
            selected_band: None,
            drag_state: None,
            q_scroll: None,
            curve_cache: None,
        }
    }
}

impl EditorApp for EqEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply_once(ui.ctx());
        // Continuous repaint so response curve follows slider movement.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));

        // The header sizes to its content: one row when the window is
        // wide enough, its groups wrapped onto a second row when it is
        // not (BUG-41).
        let header_frame =
            egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(8, 8));
        egui::Panel::top("eq_header")
            .frame(header_frame)
            .min_size(HEADER_MIN_H)
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
    // The header's controls come in groups (title, preset bar, analyzer,
    // output trim, auto-gain) that wrap as units: a group that no longer
    // fits on the row starts the next one, so a narrow window grows the
    // header a row instead of clipping the Output readout and the
    // auto-gain switch off the right edge (BUG-41). The title goes first:
    // it is drawn only while everything fits on one row, since the
    // window's own title names the plugin anyway.
    let widths: Option<Vec<f32>> = HEADER_GROUPS.iter().map(|g| group_width(ui, g)).collect();
    let separators = (HEADER_GROUPS.len() - 1) as f32 * separator_width(ui);
    let row_w = ui.max_rect().width();
    let show_title = widths.is_none_or(|w| w.iter().sum::<f32>() + separators <= row_w);
    ui.horizontal_wrapped(|ui| {
        if show_title {
            header_group(ui, "title", |ui| {
                ui.label(
                    egui::RichText::new("RESONANCE EQ")
                        .strong()
                        .color(theme::ACCENT),
                );
            });
        }

        header_group(ui, "preset", |ui| {
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
        });

        // Analyzer toggle: three-way segmented control between Off, Pre,
        // and Post. Selecting Pre or Post enables the background spectrum
        // drawn behind the EQ response curve.
        header_group(ui, "analyzer", |ui| {
            ui.label(egui::RichText::new("Analyzer").color(theme::TEXT_DIM));
            analyzer_segment(ui, &mut app.analyzer_mode, AnalyzerMode::Off, "Off");
            analyzer_segment(ui, &mut app.analyzer_mode, AnalyzerMode::Pre, "Pre");
            analyzer_segment(ui, &mut app.analyzer_mode, AnalyzerMode::Post, "Post");
        });

        // Output trim: the shared kit's slider bound to the param, like
        // the band strip below it — bipolar from the range, reset to the
        // declared default on a double-click, the readout beside it
        // takes a typed value, and the gesture is announced (PUX-01/-06).
        header_group(ui, "output", |ui| {
            ui.label(egui::RichText::new("Output").color(theme::TEXT_DIM));
            let gain = &app.params.output_gain;
            editor_widgets::param_slider(ui, ParamSlider::new(gain, OUTPUT_SLIDER_W));
            let text = gain.display(gain.get_plain());
            let font = egui::TextStyle::Body.resolve(ui.style());
            editor_widgets::param_readout(ui, gain, "", &text, 52.0, font, theme::TEXT_DIM);
        });

        // Auto-gain: the output trimmed by the curve's static loudness
        // estimate, so a move can be judged at matched level. The trim is
        // a function of the bands alone, so the editor computes the same
        // number the DSP applies.
        header_group(ui, "auto_gain", |ui| {
            editor_widgets::bool_checkbox(ui, &app.params.auto_gain, "Auto gain");
            if app.params.auto_gain.value() {
                let snaps: [crate::params::BandSnapshot; crate::params::NUM_BANDS] =
                    std::array::from_fn(|i| app.params.bands[i].snapshot());
                let trim = crate::dsp::auto_gain_trim_db(&snaps, VIS_SR);
                let readout = ui.label(
                    egui::RichText::new(format!("{trim:+.1} dB")).color(theme::TEXT_DIM),
                );
                editor_widgets::probe(ui, "eq_auto_gain_readout", readout.rect);
            }
        });
    });
}

/// The header's wrapping groups, left to right.
const HEADER_GROUPS: [&str; 5] = ["title", "preset", "analyzer", "output", "auto_gain"];

/// Width a header separator allocates, px — its line sits in the middle.
const HEADER_SEPARATOR_SPACING: f32 = 12.0;

/// What a separator between two groups adds to the row, px: its own
/// width and the item spacing either side.
fn separator_width(ui: &egui::Ui) -> f32 {
    HEADER_SEPARATOR_SPACING + 2.0 * ui.spacing().item_spacing.x
}

fn group_id(salt: &str) -> egui::Id {
    egui::Id::new(("eq_header_group", salt))
}

/// The width `salt`'s group took when last drawn, px.
fn group_width(ui: &egui::Ui, salt: &str) -> Option<f32> {
    ui.ctx().data(|d| d.get_temp(group_id(salt)))
}

/// One wrapping unit of the header: `add` laid out left to right,
/// after a separator unless it starts a row.
///
/// A nested `horizontal` does not wrap by itself — egui only knows its
/// width after drawing it — so the width each group took last frame is
/// remembered and checked against what is left of the row. The first
/// frame lays out unwrapped, the next wraps, and the one after has the
/// panel at its new height; the editor repaints every 16 ms anyway.
fn header_group(ui: &mut egui::Ui, salt: &str, add: impl FnOnce(&mut egui::Ui)) {
    let row_started = ui.cursor().min.x > ui.max_rect().min.x + 1.0;
    // What is left of this row. Not `available_width()`: in a wrapping
    // layout that is the whole row, since egui would wrap a plain widget.
    let left = ui.max_rect().max.x - ui.cursor().min.x;
    let fits = group_width(ui, salt).is_none_or(|w| w + separator_width(ui) <= left);
    if row_started && !fits {
        ui.end_row();
    } else if row_started {
        ui.add(egui::Separator::default().spacing(HEADER_SEPARATOR_SPACING));
    }
    let rect = ui.horizontal(add).response.rect;
    editor_widgets::probe(ui, &format!("eq_header_{salt}"), rect);
    ui.ctx().data_mut(|d| d.insert_temp(group_id(salt), rect.width()));
}

/// The header's height with one row of controls, px — what the fixed
/// header was before it could wrap.
const HEADER_MIN_H: f32 = 40.0;

/// Height of the band strip, px: header, kind, slope, M/S, the
/// Freq / Gain / Q sliders with their readouts, and the dynamics switch
/// with its four sliders — measured, not estimated: a column lays out
/// 400 px tall, under a 6 px top pad, over the 10 px the row's scroll
/// bar takes in a narrow window, inside the panel's 2 px margins. At
/// the old 350 the Att and Rel rows were cut off (BUG-41);
/// `tests/editor_controls/layout.rs` measures it at the window sizes the
/// factory declares, so a control added to the column fails there first.
pub(crate) const STRIP_H: f32 = 424.0;
/// Sample rate the header's auto-gain readout is estimated at. The trim
/// is nearly independent of the rate below the top octave.
const VIS_SR: f32 = 48_000.0;

/// Width of the header's Output slider, px — `egui::Slider`'s own
/// default `slider_width`, which is what it was laid out at.
const OUTPUT_SLIDER_W: f32 = 100.0;

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
