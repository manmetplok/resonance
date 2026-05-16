//! LFO tab — render LFO 1, 2, 3 each as a card with shape preview and
//! controls. The selected card gets a brighter LED.

use wayland_plugin_gui::egui;

use crate::editor::theme;
use crate::editor::viz::lfo_shape;
use crate::editor::widgets;
use crate::editor::WavetableEditorApp;
use resonance_plugin::param::Param;

use super::{float_knob, int_knob};

pub fn draw(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    ui.spacing_mut().item_spacing = egui::vec2(12.0, 10.0);

    let lfos: [(&str, &str); 3] = [
        ("LFO 1", "→ Wavetable Pos · Cutoff"),
        ("LFO 2", "→ Pan · Drive"),
        ("LFO 3", "→ Macro 4"),
    ];

    let mut clicked: Option<usize> = None;

    // First row: LFO 1 and 2 side-by-side; LFO 3 on a row of its own.
    ui.columns(2, |cols| {
        for (col_idx, lfo_idx) in [0usize, 1usize].iter().enumerate() {
            let (title, target) = lfos[*lfo_idx];
            if draw_lfo_card(&mut cols[col_idx], app, *lfo_idx, title, target) {
                clicked = Some(*lfo_idx);
            }
        }
    });
    if draw_lfo_card(ui, app, 2, lfos[2].0, lfos[2].1) {
        clicked = Some(2);
    }
    if let Some(idx) = clicked {
        app.selected_lfo = idx;
    }
}

fn panel_frame() -> egui::Frame {
    egui::Frame::default()
        .fill(theme::BG_2)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(theme::RADIUS_PANEL)
        .inner_margin(egui::Margin::same(12))
}

fn draw_lfo_card(
    ui: &mut egui::Ui,
    app: &mut WavetableEditorApp,
    idx: usize,
    title: &str,
    target: &str,
) -> bool {
    let avail = ui.available_width();
    let selected = app.selected_lfo == idx;
    let lfo = match idx {
        0 => &app.params.lfo1,
        1 => &app.params.lfo2,
        _ => &app.params.lfo3,
    };
    let live_phase = app.snapshot.lfo_phases[idx.min(2)];

    let mut clicked = false;
    let outer = panel_frame()
        .stroke(egui::Stroke::new(
            1.0,
            if selected { theme::ACCENT } else { theme::LINE_2 },
        ))
        .show(ui, |ui| {
            ui.set_min_width(avail - 24.0);
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);

            // Header.
            ui.horizontal(|ui| {
                let (r, _) =
                    ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                ui.painter().circle_filled(
                    r.center(),
                    4.0,
                    if selected { theme::ACCENT } else { theme::TEXT_4 },
                );
                ui.label(
                    egui::RichText::new(title)
                        .color(theme::TEXT_1)
                        .size(12.0)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(target)
                        .color(theme::TEXT_3)
                        .size(10.5)
                        .monospace(),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let labels = ["Free", "Sync", "Env"];
                    // Treat `retrigger` bool as a proxy mode: false=free, true=sync.
                    let mode = if lfo.retrigger.value() { 1 } else { 0 };
                    if let Some(i) = widgets::segmented(ui, &labels, mode, false) {
                        if i == 1 {
                            lfo.retrigger.set_plain(1.0);
                        } else {
                            lfo.retrigger.set_plain(0.0);
                        }
                    }
                });
            });

            // Shape stage.
            let avail_inner = ui.available_width();
            let (_id, rect) = ui.allocate_space(egui::vec2(avail_inner, 80.0));
            lfo_shape::draw(ui, rect, lfo.shape.value(), lfo.depth.value(), live_phase);

            // Controls.
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
                int_knob(ui, "Shape", &lfo.shape);
                float_knob(ui, "Rate", &lfo.rate, Some("Hz"));
                float_knob(ui, "Depth", &lfo.depth, None);
            });
        });

    if outer.response.interact(egui::Sense::click()).clicked() {
        clicked = true;
    }
    clicked
}
