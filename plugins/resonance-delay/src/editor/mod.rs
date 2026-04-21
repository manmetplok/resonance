use std::sync::Arc;

use resonance_plugin::gui::{EditorFactory, PluginEditor};
use wayland_plugin_gui::{egui, Editor as RuntimeEditor, EditorApp, EditorOptions};

use crate::params::{DelayParams, PARAM_COUNT};
use crate::presets::PRESETS;
use crate::sync::DIVISION_LABELS;
use crate::viz::DelayViz;

mod controls;
mod echo_view;
mod theme;
mod widgets;

const WINDOW_W: u32 = 1200;
const WINDOW_H: u32 = 600;

pub struct DelayEditorFactory {
    params: Arc<DelayParams>,
    viz: Arc<DelayViz>,
}

impl DelayEditorFactory {
    pub fn new(params: Arc<DelayParams>, viz: Arc<DelayViz>) -> Self {
        Self { params, viz }
    }
}

impl EditorFactory for DelayEditorFactory {
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
        let app = DelayEditorApp::new(self.params.clone(), self.viz.clone());
        let runtime = RuntimeEditor::new(
            app,
            EditorOptions {
                title: "Resonance Delay".to_string(),
                initial_size: (WINDOW_W, WINDOW_H),
                min_size: (900, 480),
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

pub(crate) struct DelayEditorApp {
    pub(crate) params: Arc<DelayParams>,
    pub(crate) viz: Arc<DelayViz>,
}

impl DelayEditorApp {
    pub fn new(params: Arc<DelayParams>, viz: Arc<DelayViz>) -> Self {
        Self { params, viz }
    }
}

impl EditorApp for DelayEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));

        egui::Panel::top("delay_header")
            .exact_size(42.0)
            .show_inside(ui, |ui| draw_header(ui, self));

        egui::Panel::bottom("delay_strip")
            .exact_size(180.0)
            .show_inside(ui, |ui| controls::draw(ui, &self.params));

        egui::CentralPanel::default().show_inside(ui, |ui| draw_center(ui, self));
    }
}

fn draw_header(ui: &mut egui::Ui, app: &mut DelayEditorApp) {
    ui.horizontal_centered(|ui| {
        ui.add_space(12.0);
        ui.label(
            egui::RichText::new("RESONANCE DELAY")
                .strong()
                .color(theme::ACCENT),
        );
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        ui.label(egui::RichText::new("Preset").color(theme::TEXT_DIM));
        egui::ComboBox::from_id_salt("delay_preset_combo")
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

        let bpm = app.viz.read_bpm();
        if bpm > 0.0 {
            ui.label(egui::RichText::new(format!("{bpm:.1} BPM")).color(theme::TEXT));
            ui.add_space(12.0);
        }

        let delay_ms = app.viz.read_delay_time_ms();
        ui.label(egui::RichText::new(format!("{delay_ms:.1} ms")).color(theme::TEXT_DIM));

        if app.params.sync.value() {
            let div = app.params.division.value() as usize;
            if let Some(label) = DIVISION_LABELS.get(div) {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(*label).color(theme::ACCENT));
            }
        }

        // Character + Routing readout.
        ui.add_space(12.0);
        let char_label = if app.params.character.value() == 1 {
            "Analog"
        } else {
            "Digital"
        };
        let route_label = match app.params.routing.value() {
            1 => "Ping-Pong",
            2 => "Dual",
            _ => "Stereo",
        };
        ui.label(
            egui::RichText::new(format!("{char_label} · {route_label}")).color(theme::TEXT_DIM),
        );

        // Freeze indicator.
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
            let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 5.0, dot_color);
        });
    });
}

fn draw_center(ui: &mut egui::Ui, app: &mut DelayEditorApp) {
    let avail = ui.available_rect_before_wrap();
    let gap = 8.0f32;
    let viz_rect = egui::Rect::from_min_max(
        egui::pos2(avail.left() + gap, avail.top() + gap),
        egui::pos2(avail.right() - gap, avail.bottom() - gap),
    );
    let painter = ui.painter_at(avail);
    echo_view::draw(&painter, viz_rect, &app.viz);
}

fn load_preset(params: &DelayParams, json: &str) {
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
