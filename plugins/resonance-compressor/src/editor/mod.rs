//! Compressor editor — egui UI hosted by wayland-plugin-gui.
//!
//! Layout (top-down):
//! - Header: plugin name, preset dropdown.
//! - Middle: transfer curve + GR history + 3 meters (In / GR / Out) in a row.
//! - Bottom: control strip with the 11 parameters.

use std::sync::Arc;

use resonance_plugin::gui::{EditorFactory, PluginEditor};
use wayland_plugin_gui::{egui, Editor as RuntimeEditor, EditorApp, EditorOptions};

use crate::params::CompressorParams;
use crate::presets::PRESETS;
use crate::viz::CompressorViz;

mod curve;
mod history;
mod meters;
mod theme;

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

pub struct CompressorEditorFactory {
    params: Arc<CompressorParams>,
    viz: Arc<CompressorViz>,
}

impl CompressorEditorFactory {
    pub fn new(params: Arc<CompressorParams>, viz: Arc<CompressorViz>) -> Self {
        Self { params, viz }
    }
}

impl EditorFactory for CompressorEditorFactory {
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
        let app = CompressorEditorApp::new(self.params.clone(), self.viz.clone());
        let runtime = RuntimeEditor::new(
            app,
            EditorOptions {
                title: "Resonance Compressor".to_string(),
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
// App
// ---------------------------------------------------------------------------

pub(crate) struct CompressorEditorApp {
    pub(crate) params: Arc<CompressorParams>,
    pub(crate) viz: Arc<CompressorViz>,
}

impl CompressorEditorApp {
    pub fn new(params: Arc<CompressorParams>, viz: Arc<CompressorViz>) -> Self {
        Self { params, viz }
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
            .exact_size(160.0)
            .show_inside(ui, |ui| draw_control_strip(ui, self));

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
        egui::ComboBox::from_id_salt("comp_preset_combo")
            .width(200.0)
            .selected_text("— select —")
            .show_ui(ui, |ui| {
                for entry in PRESETS {
                    if ui.selectable_label(false, entry.name).clicked() {
                        load_preset(&app.params, entry.json);
                    }
                }
            });
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
    meters::draw_input_meter(&painter, in_rect, app.viz.read_input_db());
    meters::draw_gr_meter(&painter, gr_rect, app.viz.read_gr_db());
    meters::draw_output_meter(&painter, out_rect, app.viz.read_output_db());
}

fn draw_control_strip(ui: &mut egui::Ui, app: &mut CompressorEditorApp) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        control_column(ui, "Threshold", "", |ui| {
            let mut v = app.params.threshold.value();
            if ui
                .add(
                    egui::Slider::new(&mut v, -60.0..=0.0)
                        .suffix(" dB")
                        .fixed_decimals(1)
                        .show_value(true),
                )
                .changed()
            {
                app.params.threshold.set_value(v);
            }
        });
        control_column(ui, "Ratio", "", |ui| {
            let mut v = app.params.ratio.value();
            if ui
                .add(
                    egui::Slider::new(&mut v, 1.0..=20.0)
                        .logarithmic(true)
                        .fixed_decimals(1)
                        .show_value(true),
                )
                .changed()
            {
                app.params.ratio.set_value(v);
            }
        });
        control_column(ui, "Attack", "", |ui| {
            let mut v = app.params.attack.value();
            if ui
                .add(
                    egui::Slider::new(&mut v, 0.1..=200.0)
                        .logarithmic(true)
                        .suffix(" ms")
                        .fixed_decimals(2)
                        .show_value(true),
                )
                .changed()
            {
                app.params.attack.set_value(v);
            }
        });
        control_column(ui, "Release", "", |ui| {
            let mut v = app.params.release.value();
            if ui
                .add(
                    egui::Slider::new(&mut v, 5.0..=2000.0)
                        .logarithmic(true)
                        .suffix(" ms")
                        .fixed_decimals(1)
                        .show_value(true),
                )
                .changed()
            {
                app.params.release.set_value(v);
            }
        });
        control_column(ui, "Knee", "", |ui| {
            let mut v = app.params.knee.value();
            if ui
                .add(
                    egui::Slider::new(&mut v, 0.0..=12.0)
                        .suffix(" dB")
                        .fixed_decimals(1)
                        .show_value(true),
                )
                .changed()
            {
                app.params.knee.set_value(v);
            }
        });
        control_column(ui, "Makeup", "", |ui| {
            let mut v = app.params.makeup.value();
            if ui
                .add(
                    egui::Slider::new(&mut v, -12.0..=24.0)
                        .suffix(" dB")
                        .fixed_decimals(1)
                        .show_value(true),
                )
                .changed()
            {
                app.params.makeup.set_value(v);
            }
            let mut auto = app.params.auto_makeup.value();
            if ui.checkbox(&mut auto, "Auto").changed() {
                app.params.auto_makeup.set_value(auto);
            }
        });
        control_column(ui, "Mix", "", |ui| {
            let mut v = app.params.mix.value();
            if ui
                .add(
                    egui::Slider::new(&mut v, 0.0..=1.0)
                        .suffix("")
                        .fixed_decimals(2)
                        .show_value(true),
                )
                .changed()
            {
                app.params.mix.set_value(v);
            }
        });
        control_column(ui, "Detector", "Peak ↔ RMS", |ui| {
            let mut v = app.params.detector_mix.value();
            if ui
                .add(
                    egui::Slider::new(&mut v, 0.0..=1.0)
                        .fixed_decimals(2)
                        .show_value(true),
                )
                .changed()
            {
                app.params.detector_mix.set_value(v);
            }
        });
        control_column(ui, "SC HPF", "", |ui| {
            let mut on = app.params.sc_hpf_on.value();
            if ui.checkbox(&mut on, "On").changed() {
                app.params.sc_hpf_on.set_value(on);
            }
            let mut v = app.params.sc_hpf_freq.value();
            if ui
                .add(
                    egui::Slider::new(&mut v, 20.0..=500.0)
                        .logarithmic(true)
                        .suffix(" Hz")
                        .fixed_decimals(0)
                        .show_value(true),
                )
                .changed()
            {
                app.params.sc_hpf_freq.set_value(v);
            }
        });
    });
}

fn control_column(
    ui: &mut egui::Ui,
    label: &str,
    sub: &str,
    mut contents: impl FnMut(&mut egui::Ui),
) {
    egui::Frame::group(ui.style())
        .fill(theme::PANEL)
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_min_width(110.0);
                ui.set_max_width(110.0);
                ui.spacing_mut().slider_width = 98.0;
                ui.label(
                    egui::RichText::new(label)
                        .strong()
                        .color(theme::TEXT),
                );
                if !sub.is_empty() {
                    ui.label(
                        egui::RichText::new(sub)
                            .size(9.0)
                            .color(theme::TEXT_DIM),
                    );
                }
                ui.add_space(4.0);
                contents(ui);
            });
        });
    ui.add_space(4.0);
}

fn load_preset(params: &CompressorParams, json: &str) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return;
    };
    let Some(map) = value.get("params").and_then(|v| v.as_object()) else {
        return;
    };
    for i in 0..crate::params::PARAM_COUNT {
        let p = params.param_at(i);
        if let Some(v) = map.get(p.id()).and_then(|v| v.as_f64()) {
            p.set_plain(v);
        }
    }
}
