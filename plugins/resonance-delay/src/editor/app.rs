//! The actual egui app: state and update/view orchestration for the delay editor.
//!
//! `DelayEditorApp` is the `EditorApp` the runtime drives each frame. It paints
//! the header (title + preset picker + readouts + freeze indicator), the bottom
//! control strip, and dispatches the centre to the echo view.

use std::sync::Arc;

use resonance_plugin::preset_ui::preset_bar;
use resonance_plugin::presets::{PresetBank, PresetEditor, PresetSession};
use resonance_plugin::Param;
use plugin_gui_core::{egui, EditorApp};

use crate::params::DelayParams;
use crate::sync::division_label;
use crate::viz::DelayViz;

use super::{controls, echo_view, theme};

pub(crate) struct DelayEditorApp {
    pub(crate) params: Arc<DelayParams>,
    pub(crate) viz: Arc<DelayViz>,
    /// Factory bank + this plugin's user preset directory.
    pub(crate) bank: PresetBank,
    /// Shared with the plugin struct, so what the bar shows is what
    /// `save_state` persists.
    pub(crate) presets: Arc<PresetSession>,
    /// Transient bar state (open combo, in-progress rename), editor-only.
    pub(crate) preset_editor: PresetEditor,
}

impl DelayEditorApp {
    pub fn new(
        params: Arc<DelayParams>,
        viz: Arc<DelayViz>,
        presets: Arc<PresetSession>,
    ) -> Self {
        Self {
            params,
            viz,
            bank: PresetBank::new(
                <crate::ResonanceDelay as resonance_plugin::ResonancePlugin>::CLAP_ID,
                <crate::ResonanceDelay as resonance_plugin::ResonancePlugin>::FACTORY_PRESETS,
            ),
            presets,
            preset_editor: PresetEditor::default(),
        }
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
        let params: Vec<&dyn Param> = (0..crate::params::PARAM_COUNT)
            .map(|i| app.params.param_at(i))
            .collect();
        preset_bar(
            ui,
            "delay_preset",
            &mut app.preset_editor,
            &app.bank,
            &app.presets,
            &params,
            "— select —",
        );

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
            ui.add_space(8.0);
            let label = division_label(app.params.division.value() as usize);
            ui.label(egui::RichText::new(label).color(theme::ACCENT));
        }

        // Gate rate reads as a division too, so "1/4 delay, 1/16 gate"
        // is legible without opening the knob.
        if app.params.gate_on.value() {
            ui.add_space(8.0);
            let label = division_label(app.params.gate_rate.value() as usize);
            ui.label(egui::RichText::new(format!("GATE {label}")).color(theme::ACCENT));
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
