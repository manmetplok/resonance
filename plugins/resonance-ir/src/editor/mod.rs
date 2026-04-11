//! IR plugin editor: an egui UI hosted in `wayland-plugin-gui`.
//!
//! Mirrors the amp / reverb editors: a Factory that produces a
//! `RuntimeEditorHandle`, which wraps the `wayland_plugin_gui::Editor`
//! and drives an `EditorApp` implementation on the editor thread.
//!
//! Layout (top → bottom):
//!
//! - Top strip: header with title, "Load IR…" button, Prev/Next and
//!   the current filename + position counter.
//! - Centre: waveform view (left) + frequency-response view (right)
//!   drawn from the `IrSnapshot` published by the loader thread, plus
//!   a stereo IN/OUT meter strip along the bottom.
//! - Bottom: the dry/wet and output-gain control strip.

use std::sync::atomic::AtomicI32;
use std::sync::Arc;

use parking_lot::Mutex;
use resonance_plugin::gui::{EditorFactory, PluginEditor};
use wayland_plugin_gui::{egui, Editor as RuntimeEditor, EditorApp, EditorOptions};

use crate::params::IrParams;
use crate::viz::IrViz;

mod controls;
mod header;
mod meters;
mod response_view;
mod theme;
mod waveform_view;

const INITIAL_SIZE: (u32, u32) = (880, 540);
const MIN_SIZE: (u32, u32) = (680, 440);

// ---------------------------------------------------------------------------
// Factory — produced by ResonanceIr::editor_factory().
// ---------------------------------------------------------------------------

pub struct IrEditorFactory {
    params: Arc<IrParams>,
    ir_name: Arc<Mutex<String>>,
    ir_info: Arc<Mutex<String>>,
    load_request: Arc<AtomicI32>,
    viz: Arc<IrViz>,
}

impl IrEditorFactory {
    pub(crate) fn new(
        params: Arc<IrParams>,
        ir_name: Arc<Mutex<String>>,
        ir_info: Arc<Mutex<String>>,
        load_request: Arc<AtomicI32>,
        viz: Arc<IrViz>,
    ) -> Self {
        Self {
            params,
            ir_name,
            ir_info,
            load_request,
            viz,
        }
    }
}

impl EditorFactory for IrEditorFactory {
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
        let app = IrEditorApp {
            params: self.params.clone(),
            ir_name: self.ir_name.clone(),
            ir_info: self.ir_info.clone(),
            load_request: self.load_request.clone(),
            viz: self.viz.clone(),
        };
        let runtime = RuntimeEditor::new(
            app,
            EditorOptions {
                title: "Resonance IR".to_string(),
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

pub(crate) struct IrEditorApp {
    pub(crate) params: Arc<IrParams>,
    pub(crate) ir_name: Arc<Mutex<String>>,
    pub(crate) ir_info: Arc<Mutex<String>>,
    pub(crate) load_request: Arc<AtomicI32>,
    pub(crate) viz: Arc<IrViz>,
}

impl EditorApp for IrEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));

        egui::Panel::top("ir_header")
            .exact_size(42.0)
            .show_inside(ui, |ui| header::draw(ui, self));

        egui::Panel::bottom("ir_strip")
            .exact_size(120.0)
            .show_inside(ui, |ui| controls::draw(ui, &self.params));

        egui::CentralPanel::default().show_inside(ui, |ui| draw_center(ui, self));
    }
}

fn draw_center(ui: &mut egui::Ui, app: &mut IrEditorApp) {
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

    // Split viz: waveform (left ~55%), response (right ~45%).
    let resp_w = (viz_rect.width() * 0.45).clamp(240.0, 520.0);
    let wave_rect = egui::Rect::from_min_max(
        viz_rect.min,
        egui::pos2(viz_rect.right() - resp_w - gap, viz_rect.bottom()),
    );
    let resp_rect = egui::Rect::from_min_max(
        egui::pos2(wave_rect.right() + gap, viz_rect.top()),
        viz_rect.max,
    );

    let painter = ui.painter_at(avail);
    waveform_view::draw(&painter, wave_rect, &app.viz, &app.ir_name.lock());
    response_view::draw(&painter, resp_rect, &app.viz);
    meters::draw(&painter, meter_rect, &app.viz);
}
