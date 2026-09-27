//! FX tab — output scope at the top, then a horizontal chain of effect
//! cards (Chorus / Delay / Distortion) each with their own knobs. The
//! chorus card also carries its mode selector (Classic / Juno I, II, I+II /
//! Ensemble).

use plugin_gui_core::{egui, widgets};

use crate::dsp::effects::ChorusMode;
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
                let chorus = &app.params.chorus;
                let mode = ChorusMode::from_int(chorus.mode.value());
                if let Some(i) = widgets::segmented(ui, &ChorusMode::LABELS, mode as usize) {
                    chorus.mode.set_plain(i as f64);
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
                    // The Juno modes run at the hardware's fixed rates and
                    // ignore the Rate parameter, so the knob is replaced by
                    // the rate they actually run at. Noise only exists on
                    // the BBD modes.
                    if let Some(rate) = mode.fixed_rate_label() {
                        fixed_readout(ui, "Rate", rate);
                    } else {
                        float_knob(ui, "Rate", &chorus.rate);
                    }
                    float_knob(ui, "Depth", &chorus.depth);
                    float_knob(ui, "Mix", &chorus.mix);
                    if mode.is_bbd() {
                        float_knob(ui, "Noise", &chorus.noise);
                    }
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
                    float_knob(ui, "Time L", &app.params.delay.time_l);
                    float_knob(ui, "Time R", &app.params.delay.time_r);
                    float_knob(ui, "Fb", &app.params.delay.feedback);
                    float_knob(ui, "Mix", &app.params.delay.mix);
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
                    float_knob(ui, "Drive", &app.params.distortion.drive);
                    float_knob(ui, "Mix", &app.params.distortion.mix);
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

/// A knob-sized cell showing a value the mode fixes, in place of a knob
/// whose parameter the mode ignores.
fn fixed_readout(ui: &mut egui::Ui, caption: &str, value: &str) {
    ui.allocate_ui_with_layout(
        egui::vec2(52.0, 72.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.add_space(20.0);
            ui.label(egui::RichText::new(value).color(theme::TEXT_2).size(11.0));
            ui.label(egui::RichText::new(caption).color(theme::TEXT_3).size(10.5));
        },
    );
}
