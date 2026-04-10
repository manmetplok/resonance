//! Modulation matrix tab: 8 rows of source/destination/amount.

use wayland_plugin_gui::egui;

use crate::editor::theme;
use crate::editor::WavetableEditorApp;
use crate::modulation::NUM_MOD_SLOTS;
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
    ui.spacing_mut().item_spacing.y = 4.0;

    ui.label(
        egui::RichText::new("MODULATION MATRIX")
            .color(theme::TEXT_DIM)
            .size(10.0)
            .strong(),
    );
    ui.separator();

    egui::Grid::new("mod_matrix_grid")
        .num_columns(4)
        .min_col_width(120.0)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            ui.label(egui::RichText::new("#").color(theme::TEXT_DIM));
            ui.label(egui::RichText::new("Source").color(theme::TEXT_DIM));
            ui.label(egui::RichText::new("Destination").color(theme::TEXT_DIM));
            ui.label(egui::RichText::new("Amount").color(theme::TEXT_DIM));
            ui.end_row();

            for i in 0..NUM_MOD_SLOTS {
                let slot = &app.params.mod_slots[i];

                ui.label(egui::RichText::new(format!("{}", i + 1)).color(theme::TEXT_DIM));

                // Source combo box.
                let mut src_idx = slot.source.value().clamp(0, (SOURCE_NAMES.len() - 1) as i32) as usize;
                egui::ComboBox::from_id_salt(format!("src_{}", i))
                    .selected_text(SOURCE_NAMES[src_idx])
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        for (j, name) in SOURCE_NAMES.iter().enumerate() {
                            if ui.selectable_label(src_idx == j, *name).clicked() {
                                src_idx = j;
                                slot.source.set_plain(j as f64);
                            }
                        }
                    });

                // Destination combo box.
                let mut dst_idx = slot.destination.value().clamp(0, (DEST_NAMES.len() - 1) as i32) as usize;
                egui::ComboBox::from_id_salt(format!("dst_{}", i))
                    .selected_text(DEST_NAMES[dst_idx])
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        for (j, name) in DEST_NAMES.iter().enumerate() {
                            if ui.selectable_label(dst_idx == j, *name).clicked() {
                                dst_idx = j;
                                slot.destination.set_plain(j as f64);
                            }
                        }
                    });

                // Amount slider + colored magnitude bar.
                let mut amount = slot.amount.value();
                let slider = ui.add(
                    egui::Slider::new(&mut amount, -1.0..=1.0)
                        .custom_formatter(|v, _| format!("{:+.2}", v)),
                );
                if slider.changed() {
                    slot.amount.set_value(amount);
                }

                ui.end_row();
            }
        });
}
