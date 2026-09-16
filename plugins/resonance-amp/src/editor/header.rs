//! Top header bar: title, Load Model button, Prev/Next browser, and
//! the current model name. Extracted from `editor/mod.rs` so the main
//! module stays focused on layout.

use std::path::Path;
use std::sync::atomic::Ordering;

use plugin_gui_core::egui;

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
            // The stem is a stand-in for a load that has been requested
            // but has not finished naming itself yet, so it is only
            // honest while a model path is actually set. Since the
            // browser is seeded from the downloads directory on a fresh
            // amp, `file_list[0]` exists long before anything is loaded,
            // and using it here would name a profile that is not playing.
            let has_model = !app.params.model_path.lock().is_empty();
            let name = if raw_name.is_empty() {
                if has_model && !stem.is_empty() {
                    stem.clone()
                } else {
                    "(no model loaded)".to_string()
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

fn format_khz(hz: f32) -> String {
    let khz = hz / 1000.0;
    if (khz - khz.round()).abs() < 0.05 {
        format!("{:.0} kHz", khz)
    } else {
        format!("{:.1} kHz", khz)
    }
}

fn load_model_clicked(app: &AmpEditorApp) {
    // Sync rfd dialog on the UI thread — the Wayland runtime's editor
    // thread, or the AppKit main thread under the Cocoa runtime, where a
    // modal panel is the supported path and the runtime's reentrancy
    // guard skips nested paints (macos-editor-plan.md §3h).
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
    // With nothing loaded the selector is parked at 0 without that
    // meaning "file 0 is playing", so the first press loads where it
    // already points rather than stepping past it — otherwise the entry
    // the browser is sitting on is the one entry you cannot reach.
    let next = if app.params.model_path.lock().is_empty() {
        current.clamp(0, len_i - 1)
    } else {
        (current + delta).rem_euclid(len_i)
    };
    app.params.file_select.set_value(next);
    app.load_request.store(next, Ordering::Release);
}
