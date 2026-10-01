//! Chrome panels: top brand bar, tab/preset bar, and bottom status bar.
//!
//! These functions are called from [`super::app::DrumsEditorApp::ui`] and
//! paint the non-tab UI furniture surrounding the central body panel.

use std::sync::atomic::Ordering;

use plugin_gui_core::egui;

use resonance_common::registry::InstalledItem;

use crate::kit_loader::KitStatus;
use crate::sample_info;

use super::app::DrumsEditorApp;
use super::{download_panel, kit_browser, theme};

pub(super) fn draw_chrome(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    ui.horizontal_centered(|ui| {
        ui.label(egui::RichText::new("●").color(theme::ACCENT).size(11.0));
        ui.add_space(2.0);
        ui.label(egui::RichText::new("Resonance").color(theme::TEXT_2).size(12.0));
        ui.label(egui::RichText::new("/").color(theme::TEXT_4).size(12.0));
        ui.label(
            egui::RichText::new("Drums")
                .italics()
                .color(theme::TEXT_1)
                .size(15.0),
        );
        ui.add_space(14.0);

        // Presets. The drum kit ships no factory bank, so until ba todo
        // #1332 a kit a user had balanced pad by pad could not be kept at
        // all outside the one project it lived in.
        let refs: Vec<&dyn resonance_plugin::Param> = (0..crate::params::PARAM_COUNT)
            .map(|i| app.params.param_at(i))
            .collect();
        resonance_plugin::preset_ui::preset_bar(
            ui,
            "drums_preset",
            &mut app.preset_editor,
            &app.bank,
            &app.presets,
            &refs,
            "— preset —",
        );

        // Entry points for getting a kit onto disk. These used to be a
        // ghost `Browse` button buried in the pad-list kit card (whose
        // overlay you then couldn't see — §1.2) and a `Load kit` next to
        // it; both are real, clearly labelled actions, so they live in
        // the chrome where the rest of the editor's actions are (ba
        // drums-plugin-rework.md §10, K0).
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Open kit file…").clicked() {
                if let Some(req) = kit_browser::load_kit_clicked(&app.bridge) {
                    app.requested_kit = Some(req);
                }
            }
            ui.add_space(6.0);
            if ui.button("Download kits…").clicked() {
                download_panel::open(&mut app.download_panel, &app.download_worker);
            }
        });
    });
}

/// Tab bar: the KIT pill (◀ name ▶). The editor has one view, Pads, so
/// there is no tab strip to switch it with. It used to carry five tabs,
/// four of which rendered a "not built yet" placeholder, plus a
/// single-option `Pads` segmented control whose click was discarded (ba
/// todo #1327). The `DRUMS` label and the "N lit" PADS badge went too
/// (drums-plugin-rework.md §6.1): decoration, not information — the
/// round-robin readouts that matter are on each pad's row and in the
/// inspector.
pub(super) fn draw_tab_bar(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    let installed = app.installed_kits.clone();
    // Step from the kit on its way if a load is in flight, not from the
    // one it is replacing: `kit_path` is only written once a load
    // succeeds, so two quick ▶ clicks used to land on the same kit.
    let current_idx =
        kit_browser::kit_path_for_stepping(&app.bridge, app.requested_kit.as_ref())
            .and_then(|path| installed_index(&path, &installed));
    let mut pick: Option<usize> = None;

    ui.horizontal_centered(|ui| {
        ui.label(
            egui::RichText::new("KIT")
                .color(theme::TEXT_3)
                .size(10.0)
                .strong(),
        );
        ui.add_space(8.0);

        let pill = egui::Frame::default()
            .fill(theme::BG_2)
            .stroke(egui::Stroke::new(1.0, theme::LINE))
            .corner_radius(7.0)
            .inner_margin(egui::Margin::symmetric(10, 4));
        pill.show(ui, |ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                if pill_arrow(ui, "◀") {
                    pick = step(current_idx, installed.len(), false);
                }

                // When the current kit is one of the installed ones, show
                // the registry's own name for it (§1.4: the loader's
                // status name is the manifest's parent directory —
                // "drummica" — not the registry's "Drummica"). Otherwise
                // fall back to whatever the loader reported, e.g. a kit
                // opened straight from a file outside the library.
                let display = match current_idx {
                    Some(i) => installed[i].name.clone(),
                    None => {
                        let name = current_kit_name(app);
                        if name.is_empty() {
                            "— no kit —".to_string()
                        } else {
                            name
                        }
                    }
                };
                egui::ComboBox::from_id_salt("drums_kit_combo")
                    .width(170.0)
                    .selected_text(
                        egui::RichText::new(display)
                            .color(theme::TEXT_1)
                            .size(12.0),
                    )
                    .show_ui(ui, |ui| {
                        if installed.is_empty() {
                            ui.label(theme::hint_text("(no kits installed)"));
                        }
                        for (idx, item) in installed.iter().enumerate() {
                            if ui
                                .selectable_label(Some(idx) == current_idx, &item.name)
                                .clicked()
                            {
                                pick = Some(idx);
                            }
                        }
                    });

                if pill_arrow(ui, "▶") {
                    pick = step(current_idx, installed.len(), true);
                }
            });
        });
    });

    if let Some(item) = pick.and_then(|i| installed.get(i)) {
        if let Some(req) = kit_browser::load_installed_kit(&app.bridge, item) {
            app.requested_kit = Some(req);
        }
    }
}

/// One of the pill's frameless ◀ / ▶ buttons. True when clicked.
fn pill_arrow(ui: &mut egui::Ui, glyph: &str) -> bool {
    ui.add(
        egui::Button::new(egui::RichText::new(glyph).color(theme::TEXT_3).size(9.0)).frame(false),
    )
    .clicked()
}

/// The installed kit one step from `current` (forward or back), clamped
/// at the ends. With no current kit, either arrow picks the first one.
fn step(current: Option<usize>, len: usize, forward: bool) -> Option<usize> {
    match current {
        Some(i) if forward => (i + 1 < len).then_some(i + 1),
        Some(i) => i.checked_sub(1),
        None => (len > 0).then_some(0),
    }
}

/// Status bar. Every figure here is a measurement published by the audio
/// thread or the kit loader — sample rate and block size from `process`,
/// decoded-sample memory from whoever built the live kit, and the OUT
/// meter from the sampler's per-block peak. Nothing is estimated: the
/// invented CPU / RAM / "Streamed" readouts this bar used to carry were
/// removed rather than guessed at (ba todo #1276).
pub(super) fn draw_status_bar(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    let peak = app.tick_out_meter();
    ui.horizontal_centered(|ui| {
        // Sample rate from bridge; fall back to "—" before activation.
        let sr_bits = app.bridge.sample_rate.load(Ordering::Acquire);
        let sr_text = if sr_bits == 0 {
            "—".to_string()
        } else {
            let hz = f32::from_bits(sr_bits);
            format!("{:.1}", hz / 1000.0)
        };
        mono(ui, &sr_text, "kHz");
        ui.add_space(8.0);

        // Real block size, as last requested by the host.
        let frames = app.bridge.block_frames.load(Ordering::Relaxed);
        let block_text = if frames == 0 {
            "—".to_string()
        } else {
            frames.to_string()
        };
        mono(ui, &block_text, "samples");
        ui.add_space(14.0);

        // Decoded sample memory held by the live kit.
        let bytes = app.bridge.kit_bytes.load(Ordering::Relaxed);
        let kit_text = if bytes == 0 {
            "—".to_string()
        } else {
            sample_info::format_bytes(bytes)
        };
        plain(ui, "SAMPLES", &kit_text);

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(peak_db_text(peak))
                    .color(theme::TEXT_2)
                    .size(10.0)
                    .monospace(),
            );
            ui.add_space(8.0);
            draw_out_meter(ui, peak);
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("OUT")
                    .color(theme::TEXT_3)
                    .size(10.0)
                    .strong(),
            );
        });
    });
}

/// Two stacked bars (left / right) filled from the decayed output peak.
fn draw_out_meter(ui: &mut egui::Ui, peak: [f32; 2]) {
    let bar_w = 80.0;
    let bar_h = 3.0;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(bar_w, bar_h * 2.0 + 2.0), egui::Sense::hover());
    let p = ui.painter_at(rect);
    for (channel, level) in peak.iter().enumerate() {
        let top = rect.left_top() + egui::vec2(0.0, channel as f32 * (bar_h + 2.0));
        p.rect_filled(
            egui::Rect::from_min_size(top, egui::vec2(bar_w, bar_h)),
            1.5,
            theme::BG_3,
        );
        let filled = meter_fraction(*level) * bar_w;
        if filled > 0.0 {
            let color = if *level >= 1.0 { theme::BAD } else { theme::GOOD };
            p.rect_filled(
                egui::Rect::from_min_size(top, egui::vec2(filled, bar_h)),
                1.5,
                color,
            );
        }
    }
    response.on_hover_text("Peak level summed across all 7 output ports.");
}

/// Map a linear peak to bar fill, -60 dBFS .. 0 dBFS.
fn meter_fraction(peak: f32) -> f32 {
    if peak <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * peak.log10();
    ((db + 60.0) / 60.0).clamp(0.0, 1.0)
}

fn peak_db_text(peak: [f32; 2]) -> String {
    let loudest = peak[0].max(peak[1]);
    if loudest <= 0.0 {
        "−∞ dB".to_string()
    } else {
        format!("{:.1} dB", 20.0 * loudest.log10())
    }
}

fn mono(ui: &mut egui::Ui, value: &str, label: &str) {
    ui.label(
        egui::RichText::new(value)
            .color(theme::TEXT_2)
            .size(10.5)
            .monospace(),
    );
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(label)
            .color(theme::TEXT_3)
            .size(10.0),
    );
}

fn plain(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(
        egui::RichText::new(label)
            .color(theme::TEXT_3)
            .size(10.0)
            .strong(),
    );
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(value)
            .color(theme::TEXT_2)
            .size(10.5)
            .monospace(),
    );
}

/// Resolve the kit name currently shown in the kit status, falling back
/// to an empty string if no kit is loaded.
fn current_kit_name(app: &DrumsEditorApp) -> String {
    let status = app.bridge.kit_status.lock();
    match &*status {
        KitStatus::Loaded { name, .. } => name.clone(),
        KitStatus::Loading { path } => path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// Which installed kit, if any, `kit_path` (a manifest) belongs to.
///
/// Matching by name does not work: the loaded name is the manifest's
/// *parent* directory (`kit_loader/mod.rs` — "drummica"), while the
/// registry's name is the kit's top directory ("Drummica"), so the combo
/// never highlighted the loaded kit and ◀/▶ always reloaded the first one
/// (ba drums-plugin-rework.md §1.4). The registry's `path` is always an
/// ancestor of the manifest path, though — downloaded or imported, the
/// manifest lives one or two levels under the top directory — so
/// matching that way is stable regardless of what either side names
/// things.
fn installed_index(kit_path: &std::path::Path, installed: &[InstalledItem]) -> Option<usize> {
    installed
        .iter()
        .position(|item| kit_path.starts_with(&item.path))
}
