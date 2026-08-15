//! LFO tab — render LFO 1, 2, 3 each as a card with shape preview and
//! controls. The selected card gets a brighter LED.

use wayland_plugin_gui::egui;

use crate::editor::theme;
use crate::editor::viz::lfo_shape;
use crate::editor::widgets;
use crate::dsp::lfo::LfoShape;
use crate::dsp::modulation::{routing_summary, ModSource};
use crate::editor::WavetableEditorApp;
use resonance_plugin::param::Param;

use super::mod_matrix::slots_of;
use super::{float_knob, int_knob_fmt};

const LFO_TITLES: [&str; 3] = ["LFO 1", "LFO 2", "LFO 3"];
const LFO_SOURCES: [ModSource; 3] = [ModSource::Lfo1, ModSource::Lfo2, ModSource::Lfo3];

pub fn draw(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    ui.spacing_mut().item_spacing = egui::vec2(12.0, 10.0);

    // Each card's subtitle is derived from the live mod matrix, so it says
    // what this LFO is actually wired to. The previous fixed strings named
    // targets ("Wavetable Pos", "Macro 4") that no routing produced.
    let slots = slots_of(app);
    let targets: [String; 3] = [
        routing_summary(&slots, LFO_SOURCES[0]),
        routing_summary(&slots, LFO_SOURCES[1]),
        routing_summary(&slots, LFO_SOURCES[2]),
    ];

    let mut clicked: Option<usize> = None;

    // First row: LFO 1 and 2 side-by-side; LFO 3 on a row of its own.
    ui.columns(2, |cols| {
        for (col_idx, lfo_idx) in [0usize, 1usize].iter().enumerate() {
            if draw_lfo_card(
                &mut cols[col_idx],
                app,
                *lfo_idx,
                LFO_TITLES[*lfo_idx],
                &targets[*lfo_idx],
            ) {
                clicked = Some(*lfo_idx);
            }
        }
    });
    if draw_lfo_card(ui, app, 2, LFO_TITLES[2], &targets[2]) {
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
                    // Exactly the two states `lfoN_retrigger` has. The third
                    // segment used to be "Env", which no parameter backed, so
                    // clicking it snapped straight back; "Sync" named tempo
                    // sync, which this bool has never been.
                    let labels = ["Free", "Retrig"];
                    let mode = usize::from(lfo.retrigger.value());
                    if let Some(i) = widgets::segmented(ui, &labels, mode, false) {
                        lfo.retrigger.set_plain(i as f64);
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
                int_knob_fmt(ui, "Shape", &lfo.shape, |v| {
                    LfoShape::from_int(v).label().to_string()
                });
                float_knob(ui, "Rate", &lfo.rate, Some("Hz"));
                float_knob(ui, "Depth", &lfo.depth, None);
            });
        });

    if outer.response.interact(egui::Sense::click()).clicked() {
        clicked = true;
    }
    clicked
}
