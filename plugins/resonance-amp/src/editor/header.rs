//! Top header bar: title, the model entry points, the preset bar, ◀/▶ and
//! the current model name. Extracted from `editor/mod.rs` so the main
//! module stays focused on layout.

use plugin_gui_core::egui;

use super::{actions, theme, AmpEditorApp};

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

        if ui.button("Load Model…").clicked() {
            load_model_clicked(app);
        }

        ui.add_space(8.0);

        // Accent-coloured rich-text button so the Tone3000 entry point
        // is visually distinct from the plain "Load Model…" button
        // next to it. The label is the darkest surface token rather than
        // pure black: this is the fleet's only solid-accent fill, and the
        // canonical accent is a much darker violet than the blue it
        // replaced (ba todo #1338) — 5.7:1 against it, still past AA, but
        // the black-on-cyan headroom is gone, so the label has to be the
        // dark end of the palette and cannot drift lighter.
        let tone3000_btn = egui::Button::new(
            egui::RichText::new("Browse Tone3000…")
                .color(theme::BG_0)
                .strong()
                .size(13.0),
        )
        .fill(theme::ACCENT);
        if ui.add(tone3000_btn).clicked() {
            app.tone3000_panel.open = true;
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

        let slots = slotted(app);
        ui.add_enabled_ui(slots.len() > 1, |ui| {
            if ui.button("◀").clicked() {
                seek_relative(app, &slots, -1);
            }
            if ui.button("▶").clicked() {
                seek_relative(app, &slots, 1);
            }
        });

        ui.add_space(12.0);

        let status = app.params.status.lock().clone();
        let color = if status.is_missing() || status.deleted {
            theme::WARN
        } else {
            theme::TEXT
        };
        ui.label(egui::RichText::new(status.header_text()).size(13.0).color(color));
        ui.add_space(8.0);
        let current = app.params.file_select.value() as u32;
        let position = match slots.iter().position(|&s| s == current) {
            Some(i) if !status.is_missing() => format!("{} / {}", i + 1, slots.len()),
            _ if slots.is_empty() => String::new(),
            _ => format!("– / {}", slots.len()),
        };
        ui.label(egui::RichText::new(position).size(11.0).color(theme::TEXT_DIM));
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

/// The occupied slots, in slot order.
fn slotted(app: &AmpEditorApp) -> Vec<u32> {
    app.params
        .library
        .read()
        .entries()
        .iter()
        .filter_map(|e| e.slot)
        .collect()
}

fn load_model_clicked(app: &mut AmpEditorApp) {
    // A picked file is imported (copied into the library, deduplicated by
    // content) and loaded through its slot.
    let Some(path) = actions::pick_nam_files(false).into_iter().next() else {
        return;
    };
    app.notice = match actions::import_and_load(app, &path) {
        Ok(resonance_common::nam_library::ImportOutcome::AlreadyPresent(_)) => {
            Some("already in library".into())
        }
        Ok(_) => None,
        Err(e) => Some(e),
    };
}

fn seek_relative(app: &AmpEditorApp, slots: &[u32], delta: i32) {
    if slots.is_empty() {
        return;
    }
    let current = app.params.file_select.value() as u32;
    let playing = !app.params.model_ref.lock().is_empty();
    // With nothing loaded the selector is parked without that meaning
    // "that slot is playing", so the first press loads where it points
    // (or the first model) rather than stepping past it.
    let target = match slots.iter().position(|&s| s == current) {
        Some(i) if playing => {
            let n = slots.len() as i32;
            slots[(i as i32 + delta).rem_euclid(n) as usize]
        }
        Some(i) => slots[i],
        None => slots[0],
    };
    actions::load_slot(app, target);
}
