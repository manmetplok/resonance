//! Modulation matrix tab — numbered rows of source → destination with a
//! bipolar amount slider per row.

use plugin_gui_core::egui;

use crate::editor::theme;
use crate::editor::WavetableEditorApp;
use crate::dsp::modulation::{ModDest, ModSlot, ModSource, NUM_MOD_SLOTS};
use resonance_plugin::param::Param;

use super::{float_slider, readout};

// Label tables live next to the enum discriminants in `dsp::modulation` so
// the picker cannot drift from what the DSP actually matches on. Which of
// them this build can act on is decided there too — see
// `ModSource::unavailable_reason` / `ModDest::unavailable_reason`.
const SOURCE_NAMES: [&str; 9] = ModSource::LABELS;
const DEST_NAMES: [&str; 12] = ModDest::LABELS;

/// Marker appended to an option the DSP cannot act on.
const UNAVAILABLE_MARK: &str = "⚠";

/// Read one matrix row's params back into the DSP's own slot type, so the
/// editor asks `dsp::modulation` what a routing does instead of duplicating
/// the rules.
pub(crate) fn slot_of(app: &WavetableEditorApp, index: usize) -> ModSlot {
    let slot = &app.params.mod_slots[index];
    ModSlot {
        source: ModSource::from_int(slot.source.value()),
        dest: ModDest::from_int(slot.destination.value()),
        amount: slot.amount.value(),
    }
}

/// All eight matrix rows as DSP slots.
pub(crate) fn slots_of(app: &WavetableEditorApp) -> Vec<ModSlot> {
    (0..NUM_MOD_SLOTS).map(|i| slot_of(app, i)).collect()
}

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
                // "Active" means the routing can change the sound, so a slot
                // wired to an unimplemented end is not counted — it is
                // reported separately as inert.
                let slots = slots_of(app);
                let active = slots.iter().filter(|s| s.is_effective()).count();
                let inert = slots
                    .iter()
                    .filter(|s| {
                        s.source != ModSource::None
                            && s.dest != ModDest::None
                            && !s.is_effective()
                    })
                    .count();
                if inert > 0 {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} {} inert",
                            UNAVAILABLE_MARK, inert
                        ))
                        .color(theme::WARM)
                        .size(10.5)
                        .monospace(),
                    )
                    .on_hover_text(
                        "These routings are saved but produce no modulation in this build.",
                    );
                    ui.add_space(8.0);
                }
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
                        |v| ModSource::from_int(v as i32).unavailable_reason(),
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
                        |v| ModDest::from_int(v as i32).unavailable_reason(),
                        theme::ACCENT,
                        |v| slot.destination.set_plain(v as f64),
                        180.0,
                    );

                    ui.add_space(10.0);

                    // Amount slider (compact; bipolar comes off the range).
                    let slider_w =
                        (ui.available_width() - 48.0).clamp(80.0, 200.0);
                    float_slider(ui, slider_w, &slot.amount);
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(readout(&slot.amount))
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
    // `None` for an option this build cannot act on, carrying the reason.
    unavailable: impl Fn(usize) -> Option<&'static str>,
    dot_color: egui::Color32,
    mut on_select: impl FnMut(usize),
    width: f32,
) {
    let current_reason = unavailable(current);
    let frame = egui::Frame::default()
        .fill(theme::BG_1)
        .stroke(egui::Stroke::new(
            1.0,
            if current_reason.is_some() {
                theme::WARM
            } else {
                theme::LINE_2
            },
        ))
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
                match (current, current_reason) {
                    (0, _) => theme::TEXT_4,
                    (_, Some(_)) => theme::WARM,
                    _ => dot_color,
                },
            );
            ui.add_space(6.0);
            // A patch may already reference an option this build cannot act
            // on (three factory presets do). Say so on the closed pill —
            // silently doing nothing is what the audit flagged.
            let current_label = match (options.get(current).copied(), current_reason) {
                (Some(name), Some(_)) => format!("{} {}", name, UNAVAILABLE_MARK),
                (Some(name), None) => name.to_string(),
                (None, _) => "?".to_string(),
            };
            let combo = egui::ComboBox::from_id_salt(format!("mtx_{}_{}", kind, row))
                .selected_text(
                    egui::RichText::new(current_label)
                        .color(match (current, current_reason) {
                            (0, _) => theme::TEXT_4,
                            (_, Some(_)) => theme::WARM,
                            _ => theme::TEXT_1,
                        })
                        .size(11.0),
                )
                .width(width - 28.0)
                .show_ui(ui, |ui| {
                    for (i, name) in options.iter().enumerate() {
                        match unavailable(i) {
                            // Offered, but not selectable, and it says why.
                            Some(reason) => {
                                ui.add_enabled(
                                    false,
                                    egui::Button::selectable(
                                        i == current,
                                        format!("{} {}", name, UNAVAILABLE_MARK),
                                    ),
                                )
                                .on_disabled_hover_text(reason);
                            }
                            None => {
                                if ui.selectable_label(i == current, *name).clicked() {
                                    on_select(i);
                                }
                            }
                        }
                    }
                });
            if let Some(reason) = current_reason {
                combo.response.on_hover_text(reason);
            }
        });
    });
}
