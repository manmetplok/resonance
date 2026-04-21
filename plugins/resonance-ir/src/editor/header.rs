//! Top header bar: title, Load IR button, Prev/Next browser, current
//! filename + info. Mirrors the amp editor's header shape.

use std::path::Path;
use std::sync::atomic::Ordering;

use wayland_plugin_gui::egui;

use super::theme;
use super::IrEditorApp;

pub fn draw(ui: &mut egui::Ui, app: &mut IrEditorApp) {
    ui.horizontal_centered(|ui| {
        ui.add_space(12.0);
        ui.label(
            egui::RichText::new("RESONANCE IR")
                .strong()
                .color(theme::ACCENT)
                .size(14.0),
        );

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);

        if ui.button("Load IR…").clicked() {
            load_ir_clicked(app);
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

        // Filename + position + info text.
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

            let raw_name = app.ir_name.lock().clone();
            let name = if raw_name.is_empty() {
                if stem.is_empty() {
                    "(no IR loaded)".to_string()
                } else {
                    stem
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

        ui.label(egui::RichText::new(name_text).size(13.0).color(theme::TEXT));
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(position_text)
                .size(11.0)
                .color(theme::TEXT_DIM),
        );

        ui.add_space(12.0);
        let info = app.ir_info.lock().clone();
        if !info.is_empty() {
            ui.label(egui::RichText::new(info).size(11.0).color(theme::TEXT_DIM));
        }
    });
}

fn load_ir_clicked(app: &IrEditorApp) {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Impulse response (WAV)", &["wav"])
        .pick_file()
    else {
        return;
    };
    let path_str = path.to_string_lossy().into_owned();

    let Some(dir) = path.parent() else {
        return;
    };
    let files = resonance_common::scan_directory(dir, "wav");
    let idx = files.iter().position(|f| f == &path_str).unwrap_or(0);

    *app.params.file_list.lock() = files;
    *app.params.ir_path.lock() = path_str;
    app.params.file_select.set_value(idx as i32);
    app.load_request.store(idx as i32, Ordering::Release);
}

fn seek_relative(app: &IrEditorApp, delta: i32) {
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
