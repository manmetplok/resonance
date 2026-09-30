//! Top header bar: title, the model entry points, the preset bar, ◀/▶ and
//! the current model name. Extracted from `editor/mod.rs` so the main
//! module stays focused on layout.

use plugin_gui_core::egui;

use super::{actions, theme, AmpEditorApp};
use crate::library_rows::{step_in_view, view_counter};

pub fn draw(ui: &mut egui::Ui, app: &mut AmpEditorApp) {
    ui.horizontal_centered(|ui| {
        ui.add_space(12.0);
        ui.label(
            egui::RichText::new("RESONANCE AMP")
                .strong()
                .color(theme::ACCENT)
                .size(14.0),
        );

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);

        // One entry point for every model source (nam-model-library.md
        // §6.1). The fleet's only solid-accent fill; the label is the darkest
        // surface token because the canonical accent is a dark violet
        // (5.7:1 against it, past AA; ba todo #1338).
        let library_btn = egui::Button::new(
            egui::RichText::new("Library…")
                .color(theme::BG_0)
                .strong()
                .size(13.0),
        )
        .fill(theme::ACCENT);
        if ui.add(library_btn).clicked() {
            app.open_library();
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        // Presets. The amp ships no factory bank, so until ba todo #1332
        // a rig a user had dialled in around a model could not be kept at
        // all outside the one project it lived in.
        let refs: Vec<&dyn resonance_plugin::Param> = (0..crate::params::PARAM_COUNT)
            .map(|i| app.params.param_at(i))
            .collect();
        resonance_plugin::preset_ui::preset_bar(
            ui,
            "amp_preset",
            &mut app.preset_editor,
            &app.bank,
            &app.presets,
            &refs,
            "— preset —",
        );

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        // ◀/▶ walk the Library panel's current view (search, filters,
        // sort, favourites first), not slot order: "next" is what the user
        // sees next (§5.1). They write the slot number.
        app.refresh_rows();
        let status = app.params.status.lock().clone();
        let loaded_id = if status.state == crate::model_ref::ModelState::Loaded {
            status.id.clone()
        } else {
            None
        };
        ui.add_enabled_ui(app.browser.view_len() > 0, |ui| {
            if ui.button("◀").clicked() {
                step(app, loaded_id.as_deref(), -1);
            }
            if ui.button("▶").clicked() {
                step(app, loaded_id.as_deref(), 1);
            }
        });

        ui.add_space(12.0);

        let color = if status.is_missing() || status.deleted {
            theme::WARN
        } else {
            theme::TEXT
        };
        // The name opens the Library too.
        let name = ui
            .add(
                egui::Label::new(egui::RichText::new(status.header_text()).size(13.0).color(color))
                    .sense(egui::Sense::click()),
            )
            .on_hover_text("Open the model library");
        if name.clicked() {
            app.open_library();
        }
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(view_counter(&app.browser, loaded_id.as_deref()))
                .size(11.0)
                .color(theme::TEXT_DIM),
        );
        if let Some(notice) = status.notice.as_ref().or(app.notice.as_ref()) {
            ui.add_space(8.0);
            ui.label(egui::RichText::new(notice).size(11.0).color(theme::TEXT_DIM));
        }

        // Sample-rate mismatch warning: a NAM profile runs sample-for-sample
        // at the engine rate, so a rate mismatch shifts its frequency
        // response.
        let (model_hz, engine_hz) = app.viz.read_sample_rates();
        if model_hz > 0.0 && engine_hz > 0.0 && (model_hz - engine_hz).abs() > 0.5 {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new(format!(
                    "⚠ model {} / host {}",
                    format_khz(model_hz),
                    format_khz(engine_hz)
                ))
                .size(11.0)
                .color(theme::WARN),
            )
            .on_hover_text(
                "This NAM profile was captured at a different sample rate than \
                 the host is running at, so the amp's frequency response is \
                 shifted. Run the session at the model's rate for an accurate tone.",
            );
        }
    });
}

pub(crate) fn format_khz(hz: f32) -> String {
    let khz = hz / 1000.0;
    if (khz - khz.round()).abs() < 0.05 {
        format!("{:.0} kHz", khz)
    } else {
        format!("{:.1} kHz", khz)
    }
}

fn step(app: &mut AmpEditorApp, loaded_id: Option<&str>, delta: i32) {
    if let Some(slot) = step_in_view(&app.browser, &app.rows, loaded_id, delta) {
        actions::load_slot(app, slot);
    }
}
