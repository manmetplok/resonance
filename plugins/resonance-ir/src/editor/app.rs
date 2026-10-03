//! The actual egui app: state and update/view orchestration for the IR editor.
//!
//! `IrEditorApp` is the `EditorApp` the runtime drives each frame. It paints
//! the chrome panels (header, control strip) and dispatches the centre to the
//! waveform/response/meters views.

use std::sync::atomic::AtomicI32;
use std::sync::Arc;

use parking_lot::Mutex;
use plugin_gui_core::{egui, EditorApp};

use crate::params::IrParams;
use crate::viz::IrViz;

use super::{controls, header, latency, meters, missing_banner, response_view, theme, waveform_view};

pub(crate) struct IrEditorApp {
    pub(crate) params: Arc<IrParams>,
    pub(crate) ir_name: Arc<Mutex<String>>,
    pub(crate) ir_info: Arc<Mutex<String>>,
    pub(crate) load_request: Arc<AtomicI32>,
    pub(crate) viz: Arc<IrViz>,
    /// This plugin ships no factory presets, so the bank is the user's
    /// own directory alone (ba todo #1358).
    pub(crate) bank: resonance_plugin::presets::PresetBank,
    /// Shared with the plugin struct, so what the bar shows is what
    /// `save_state` persists.
    pub(crate) presets: Arc<resonance_plugin::presets::PresetSession>,
    /// Transient bar state (open combo, in-progress rename), editor-only.
    pub(crate) preset_editor: resonance_plugin::presets::PresetEditor,
    /// `Load IR…`'s dialog, polled from `header::draw` (PUX-07) — a
    /// field rather than a one-off local because the dialog can take
    /// many frames to resolve. Linux only; Cocoa calls `rfd` directly
    /// on the guarded AppKit main thread.
    #[cfg(not(target_os = "macos"))]
    pub(crate) ir_picker: Mutex<resonance_plugin::file_picker::FilePicker>,
}

impl EditorApp for IrEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply_once(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));

        egui::Panel::top("ir_header")
            .exact_size(42.0)
            .show_inside(ui, |ui| header::draw(ui, self));

        egui::Panel::bottom("ir_strip")
            .exact_size(120.0)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    controls::draw(ui, &self.params);
                    ui.add_space(16.0);
                    ui.separator();
                    ui.add_space(16.0);
                    latency::draw(ui, self);
                });
            });

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

    // PUX-09: a load failure has nothing real to draw — the banner
    // takes the waveform/response views' place, as the amp's
    // missing-model banner does for its scope/curve.
    let error = missing_banner::load_error(&app.ir_name.lock()).map(str::to_string);
    if let Some(message) = error {
        missing_banner::draw(ui, viz_rect, app, &message);
        let painter = ui.painter_at(avail);
        meters::draw(&painter, meter_rect, &app.viz);
        return;
    }

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
