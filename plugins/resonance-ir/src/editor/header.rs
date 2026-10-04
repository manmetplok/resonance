//! Top header bar: title, Load IR button, Prev/Next browser, current
//! filename + info. Mirrors the amp editor's header shape.

use std::path::Path;
use std::sync::atomic::Ordering;

use plugin_gui_core::egui;

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
            start_load_ir(ui.ctx(), app);
        }
        #[cfg(not(target_os = "macos"))]
        poll_load_ir(ui.ctx(), app);

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        // Presets. The IR loader ships no factory bank, so until ba todo
        // #1332 a cab setup a user had dialled in could not be kept at all
        // outside the one project it lived in.
        let refs: Vec<&dyn resonance_plugin::Param> = (0..crate::params::PARAM_COUNT)
            .map(|i| app.params.param_at(i))
            .collect();
        resonance_plugin::preset_ui::preset_bar(
            ui,
            "ir_preset",
            &mut app.preset_editor,
            &app.bank,
            &app.presets,
            &refs,
            "— preset —",
        );

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        let list_len = app.params.file_list.lock().len();
        let enabled = list_len > 1;
        ui.add_enabled_ui(enabled, |ui| {
            if ui.button("◀").clicked() {
                seek_relative(ui.ctx(), app, -1);
            }
            if ui.button("▶").clicked() {
                seek_relative(ui.ctx(), app, 1);
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
            // PUX-09: a load failure's only channel is "Error: {e}" in
            // this same string (`loader.rs`'s doc comment on
            // `load_into` says so) — shown here as plain filename text
            // it used to be indistinguishable from a real one at a
            // glance. The banner (`missing_banner::draw`, drawn over
            // the centre when `load_error` is `Some`) says the real
            // detail; this slot just stops pretending it's a filename.
            let name = if super::missing_banner::load_error(&raw_name).is_some() {
                "(load failed — see below)".to_string()
            } else if raw_name.is_empty() {
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

/// PUX-07: on Cocoa the dialog runs inside the guarded AppKit modal run
/// loop, so `Load IR…` stays a direct, synchronous `rfd` call there.
/// On Linux it runs on its own thread and is polled every frame
/// ([`poll_load_ir`]) — a modal `rfd::FileDialog::pick_file()` call
/// inside `ui()` would otherwise block the Wayland editor thread (no
/// repaint, no Wayland dispatch) for as long as the dialog is up.
#[cfg(target_os = "macos")]
pub(super) fn start_load_ir(ctx: &egui::Context, app: &IrEditorApp) {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Impulse response (WAV)", &["wav"])
        .pick_file()
    else {
        return;
    };
    apply_ir_path(ctx, app, path);
}

#[cfg(not(target_os = "macos"))]
pub(super) fn start_load_ir(ctx: &egui::Context, app: &IrEditorApp) {
    use resonance_plugin::file_picker::FileDialogRequest;
    app.ir_picker.lock().start(
        FileDialogRequest::open_file()
            .title("Load IR")
            .filter("Impulse response (WAV)", &["wav"]),
    );
    let _ = ctx; // kept for signature parity with the macOS path
}

#[cfg(not(target_os = "macos"))]
pub(super) fn poll_load_ir(ctx: &egui::Context, app: &IrEditorApp) {
    let Some(answer) = app.ir_picker.lock().poll() else {
        return;
    };
    if let Some(path) = answer.into_one() {
        apply_ir_path(ctx, app, path);
    }
}

/// Everything a chosen impulse-response path drives: the file-list
/// browser, `ir_path`, the `file_select` param (announced as an edit)
/// and the load request itself.
fn apply_ir_path(ctx: &egui::Context, app: &IrEditorApp, path: std::path::PathBuf) {
    let path_str = path.to_string_lossy().into_owned();

    let Some(dir) = path.parent() else {
        return;
    };
    let files = resonance_common::scan_directory(dir, "wav");
    let idx = files.iter().position(|f| f == &path_str).unwrap_or(0);

    *app.params.file_list.lock() = files;
    *app.params.ir_path.lock() = path_str;
    // A new file is an edit even at the same index (the path changed).
    app.params.file_select.set_value(idx as i32);
    resonance_plugin::editor_widgets::announce_edit(ctx, &app.params.file_select);
    app.load_request.store(idx as i32, Ordering::Release);
}

fn seek_relative(ctx: &egui::Context, app: &IrEditorApp, delta: i32) {
    let len = app.params.file_list.lock().len();
    if len == 0 {
        return;
    }
    let len_i = len as i32;
    let current = app.params.file_select.value();
    let next = (current + delta).rem_euclid(len_i);
    resonance_plugin::editor_widgets::commit_plain(ctx, &app.params.file_select, next as f64);
    app.load_request.store(next, Ordering::Release);
}
