//! Custom EQ editor: a frequency-response curve with draggable band nodes,
//! a per-band control strip underneath, and a factory-preset dropdown in
//! the header. Runs on an egui UI hosted by `wayland-plugin-gui`.

use std::sync::Arc;

use resonance_plugin::gui::{EditorFactory, PluginEditor};
use wayland_plugin_gui::{egui, Editor as RuntimeEditor, EditorApp, EditorOptions};

use crate::analyzer::AnalyzerState;
use crate::band::{BandKind, BandSlope};
use crate::params::{EqParams, NUM_BANDS};
use crate::presets::PRESETS;

mod nodes;
mod response;
mod theme;

/// Which side of the EQ chain to display in the spectrum analyzer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AnalyzerMode {
    Off,
    Pre,
    Post,
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

pub struct EqEditorFactory {
    params: Arc<EqParams>,
    analyzer: Arc<AnalyzerState>,
}

impl EqEditorFactory {
    pub fn new(params: Arc<EqParams>, analyzer: Arc<AnalyzerState>) -> Self {
        Self { params, analyzer }
    }
}

impl EditorFactory for EqEditorFactory {
    fn supports(&self, api_name: &str, is_floating: bool) -> bool {
        is_floating && api_name == "wayland"
    }

    fn preferred(&self) -> Option<(&'static str, bool)> {
        Some(("wayland", true))
    }

    fn preferred_size(&self) -> (u32, u32) {
        (960, 540)
    }

    fn create(&self, api_name: &str, is_floating: bool) -> Option<Box<dyn PluginEditor>> {
        if !self.supports(api_name, is_floating) {
            return None;
        }
        let app = EqEditorApp::new(self.params.clone(), self.analyzer.clone());
        let runtime = RuntimeEditor::new(
            app,
            EditorOptions {
                title: "Resonance EQ".to_string(),
                initial_size: (960, 540),
                min_size: (720, 420),
                resizable: true,
            },
        )
        .ok()?;
        Some(Box::new(RuntimeEditorHandle {
            runtime: Some(runtime),
            size: (960, 540),
        }))
    }
}

struct RuntimeEditorHandle {
    runtime: Option<RuntimeEditor>,
    size: (u32, u32),
}

impl PluginEditor for RuntimeEditorHandle {
    fn show(&mut self) {
        if let Some(r) = &self.runtime {
            r.show();
        }
    }
    fn hide(&mut self) {
        if let Some(r) = &self.runtime {
            r.hide();
        }
    }
    fn size(&self) -> (u32, u32) {
        self.size
    }
    fn set_size(&mut self, width: u32, height: u32) -> bool {
        if let Some(r) = &self.runtime {
            if r.set_size(width, height).is_ok() {
                self.size = (width, height);
                return true;
            }
        }
        false
    }
    fn can_resize(&self) -> bool {
        self.runtime
            .as_ref()
            .map(|r| r.is_resizable())
            .unwrap_or(false)
    }
}

impl Drop for RuntimeEditorHandle {
    fn drop(&mut self) {
        if let Some(r) = self.runtime.take() {
            r.destroy();
        }
    }
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
            .show_inside(ui, |ui| draw_band_strip(ui, self));

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
    let button = egui::Button::new(
        egui::RichText::new(label).color(color).strong(),
    )
    .frame(false);
    if ui.add(button).clicked() {
        *current = this;
    }
}

fn draw_band_strip(ui: &mut egui::Ui, app: &mut EqEditorApp) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        for i in 0..NUM_BANDS {
            draw_band_column(ui, app, i);
            ui.add_space(4.0);
        }
    });
}

fn draw_band_column(ui: &mut egui::Ui, app: &mut EqEditorApp, band_index: usize) {
    let band = &app.params.bands[band_index];
    let selected = app.selected_band == Some(band_index);
    let header_color = if selected {
        theme::ACCENT
    } else {
        theme::TEXT_DIM
    };

    egui::Frame::group(ui.style())
        .fill(if selected {
            theme::PANEL_LIGHT
        } else {
            theme::PANEL
        })
        .stroke(egui::Stroke::new(
            1.0,
            if selected {
                theme::ACCENT
            } else {
                theme::BORDER
            },
        ))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            // The band strip lays out columns horizontally, so `Frame::show`
            // inherits a horizontal parent layout. Everything inside a band
            // cell needs to stack vertically, hence the explicit wrap.
            ui.vertical(|ui| {
                ui.set_min_width(104.0);
                ui.set_max_width(104.0);
                ui.spacing_mut().slider_width = 92.0;

                // Header row — index + enable toggle.
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("B{}", band_index + 1))
                            .strong()
                            .color(header_color),
                    );
                    let mut enabled = band.enabled.value();
                    if ui.checkbox(&mut enabled, "").changed() {
                        band.enabled.set_value(enabled);
                    }
                });

                ui.add_space(2.0);

                // Kind dropdown.
                let mut kind = BandKind::from_index(band.kind.value());
                egui::ComboBox::from_id_salt(("eq_band_kind", band_index))
                    .width(92.0)
                    .selected_text(kind.short_name())
                    .show_ui(ui, |ui| {
                        for opt in [
                            BandKind::Bell,
                            BandKind::LowShelf,
                            BandKind::HighShelf,
                            BandKind::LowCut,
                            BandKind::HighCut,
                        ] {
                            if ui
                                .selectable_label(kind == opt, opt.short_name())
                                .clicked()
                            {
                                kind = opt;
                                band.kind.set_value(kind.to_index());
                            }
                        }
                    });

                // Slope dropdown (only meaningful on cuts).
                if kind.is_cut() {
                    let mut slope = BandSlope::from_index(band.slope.value());
                    egui::ComboBox::from_id_salt(("eq_band_slope", band_index))
                        .width(92.0)
                        .selected_text(slope.label())
                        .show_ui(ui, |ui| {
                            for opt in [BandSlope::Db12, BandSlope::Db24, BandSlope::Db48] {
                                if ui.selectable_label(slope == opt, opt.label()).clicked() {
                                    slope = opt;
                                    band.slope.set_value(slope.to_index());
                                }
                            }
                        });
                } else {
                    // Keep columns the same height whether or not the slope
                    // row is rendered, so all bands line up.
                    ui.add_space(22.0);
                }

                ui.add_space(4.0);

                // Freq.
                let mut freq = band.freq.value();
                if ui
                    .add(
                        egui::Slider::new(&mut freq, 20.0..=20_000.0)
                            .logarithmic(true)
                            .show_value(false),
                    )
                    .changed()
                {
                    band.freq.set_value(freq);
                }
                ui.label(egui::RichText::new(format_hz_short(freq)).color(theme::TEXT_DIM));

                // Gain (only meaningful for bell/shelf).
                if kind.uses_gain() {
                    let mut gain = band.gain.value();
                    if ui
                        .add(
                            egui::Slider::new(&mut gain, -24.0..=24.0)
                                .fixed_decimals(1)
                                .show_value(false),
                        )
                        .changed()
                    {
                        band.gain.set_value(gain);
                    }
                    ui.label(
                        egui::RichText::new(format!("{:+.1} dB", gain)).color(theme::TEXT_DIM),
                    );
                } else {
                    // Keep vertical alignment with bell/shelf bands.
                    ui.add_space(22.0);
                    ui.label(egui::RichText::new(" ").color(theme::TEXT_DIM));
                }

                // Q.
                let mut q = band.q.value();
                if ui
                    .add(
                        egui::Slider::new(&mut q, 0.1..=10.0)
                            .logarithmic(true)
                            .show_value(false),
                    )
                    .changed()
                {
                    band.q.set_value(q);
                }
                ui.label(egui::RichText::new(format!("Q {:.2}", q)).color(theme::TEXT_DIM));
            });
        });
}

fn format_hz_short(freq: f32) -> String {
    if freq >= 1000.0 {
        format!("{:.2} kHz", freq / 1000.0)
    } else {
        format!("{:.0} Hz", freq)
    }
}

fn load_preset(params: &EqParams, json: &str) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return;
    };
    let Some(map) = value.get("params").and_then(|v| v.as_object()) else {
        return;
    };
    // Walk through every param and apply any matching entry.
    for i in 0..crate::params::PARAM_COUNT {
        let p = params.param_at(i);
        if let Some(v) = map.get(p.id()).and_then(|v| v.as_f64()) {
            p.set_plain(v);
        }
    }
}
