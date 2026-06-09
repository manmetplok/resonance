//! Modulation matrix tab — numbered rows of source → destination with a
//! bipolar amount slider per row.

use wayland_plugin_gui::egui;

use crate::editor::theme;
use crate::editor::widgets;
use crate::editor::WavetableEditorApp;
use crate::dsp::modulation::NUM_MOD_SLOTS;
use resonance_plugin::param::Param;

const SOURCE_NAMES: [&str; 9] = [
    "None",
    "LFO 1",
    "LFO 2",
    "LFO 3",
    "Mod Env",
    "Velocity",
    "Key Track",
    "Mod Wheel",
    "Aftertouch",
];

const DEST_NAMES: [&str; 12] = [
    "None",
    "Osc1 Position",
    "Osc2 Position",
    "Osc1 Pitch",
    "Osc2 Pitch",
    "Filter Cutoff",
    "Filter Reso",
    "Osc Balance",
    "Amp Level",
    "Unison Detune",
    "Osc1 Pan",
    "Osc2 Pan",
];

pub fn draw(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    ui.spacing_mut().item_spacing = egui::vec2(0.0, 10.0);

    let panel = egui::Frame::default()
        .fill(theme::BG_2)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(theme::RADIUS_PANEL)
        .inner_margin(egui::Margin::same(14));
    let avail_w = ui.available_width();
    panel.show(ui, |ui| {
        ui.set_min_width(avail_w - 28.0);
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 4.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("MODULATION MATRIX")
                    .color(theme::TEXT_3)
                    .size(10.5)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let active = (0..NUM_MOD_SLOTS)
                    .filter(|i| {
                        app.params.mod_slots[*i].source.value() != 0
                            && app.params.mod_slots[*i].destination.value() != 0
                    })
                    .count();
                ui.label(
                    egui::RichText::new(format!("{} active · {} slots", active, NUM_MOD_SLOTS))
                        .color(theme::TEXT_3)
                        .size(10.5)
                        .monospace(),
                );
            });
        });

        ui.separator();

        // Headers.
        egui::Grid::new("mod_matrix_grid_header")
            .num_columns(5)
            .spacing([12.0, 4.0])
            .min_col_width(40.0)
            .show(ui, |ui| {
                let mk = |s: &str| {
                    egui::RichText::new(s)
                        .color(theme::TEXT_4)
                        .size(9.5)
                        .strong()
                };
                ui.label(mk("#"));
                ui.label(mk("SOURCE"));
                ui.label(mk(""));
                ui.label(mk("DESTINATION"));
                ui.label(mk("AMOUNT"));
                ui.end_row();
            });

        // Rows.
        for i in 0..NUM_MOD_SLOTS {
            let slot = &app.params.mod_slots[i];
            let off = slot.source.value() == 0 || slot.destination.value() == 0;

            let row_frame = egui::Frame::default()
                .inner_margin(egui::Margin::symmetric(6, 4))
                .corner_radius(6.0);
            row_frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Index.
                    ui.add_sized(
                        egui::vec2(20.0, 0.0),
                        egui::Label::new(
                            egui::RichText::new(format!("{:02}", i + 1))
                                .monospace()
                                .size(10.0)
                                .color(if off { theme::TEXT_4 } else { theme::TEXT_3 }),
                        ),
                    );
                    ui.add_space(6.0);

                    // Source.
                    draw_mtx_pill(
                        ui,
                        "src",
                        i,
                        slot.source.value() as usize,
                        &SOURCE_NAMES,
                        theme::WARM,
                        |v| slot.source.set_plain(v as f64),
                        160.0,
                    );

                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("▸")
                            .color(theme::TEXT_4)
                            .size(11.0),
                    );
                    ui.add_space(6.0);

                    // Destination.
                    draw_mtx_pill(
                        ui,
                        "dst",
                        i,
                        slot.destination.value() as usize,
                        &DEST_NAMES,
                        theme::ACCENT,
                        |v| slot.destination.set_plain(v as f64),
                        180.0,
                    );

                    ui.add_space(10.0);

                    // Amount slider (compact, bipolar).
                    let slider_w =
                        (ui.available_width() - 48.0).clamp(80.0, 200.0);
                    if let Some(v) = widgets::slider_bipolar(ui, slider_w, slot.amount.value()) {
                        slot.amount.set_value(v);
                    }
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(format!("{:+.2}", slot.amount.value()))
                            .monospace()
                            .size(10.5)
                            .color(if slot.amount.value() < 0.0 {
                                theme::WARM
                            } else {
                                theme::TEXT_1
                            }),
                    );
                });
            });
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn draw_mtx_pill(
    ui: &mut egui::Ui,
    kind: &str,
    row: usize,
    current: usize,
    options: &[&str],
    dot_color: egui::Color32,
    mut on_select: impl FnMut(usize),
    width: f32,
) {
    let frame = egui::Frame::default()
        .fill(theme::BG_1)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(5.0)
        .inner_margin(egui::Margin::symmetric(8, 3));
    frame.show(ui, |ui| {
        ui.set_min_width(width - 16.0);
        ui.horizontal(|ui| {
            let (r, _) =
                ui.allocate_exact_size(egui::vec2(7.0, 7.0), egui::Sense::hover());
            ui.painter().circle_filled(
                r.center(),
                3.0,
                if current == 0 { theme::TEXT_4 } else { dot_color },
            );
            ui.add_space(6.0);
            let current_label = options.get(current).copied().unwrap_or("?");
            egui::ComboBox::from_id_salt(format!("mtx_{}_{}", kind, row))
                .selected_text(
                    egui::RichText::new(current_label)
                        .color(if current == 0 {
                            theme::TEXT_4
                        } else {
                            theme::TEXT_1
                        })
                        .size(11.0),
                )
                .width(width - 28.0)
                .show_ui(ui, |ui| {
                    for (i, name) in options.iter().enumerate() {
                        if ui.selectable_label(i == current, *name).clicked() {
                            on_select(i);
                        }
                    }
                });
        });
    });
}
