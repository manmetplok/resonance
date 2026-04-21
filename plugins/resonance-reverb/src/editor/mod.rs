//! Reverb editor — egui UI hosted by `wayland-plugin-gui`.
//!
//! Layout (top-down):
//! - Header: plugin name, preset dropdown, live readouts, freeze indicator.
//! - Central: impulse tail hero visualisation on the left, FDN tank view
//!   on the right, stereo peak meters along the bottom of the central area.
//! - Bottom: control strip with the 12 parameters.

use std::sync::Arc;

use resonance_plugin::gui::{EditorFactory, PluginEditor};
use wayland_plugin_gui::{egui, Editor as RuntimeEditor, EditorApp, EditorOptions};

use crate::params::{ReverbParams, PARAM_COUNT};
use crate::presets::PRESETS;
use crate::viz::ReverbViz;

mod controls;
mod impulse_view;
mod meters;
mod tank_view;
mod theme;

const WINDOW_W: u32 = 1320;
const WINDOW_H: u32 = 660;

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

pub struct ReverbEditorFactory {
    params: Arc<ReverbParams>,
    viz: Arc<ReverbViz>,
}

impl ReverbEditorFactory {
    pub fn new(params: Arc<ReverbParams>, viz: Arc<ReverbViz>) -> Self {
        Self { params, viz }
    }
}

impl EditorFactory for ReverbEditorFactory {
    fn supports(&self, api_name: &str, is_floating: bool) -> bool {
        is_floating && api_name == "wayland"
    }
    fn preferred(&self) -> Option<(&'static str, bool)> {
        Some(("wayland", true))
    }
    fn preferred_size(&self) -> (u32, u32) {
        (WINDOW_W, WINDOW_H)
    }
    fn create(&self, api_name: &str, is_floating: bool) -> Option<Box<dyn PluginEditor>> {
        if !self.supports(api_name, is_floating) {
            return None;
        }
        let app = ReverbEditorApp::new(self.params.clone(), self.viz.clone());
        let runtime = RuntimeEditor::new(
            app,
            EditorOptions {
                title: "Resonance Reverb".to_string(),
                initial_size: (WINDOW_W, WINDOW_H),
                min_size: (720, 560),
                resizable: true,
            },
        )
        .ok()?;
        Some(Box::new(RuntimeEditorHandle {
            runtime: Some(runtime),
            size: (WINDOW_W, WINDOW_H),
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

pub(crate) struct ReverbEditorApp {
    pub(crate) params: Arc<ReverbParams>,
    pub(crate) viz: Arc<ReverbViz>,
}

impl ReverbEditorApp {
    pub fn new(params: Arc<ReverbParams>, viz: Arc<ReverbViz>) -> Self {
        Self { params, viz }
    }
}

impl EditorApp for ReverbEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));

        egui::Panel::top("reverb_header")
            .exact_size(42.0)
            .show_inside(ui, |ui| draw_header(ui, self));

        egui::Panel::bottom("reverb_strip")
            .exact_size(200.0)
            .show_inside(ui, |ui| controls::draw(ui, &self.params));

        egui::CentralPanel::default().show_inside(ui, |ui| draw_center(ui, self));
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

        ui.label(egui::RichText::new("Preset").color(theme::TEXT_DIM));
        egui::ComboBox::from_id_salt("reverb_preset_combo")
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

        let decay = app.params.decay.value();
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

fn draw_center(ui: &mut egui::Ui, app: &mut ReverbEditorApp) {
    let avail = ui.available_rect_before_wrap();

    // Reserve a thin strip along the bottom for the stereo peak meters.
    let meter_h = 28.0f32;
    let gap = 8.0f32;
    let viz_rect = egui::Rect::from_min_max(
        egui::pos2(avail.left() + gap, avail.top() + gap),
        egui::pos2(avail.right() - gap, avail.bottom() - meter_h - gap),
    );
    let meter_rect = egui::Rect::from_min_max(
        egui::pos2(avail.left() + gap, avail.bottom() - meter_h),
        egui::pos2(avail.right() - gap, avail.bottom() - 2.0),
    );

    // Split the viz area: impulse hero (left ~68%) + FDN tank (right ~32%).
    let tank_w = 300.0f32.min(viz_rect.width() * 0.35);
    let impulse_rect = egui::Rect::from_min_max(
        viz_rect.min,
        egui::pos2(viz_rect.right() - tank_w - gap, viz_rect.bottom()),
    );
    let tank_rect = egui::Rect::from_min_max(
        egui::pos2(impulse_rect.right() + gap, viz_rect.top()),
        viz_rect.max,
    );

    let painter = ui.painter_at(avail);
    impulse_view::draw(&painter, impulse_rect, app);
    tank_view::draw(&painter, tank_rect, app);
    meters::draw(&painter, meter_rect, &app.viz);
}

/// Apply a factory preset: walk every param and call `set_plain` for any
/// id that matches a key in the preset's `params` object. Missing keys
/// are ignored so older presets still load after a param is added.
fn load_preset(params: &ReverbParams, json: &str) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return;
    };
    let Some(map) = value.get("params").and_then(|v| v.as_object()) else {
        return;
    };
    for i in 0..PARAM_COUNT {
        let p = params.param_at(i);
        if let Some(v) = map.get(p.id()).and_then(|v| v.as_f64()) {
            p.set_plain(v);
        }
    }
}
