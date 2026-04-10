//! LFO tab: selector for LFO1/2/3 + shape preview + controls.

use wayland_plugin_gui::egui;

use crate::editor::theme;
use crate::editor::viz::lfo_shape;
use crate::editor::WavetableEditorApp;

use super::{bool_checkbox, float_slider, int_slider, section_header};

pub fn draw(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    ui.spacing_mut().item_spacing.y = 6.0;

    // LFO selector.
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("LFO").color(theme::TEXT_DIM));
        ui.selectable_value(&mut app.selected_lfo, 0, "LFO 1");
        ui.selectable_value(&mut app.selected_lfo, 1, "LFO 2");
        ui.selectable_value(&mut app.selected_lfo, 2, "LFO 3");
    });

    let lfo = match app.selected_lfo {
        0 => &app.params.lfo1,
        1 => &app.params.lfo2,
        _ => &app.params.lfo3,
    };
    let live_phase = app.snapshot.lfo_phases[app.selected_lfo.min(2)];

    ui.add_space(4.0);

    ui.horizontal(|ui| {
        // Shape preview.
        ui.vertical(|ui| {
            let (_id, rect) = ui.allocate_space(egui::vec2(360.0, 150.0));
            lfo_shape::draw(ui, rect, lfo.shape.value(), lfo.depth.value(), live_phase);
        });

        ui.add_space(12.0);

        // Controls.
        ui.vertical(|ui| {
            section_header(
                ui,
                match app.selected_lfo {
                    0 => "LFO 1",
                    1 => "LFO 2",
                    _ => "LFO 3",
                },
            );
            int_slider(ui, "Shape", &lfo.shape);
            let shape_label = match lfo.shape.value() {
                0 => "Sine",
                1 => "Triangle",
                2 => "Saw",
                3 => "Square",
                4 => "S&H",
                _ => "?",
            };
            ui.label(
                egui::RichText::new(shape_label)
                    .color(theme::ACCENT)
                    .size(10.0),
            );
            float_slider(ui, "Rate", &lfo.rate, Some(" Hz"));
            float_slider(ui, "Depth", &lfo.depth, None);
            bool_checkbox(ui, "Retrigger", &lfo.retrigger);
        });
    });

    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(
            "Tip: route this LFO to an oscillator position, filter cutoff, or osc pitch via the MOD tab.",
        )
        .color(theme::TEXT_DIM)
        .size(10.0),
    );
}
