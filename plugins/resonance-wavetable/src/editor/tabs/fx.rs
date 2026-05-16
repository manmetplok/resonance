//! FX tab — output scope at the top, then a horizontal chain of effect
//! cards (Chorus / Delay / Distortion) each with their own knobs.

use wayland_plugin_gui::egui;

use crate::editor::theme;
use crate::editor::viz::scope;
use crate::editor::WavetableEditorApp;
use resonance_plugin::param::Param;

use super::float_knob;

pub fn draw(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    ui.spacing_mut().item_spacing = egui::vec2(12.0, 10.0);

    let panel_frame = || {
        egui::Frame::default()
            .fill(theme::BG_2)
            .stroke(egui::Stroke::new(1.0, theme::LINE_2))
            .corner_radius(theme::RADIUS_PANEL)
            .inner_margin(egui::Margin::same(12))
    };

    // Output scope at the top.
    let scope_avail = ui.available_width();
    panel_frame().show(ui, |ui| {
        ui.set_min_width(scope_avail - 24.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("OUTPUT")
                    .color(theme::TEXT_3)
                    .size(10.5)
                    .strong(),
            );
        });
        let avail_inner = ui.available_width();
        let (_id, rect) = ui.allocate_space(egui::vec2(avail_inner, 84.0));
        scope::draw(ui, rect, &app.snapshot.scope_samples);
    });

    // Three FX cards.
    ui.columns(3, |cols| {
        draw_fx_card(
            &mut cols[0],
            "Chorus",
            "1",
            app.params.chorus.enabled.value(),
            |on| app.params.chorus.enabled.set_plain(on),
            |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
                    float_knob(ui, "Rate", &app.params.chorus.rate, Some("Hz"));
                    float_knob(ui, "Depth", &app.params.chorus.depth, None);
                    float_knob(ui, "Mix", &app.params.chorus.mix, None);
                });
            },
        );

        draw_fx_card(
            &mut cols[1],
            "Delay",
            "2",
            app.params.delay.enabled.value(),
            |on| app.params.delay.enabled.set_plain(on),
            |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
                    float_knob(ui, "Time L", &app.params.delay.time_l, Some("ms"));
                    float_knob(ui, "Time R", &app.params.delay.time_r, Some("ms"));
                    float_knob(ui, "Fb", &app.params.delay.feedback, None);
                    float_knob(ui, "Mix", &app.params.delay.mix, None);
                });
            },
        );

        draw_fx_card(
            &mut cols[2],
            "Distortion",
            "3",
            app.params.distortion.enabled.value(),
            |on| app.params.distortion.enabled.set_plain(on),
            |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
                    float_knob(ui, "Drive", &app.params.distortion.drive, None);
                    float_knob(ui, "Mix", &app.params.distortion.mix, None);
                });
            },
        );
    });
}

fn draw_fx_card(
    ui: &mut egui::Ui,
    name: &str,
    slot: &str,
    enabled: bool,
    mut on_toggle: impl FnMut(f64),
    body: impl FnOnce(&mut egui::Ui),
) {
    let avail = ui.available_width();
    let stroke_color = if enabled {
        theme::ACCENT
    } else {
        theme::LINE_2
    };
    let frame = egui::Frame::default()
        .fill(theme::BG_2)
        .stroke(egui::Stroke::new(1.0, stroke_color))
        .corner_radius(theme::RADIUS_PANEL)
        .inner_margin(egui::Margin::same(12));
    frame.show(ui, |ui| {
        ui.set_min_width(avail - 24.0);
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 10.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("SLOT {}", slot))
                    .color(theme::TEXT_4)
                    .size(9.5)
                    .strong()
                    .monospace(),
            );
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(name)
                    .color(theme::TEXT_1)
                    .size(13.0)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (r, resp) = ui.allocate_exact_size(
                    egui::vec2(14.0, 14.0),
                    egui::Sense::click(),
                );
                ui.painter()
                    .circle_filled(r.center(), 7.0, theme::BG_1);
                ui.painter().circle_stroke(
                    r.center(),
                    7.0,
                    egui::Stroke::new(1.0, theme::LINE),
                );
                let core_color = if enabled { theme::GOOD } else { theme::TEXT_4 };
                ui.painter().circle_filled(r.center(), 2.5, core_color);
                if resp.clicked() {
                    on_toggle(if enabled { 0.0 } else { 1.0 });
                }
            });
        });
        body(ui);
    });
}
