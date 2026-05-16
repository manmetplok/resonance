//! ENV / FILTER tab — three envelope cards (Amp / Filter / Mod) on the left
//! plus a filter panel with type chips and response graph on the right.

use wayland_plugin_gui::egui;

use crate::editor::theme;
use crate::editor::viz::{envelope, filter_response};
use crate::editor::widgets;
use crate::editor::WavetableEditorApp;
use resonance_plugin::param::Param;

use super::{float_knob, float_knob_bipolar};

pub fn draw(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    ui.spacing_mut().item_spacing = egui::vec2(12.0, 10.0);
    ui.columns(2, |cols| {
        draw_env_stack(&mut cols[0], app);
        draw_filter_panel(&mut cols[1], app);
    });
}

fn panel_frame() -> egui::Frame {
    egui::Frame::default()
        .fill(theme::BG_2)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(theme::RADIUS_PANEL)
        .inner_margin(egui::Margin::same(12))
}

fn draw_env_stack(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    let avail = ui.available_width();

    let configs: [(&str, &str, egui::Color32); 2] = [
        ("Amp Env", "→ Voice gain", theme::GOOD),
        ("Mod Env", "→ unassigned", theme::TEXT_4),
    ];

    for (i, (title, target, led)) in configs.iter().enumerate() {
        let env = if i == 0 {
            &app.params.amp_env
        } else {
            &app.params.mod_env
        };
        let (live_value, live_stage) = if i == 0 {
            (app.snapshot.env_amp_value, app.snapshot.env_amp_stage)
        } else {
            (app.snapshot.env_mod_value, 0)
        };
        panel_frame().show(ui, |ui| {
            ui.set_min_width(avail - 24.0);
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);

            ui.horizontal(|ui| {
                let (r, _) =
                    ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                ui.painter().circle_filled(r.center(), 4.0, *led);
                ui.label(
                    egui::RichText::new(*title)
                        .color(theme::TEXT_1)
                        .size(12.0)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(*target)
                        .color(theme::TEXT_3)
                        .size(10.5)
                        .monospace(),
                );
            });

            // Envelope curve.
            let avail_inner = ui.available_width();
            let (_id, rect) = ui.allocate_space(egui::vec2(avail_inner, 78.0));
            let adsr = envelope::Adsr {
                attack: env.attack.value(),
                decay: env.decay.value(),
                sustain: env.sustain.value(),
                release: env.release.value(),
                curve: env.curve.value(),
            };
            envelope::draw(ui, rect, &adsr, Some(live_value), live_stage);

            // Stage knobs.
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
                float_knob(ui, "Attack", &env.attack, Some("s"));
                float_knob(ui, "Decay", &env.decay, Some("s"));
                float_knob(ui, "Sustain", &env.sustain, None);
                float_knob(ui, "Release", &env.release, Some("s"));
                float_knob_bipolar(ui, "Curve", &env.curve, None);
            });
        });
    }
}

fn draw_filter_panel(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    let avail = ui.available_width();
    panel_frame().show(ui, |ui| {
        ui.set_min_width(avail - 24.0);
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 10.0);

        // Header.
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("FILTER")
                    .color(theme::TEXT_3)
                    .size(10.5)
                    .strong(),
            );
            ui.add_space(8.0);

            // Enabled chip on the far right.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let enabled = app.params.filter.enabled.value();
                let led_color = if enabled { theme::GOOD } else { theme::TEXT_4 };
                let frame = egui::Frame::default()
                    .fill(theme::BG_1)
                    .stroke(egui::Stroke::new(1.0, theme::LINE_2))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(9, 4));
                let resp = frame
                    .show(ui, |ui| {
                        ui.with_layout(
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                let (r, _) = ui.allocate_exact_size(
                                    egui::vec2(8.0, 8.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().circle_filled(r.center(), 4.0, led_color);
                                ui.add_space(6.0);
                                ui.label(
                                    egui::RichText::new(if enabled {
                                        "Enabled"
                                    } else {
                                        "Bypassed"
                                    })
                                    .color(theme::TEXT_2)
                                    .size(11.0),
                                );
                            },
                        );
                    })
                    .response;
                if resp.interact(egui::Sense::click()).clicked() {
                    app.params
                        .filter
                        .enabled
                        .set_plain(if enabled { 0.0 } else { 1.0 });
                }
            });
        });

        // Filter-type chips. Limited to the four types the DSP exposes.
        let types = ["Lowpass", "Highpass", "Bandpass", "Notch"];
        let current = app.params.filter.filter_type.value();
        ui.horizontal_wrapped(|ui| {
            for (i, label) in types.iter().enumerate() {
                let active = i as i32 == current;
                if widgets::chip_button(ui, label, active) {
                    app.params.filter.filter_type.set_plain(i as f64);
                }
            }
        });

        // Response graph.
        let avail_inner = ui.available_width();
        let graph_h = (avail_inner * (9.0 / 22.0)).clamp(120.0, 180.0);
        let (_id, rect) = ui.allocate_space(egui::vec2(avail_inner, graph_h));
        filter_response::draw(
            ui,
            rect,
            app.params.filter.filter_type.value(),
            app.params.filter.cutoff.value(),
            app.params.filter.resonance.value(),
            app.params.filter.drive.value(),
            app.snapshot.filter_cutoff_live,
        );

        // Filter knob row.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
            float_knob(ui, "Cutoff", &app.params.filter.cutoff, Some("Hz"));
            float_knob(ui, "Reso", &app.params.filter.resonance, None);
            float_knob_bipolar(ui, "Env", &app.params.filter.env_depth, None);
            float_knob_bipolar(ui, "Key", &app.params.filter.keytrack, None);
            float_knob(ui, "Drive", &app.params.filter.drive, None);
        });
    });
}
