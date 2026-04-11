//! Top header bar: title, Load Model button, Prev/Next browser, and
//! the current model name. Extracted from `editor/mod.rs` so the main
//! module stays focused on layout.

use std::path::Path;
use std::sync::atomic::Ordering;

use wayland_plugin_gui::egui;

use super::theme;
use super::AmpEditorApp;

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
        // next to it.
        let tone3000_btn = egui::Button::new(
            egui::RichText::new("Browse Tone3000…")
                .color(egui::Color32::BLACK)
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

        let list_len = app.params.file_list.lock().len();
        let enabled = list_len > 1;
        ui.add_enabled_ui(enabled, |ui| {
            if ui.button("◀").clicked() {
                seek_relative(app, -1);
            }
            if ui.button("▶").clicked() {
                seek_relative(app, 1);
            }
        });

        ui.add_space(12.0);

        // Current model name + position counter.
        let current_index = app.params.file_select.value() as usize;
        let (name_text, position_text) = {
            let list = app.params.file_list.lock();
            let len = list.len();
            let clamped = current_index.min(len.saturating_sub(1));
            let stem = list
                .get(clamped)
                .and_then(|p| {
                    Path::new(p)
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                })
                .unwrap_or_default();
            drop(list);

            let raw_name = app.model_name.lock().clone();
            let name = if raw_name.is_empty() {
                if stem.is_empty() {
                    "(no model loaded)".to_string()
                } else {
                    stem.clone()
                }
            } else {
                raw_name
            };

            let position = if len == 0 {
                String::new()
            } else {
                format!("{} / {}", clamped + 1, len)
            };
            (name, position)
        };

        ui.label(
            egui::RichText::new(name_text)
                .size(13.0)
                .color(theme::TEXT),
        );
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(position_text)
                .size(11.0)
                .color(theme::TEXT_DIM),
        );
    });
}

fn load_model_clicked(app: &AmpEditorApp) {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("NAM model", &["nam"])
        .pick_file()
    else {
        return;
    };
    let path_str = path.to_string_lossy().into_owned();

    let Some(dir) = path.parent() else {
        return;
    };
    let files = resonance_common::scan_directory(dir, "nam");
    let idx = files.iter().position(|f| f == &path_str).unwrap_or(0);

    *app.params.file_list.lock() = files;
    *app.params.model_path.lock() = path_str;
    app.params.file_select.set_value(idx as i32);
    app.load_request.store(idx as i32, Ordering::Release);
}

fn seek_relative(app: &AmpEditorApp, delta: i32) {
    let len = app.params.file_list.lock().len();
    if len == 0 {
        return;
    }
    let len_i = len as i32;
    let current = app.params.file_select.value();
    let next = (current + delta).rem_euclid(len_i);
    app.params.file_select.set_value(next);
    app.load_request.store(next, Ordering::Release);
}
