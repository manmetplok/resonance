//! Amp plugin editor: an egui UI hosted in `wayland-plugin-gui`.
//!
//! Layout mirrors the reverb editor's three-zone structure:
//!
//! - Top strip: header with title, Load Model button, file browser,
//!   and current model name.
//! - Below the header: a dedicated tuner strip.
//! - Centre: the main visualisation area — live oscilloscope on the
//!   left, static transfer-curve plot on the right, with stereo peak
//!   meters along the bottom.
//! - Bottom: the gain control strip.

use std::sync::atomic::AtomicI32;
use std::sync::Arc;

use parking_lot::Mutex;
use resonance_plugin::gui::{EditorFactory, PluginEditor};
use wayland_plugin_gui::{egui, Editor as RuntimeEditor, EditorApp, EditorOptions};

use crate::params::AmpParams;
use crate::viz::AmpViz;

mod controls;
mod curve_view;
mod header;
mod meters;
mod scope_view;
mod theme;
mod tuner_view;

const INITIAL_SIZE: (u32, u32) = (960, 620);
const MIN_SIZE: (u32, u32) = (760, 520);

// ---------------------------------------------------------------------------
// Factory — produced by ResonanceAmp::editor_factory().
// ---------------------------------------------------------------------------

pub struct AmpEditorFactory {
    params: Arc<AmpParams>,
    model_name: Arc<Mutex<String>>,
    load_request: Arc<AtomicI32>,
    viz: Arc<AmpViz>,
}

impl AmpEditorFactory {
    pub(crate) fn new(
        params: Arc<AmpParams>,
        model_name: Arc<Mutex<String>>,
        load_request: Arc<AtomicI32>,
        viz: Arc<AmpViz>,
    ) -> Self {
        Self {
            params,
            model_name,
            load_request,
            viz,
        }
    }
}

impl EditorFactory for AmpEditorFactory {
    fn supports(&self, api_name: &str, is_floating: bool) -> bool {
        is_floating && api_name == "wayland"
    }

    fn preferred(&self) -> Option<(&'static str, bool)> {
        Some(("wayland", true))
    }

    fn preferred_size(&self) -> (u32, u32) {
        INITIAL_SIZE
    }

    fn create(&self, api_name: &str, is_floating: bool) -> Option<Box<dyn PluginEditor>> {
        if !self.supports(api_name, is_floating) {
            return None;
        }
        let app = AmpEditorApp {
            params: self.params.clone(),
            model_name: self.model_name.clone(),
            load_request: self.load_request.clone(),
            viz: self.viz.clone(),
        };
        let runtime = RuntimeEditor::new(
            app,
            EditorOptions {
                title: "Resonance Amp".to_string(),
                initial_size: INITIAL_SIZE,
                min_size: MIN_SIZE,
                resizable: true,
            },
        )
        .ok()?;
        Some(Box::new(RuntimeEditorHandle {
            runtime: Some(runtime),
            size: INITIAL_SIZE,
        }))
    }
}

// ---------------------------------------------------------------------------
// RuntimeEditorHandle — bridges `PluginEditor` to `wayland_plugin_gui::Editor`.
// ---------------------------------------------------------------------------

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

    fn set_title(&mut self, _title: &str) {}
}

impl Drop for RuntimeEditorHandle {
    fn drop(&mut self) {
        if let Some(r) = self.runtime.take() {
            r.destroy();
        }
    }
}

// ---------------------------------------------------------------------------
// EditorApp — the egui UI driven on the editor thread.
// ---------------------------------------------------------------------------

pub(crate) struct AmpEditorApp {
    pub(crate) params: Arc<AmpParams>,
    pub(crate) model_name: Arc<Mutex<String>>,
    pub(crate) load_request: Arc<AtomicI32>,
    pub(crate) viz: Arc<AmpViz>,
}

impl EditorApp for AmpEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));

        egui::Panel::top("amp_header")
            .exact_size(38.0)
            .show_inside(ui, |ui| header::draw(ui, self));

        egui::Panel::top("amp_tuner")
            .exact_size(72.0)
            .show_inside(ui, |ui| {
                let rect = ui.available_rect_before_wrap();
                let painter = ui.painter_at(rect);
                tuner_view::draw(&painter, rect, &self.viz);
            });

        egui::Panel::bottom("amp_strip")
            .exact_size(140.0)
            .show_inside(ui, |ui| controls::draw(ui, &self.params));

        egui::CentralPanel::default().show_inside(ui, |ui| draw_center(ui, self));
    }
}

fn draw_center(ui: &mut egui::Ui, app: &mut AmpEditorApp) {
    let avail = ui.available_rect_before_wrap();
    let gap = 8.0f32;
    let meter_h = 28.0f32;

    let viz_rect = egui::Rect::from_min_max(
        egui::pos2(avail.left() + gap, avail.top() + gap),
        egui::pos2(avail.right() - gap, avail.bottom() - meter_h - gap),
    );
    let meter_rect = egui::Rect::from_min_max(
        egui::pos2(avail.left() + gap, avail.bottom() - meter_h),
        egui::pos2(avail.right() - gap, avail.bottom() - 2.0),
    );

    // Split the viz area: scope (left ~65%) + transfer curve (right ~35%).
    let curve_w = 280.0f32.min(viz_rect.width() * 0.4);
    let scope_rect = egui::Rect::from_min_max(
        viz_rect.min,
        egui::pos2(viz_rect.right() - curve_w - gap, viz_rect.bottom()),
    );
    let curve_rect = egui::Rect::from_min_max(
        egui::pos2(scope_rect.right() + gap, viz_rect.top()),
        viz_rect.max,
    );

    let painter = ui.painter_at(avail);
    scope_view::draw(&painter, scope_rect, &app.viz);
    curve_view::draw(&painter, curve_rect, &app.viz);
    meters::draw(&painter, meter_rect, &app.viz);
}
