//! OSC tab: wavetable viewer + osc selector + per-osc controls + unison.

use wayland_plugin_gui::egui;

use crate::editor::display_waves;
use crate::editor::theme;
use crate::editor::viz::{frame_strip, waveform};
use crate::editor::widgets;
use crate::editor::WavetableEditorApp;
use resonance_plugin::param::Param;

use super::{float_knob, float_knob_bipolar, int_knob};

pub fn draw(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    ui.spacing_mut().item_spacing = egui::vec2(12.0, 10.0);

    // Two-column body: left = wave panel, right = params panel.
    ui.columns(2, |cols| {
        draw_osc_panel(&mut cols[0], app);
        draw_params_panel(&mut cols[1], app);
    });

    ui.add_space(2.0);

    // Bottom row: Unison + Global cards.
    ui.columns(2, |cols| {
        draw_unison_card(&mut cols[0], app);
        draw_global_card(&mut cols[1], app);
    });
}

fn panel_frame() -> egui::Frame {
    egui::Frame::default()
        .fill(theme::BG_2)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(theme::RADIUS_PANEL)
        .inner_margin(egui::Margin::same(12))
}

/// Helper: render `body` inside a panel frame that fills the column width.
fn panel<R>(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let avail_w = ui.available_width();
    let mut out = None;
    panel_frame().show(ui, |ui| {
        ui.set_min_width(avail_w - 24.0); // subtract inner margin*2
        out = Some(body(ui));
    });
    out.expect("body always runs")
}

fn panel_title(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .color(theme::TEXT_3)
            .size(10.5)
            .strong(),
    );
}

fn draw_osc_panel(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    panel(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(10.0, 8.0);

        // Header: Osc1/Osc2 segmented + balance.
        ui.horizontal(|ui| {
            let labels = ["Osc 1", "Osc 2"];
            if let Some(i) = widgets::segmented(ui, &labels, app.selected_osc, false) {
                app.selected_osc = i;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!("{:+.2}", app.params.osc_balance.value()))
                        .monospace()
                        .color(theme::TEXT_1)
                        .size(11.0),
                );
                ui.add_space(8.0);
                let new = widgets::slider_bipolar(ui, 110.0, app.params.osc_balance.value());
                if let Some(v) = new {
                    app.params.osc_balance.set_value(v);
                }
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("BALANCE")
                        .color(theme::TEXT_3)
                        .size(9.5)
                        .strong(),
                );
            });
        });

        let (osc_params, live_pos) = if app.selected_osc == 0 {
            (&app.params.osc1, app.snapshot.osc1_position_live)
        } else {
            (&app.params.osc2, app.snapshot.osc2_position_live)
        };

        let wt_idx = osc_params.wavetable.value() as usize;
        let position = osc_params.position.value();

        // Wave display.
        let avail = ui.available_width();
        let (_id, rect) = ui.allocate_space(egui::vec2(avail, 170.0));
        waveform::draw(ui, rect, wt_idx, position, live_pos);

        // Frame strip.
        let (_id2, strip_rect) = ui.allocate_space(egui::vec2(avail, 24.0));
        frame_strip::draw(ui, strip_rect, wt_idx, position);

        // Wavetable category row.
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Button::new(egui::RichText::new("◀").color(theme::TEXT_3).size(10.0))
                        .frame(false),
                )
                .clicked()
                && wt_idx > 0
            {
                osc_params.wavetable.set_plain((wt_idx - 1) as f64);
            }
            ui.label(
                egui::RichText::new(display_waves::wavetable_name(wt_idx))
                    .color(theme::TEXT_1)
                    .size(12.0)
                    .strong(),
            );
            if ui
                .add(
                    egui::Button::new(egui::RichText::new("▶").color(theme::TEXT_3).size(10.0))
                        .frame(false),
                )
                .clicked()
                && wt_idx + 1 < display_waves::WAVETABLE_NAMES.len()
            {
                osc_params.wavetable.set_plain((wt_idx + 1) as f64);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let frames = display_waves::frame_count(wt_idx);
                let frame_idx = ((position * (frames.saturating_sub(1)).max(1) as f32)
                    .round() as usize)
                    .min(frames.saturating_sub(1));
                ui.label(
                    egui::RichText::new(format!("FRAME {} / {}", frame_idx + 1, frames))
                        .color(theme::TEXT_3)
                        .size(10.0)
                        .monospace(),
                );
            });
        });
    });
}

fn draw_params_panel(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    panel(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);

        let osc_params = if app.selected_osc == 0 {
            &app.params.osc1
        } else {
            &app.params.osc2
        };
        let title = if app.selected_osc == 0 { "Osc 1" } else { "Osc 2" };
        let wt_idx = osc_params.wavetable.value() as usize;

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(title)
                    .color(theme::TEXT_1)
                    .size(13.0)
                    .strong(),
            );
            ui.label(
                egui::RichText::new(format!(
                    "{} · {} frames",
                    display_waves::wavetable_name(wt_idx),
                    display_waves::frame_count(wt_idx)
                ))
                .color(theme::TEXT_3)
                .size(11.0),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let enabled = osc_params.enabled.value();
                let led_color = if enabled { theme::GOOD } else { theme::TEXT_4 };
                let frame = egui::Frame::default()
                    .fill(theme::BG_1)
                    .stroke(egui::Stroke::new(1.0, theme::LINE_2))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(9, 4));
                let resp = frame
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(
                                egui::vec2(8.0, 8.0),
                                egui::Sense::hover(),
                            );
                            ui.painter().circle_filled(r.center(), 4.0, led_color);
                            ui.label(
                                egui::RichText::new(if enabled { "Enabled" } else { "Bypassed" })
                                    .color(theme::TEXT_2)
                                    .size(11.0),
                            );
                        });
                    })
                    .response;
                if resp.interact(egui::Sense::click()).clicked() {
                    osc_params.enabled.set_plain(if enabled { 0.0 } else { 1.0 });
                }
            });
        });
        ui.separator();

        // Knob row.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
            float_knob(ui, "Position", &osc_params.position, None);
            int_knob(ui, "Coarse", &osc_params.coarse);
            float_knob_bipolar(ui, "Fine", &osc_params.fine, Some("ct"));
            float_knob(ui, "Level", &osc_params.level, None);
            float_knob_bipolar(ui, "Pan", &osc_params.pan, None);
        });

        // Routes-to footer.
        ui.add_space(2.0);
        let footer = egui::Frame::default()
            .fill(theme::BG_1)
            .stroke(egui::Stroke::new(1.0, theme::LINE_2))
            .corner_radius(8.0)
            .inner_margin(egui::Margin::symmetric(11, 8));
        footer.show(ui, |ui| {
            ui.horizontal(|ui| {
                let (r, _) =
                    ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                ui.painter().circle_filled(r.center(), 3.0, theme::ACCENT);
                ui.label(
                    egui::RichText::new("Routes to")
                        .color(theme::TEXT_1)
                        .size(11.5),
                );
                ui.label(
                    egui::RichText::new("Filter · Amp")
                        .color(theme::TEXT_3)
                        .size(11.0),
                );
            });
        });
    });
}

fn draw_unison_card(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    panel(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(10.0, 8.0);
        ui.horizontal(|ui| {
            panel_title(ui, "UNISON");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let v = app.params.unison.voices.value();
                ui.label(
                    egui::RichText::new(format!("{} voice{}", v, if v == 1 { "" } else { "s" }))
                        .color(theme::TEXT_3)
                        .size(10.5)
                        .monospace(),
                );
            });
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
            int_knob(ui, "Voices", &app.params.unison.voices);
            float_knob(ui, "Detune", &app.params.unison.detune, Some("ct"));
            float_knob(ui, "Spread", &app.params.unison.spread, None);
        });
    });
}

fn draw_global_card(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    panel(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(10.0, 8.0);
        ui.horizontal(|ui| {
            panel_title(ui, "GLOBAL");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let max = app.params.max_voices.value();
                ui.label(
                    egui::RichText::new(format!("poly · {} max", max))
                        .color(theme::TEXT_3)
                        .size(10.5)
                        .monospace(),
                );
            });
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
            float_knob(ui, "Master", &app.params.master_volume, None);
            float_knob(ui, "Glide", &app.params.glide_time, Some("s"));
            int_knob(ui, "Max", &app.params.max_voices);
        });

        // Glide toggle below the knob row.
        let on = app.params.glide_enabled.value();
        let resp = ui
            .horizontal(|ui| {
                let (rect, r) = ui.allocate_exact_size(
                    egui::vec2(32.0, 18.0),
                    egui::Sense::click(),
                );
                let pill_color = if on { theme::ACCENT } else { theme::BG_3 };
                ui.painter().rect_filled(rect, 9.0, pill_color);
                ui.painter().rect_stroke(
                    rect,
                    9.0,
                    egui::Stroke::new(
                        1.0,
                        if on { theme::ACCENT } else { theme::LINE },
                    ),
                    egui::StrokeKind::Inside,
                );
                let knob_x = if on { rect.right() - 9.0 } else { rect.left() + 9.0 };
                let knob_color = if on {
                    egui::Color32::WHITE
                } else {
                    theme::TEXT_3
                };
                ui.painter()
                    .circle_filled(egui::pos2(knob_x, rect.center().y), 6.0, knob_color);
                ui.label(
                    egui::RichText::new("Glide on")
                        .color(theme::TEXT_1)
                        .size(11.0),
                );
                r
            })
            .inner;
        if resp.clicked() {
            app.params.glide_enabled.set_plain(if on { 0.0 } else { 1.0 });
        }
    });
}
