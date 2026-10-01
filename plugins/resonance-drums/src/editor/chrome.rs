//! Chrome panels: top brand bar, kit bar, and bottom status bar.
//!
//! These functions are called from [`super::app::DrumsEditorApp::ui`] and
//! paint the non-tab UI furniture surrounding the central body panel.

use std::sync::atomic::Ordering;

use plugin_gui_core::egui;
use resonance_plugin::kit_rows::{step_in_view, view_counter};

use crate::kit_loader::KitStatus;
use crate::sample_info;

use super::app::DrumsEditorApp;
use super::kit_browser::LoadKind;
use super::{probe, theme};

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

        // One entry point for every kit source — installed, plok.org,
        // import (drums-plugin-rework.md §6.1). It replaced "Download
        // kits…" and "Open kit file…". The fleet's solid-accent fill; the
        // label is the darkest surface token because the canonical accent
        // is a dark violet.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let library_btn = egui::Button::new(
                egui::RichText::new("Library…")
                    .color(theme::BG_0)
                    .strong()
                    .size(12.0),
            )
            .fill(theme::ACCENT);
            let b = ui.add(library_btn).on_hover_text(
                "Installed kits, plok.org downloads, import, favourites and tags",
            );
            probe(ui, "header.library", b.rect);
            if b.clicked() {
                app.open_library();
            }
        });
    });
}

/// The kit bar: ☆/★ for the loaded kit, and the kit pill (◀ name ▶). The
/// name is a dropdown of the library view — favourites first, following
/// the Library's search and filters — and ◀/▶ step through that same view
/// (drums-plugin-rework.md §6.1). The editor has one view, Pads, so there
/// is no tab strip to switch it with (ba todo #1327).
pub(super) fn draw_tab_bar(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    app.refresh_rows();
    // The kit on its way if a load is in flight, not the one it replaces:
    // `kit_path` is only written once a load succeeds, so two quick ▶
    // clicks used to land on the same kit.
    let loaded = app.loaded_entry();
    let loaded_id = loaded.as_ref().map(|e| e.id.clone());
    let mut pick: Option<(usize, LoadKind)> = None;
    let mut step: Option<i32> = None;

    ui.horizontal_centered(|ui| {
        ui.label(
            egui::RichText::new("KIT")
                .color(theme::TEXT_3)
                .size(10.0)
                .strong(),
        );
        ui.add_space(8.0);

        // ☆/★ favourites the loaded kit (WARM when set).
        if let Some(id) = loaded_id.as_deref() {
            let fav = app.library.marks_of(id).favorite;
            let star = plugin_gui_core::widgets::star_toggle(ui, fav)
                .on_hover_text("Favourite this kit");
            probe(ui, "kit.star", star.rect);
            if star.clicked() {
                app.toggle_favorite(id);
            }
            ui.add_space(4.0);
        }

        let pill = egui::Frame::default()
            .fill(theme::BG_2)
            .stroke(egui::Stroke::new(1.0, theme::LINE))
            .corner_radius(7.0)
            .inner_margin(egui::Margin::symmetric(10, 4));
        pill.show(ui, |ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let can_step = app.browser.view_len() > 0;
                if pill_arrow(ui, "◀", can_step) {
                    step = Some(-1);
                }

                // The library's name for the kit ("Drummica", or its
                // `_meta.name`), not the manifest's directory. A kit loaded
                // from outside the library falls back to the loader's name.
                let display = match &loaded {
                    Some(e) => e.name.clone(),
                    None => {
                        let name = current_kit_name(app);
                        if name.is_empty() {
                            "— no kit —".to_string()
                        } else {
                            name
                        }
                    }
                };
                let loaded_key = loaded.as_ref().map(|e| e.mark_key());
                let combo = egui::ComboBox::from_id_salt("drums_kit_combo")
                    .width(170.0)
                    .selected_text(
                        egui::RichText::new(display)
                            .color(theme::TEXT_1)
                            .size(12.0),
                    )
                    .show_ui(ui, |ui| {
                        let view = app.browser.view().to_vec();
                        if view.is_empty() {
                            ui.label(theme::hint_text(if app.rows.rows.is_empty() {
                                "(no kits installed — open the Library)"
                            } else {
                                "(no kit matches the Library's filters)"
                            }));
                        }
                        for row in view {
                            let r = &app.rows.rows[row];
                            let fav = app.rows.marks_of(row).is_some_and(|m| m.favorite);
                            let text = format!("{} {}", if fav { "★" } else { "☆" }, r.entry.name);
                            let selected = loaded_key.as_deref() == Some(r.key.as_str());
                            if ui
                                .add_enabled(
                                    r.entry.is_loadable(),
                                    egui::Button::selectable(selected, text),
                                )
                                .clicked()
                            {
                                pick = Some((row, LoadKind::Pick));
                            }
                        }
                    });
                probe(ui, "kit.combo", combo.response.rect);

                if pill_arrow(ui, "▶", can_step) {
                    step = Some(1);
                }
            });
        });
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(view_counter(&app.browser, loaded_id.as_deref()))
                .size(10.5)
                .color(theme::TEXT_3),
        );
    });

    if let Some(delta) = step {
        let slot = step_in_view(&app.browser, &app.rows, loaded_id.as_deref(), delta);
        if let Some(row) = slot.and_then(|s| {
            app.rows
                .rows
                .iter()
                .position(|r| r.entry.slot == Some(s))
        }) {
            // Browsing, not a pick: no Recent bump.
            pick = Some((row, LoadKind::Browse));
        }
    }
    if let Some((row, kind)) = pick {
        let entry = app.rows.rows[row].entry.clone();
        app.load_entry(&entry, kind);
    }
}

/// One of the pill's frameless ◀ / ▶ buttons. True when clicked.
fn pill_arrow(ui: &mut egui::Ui, glyph: &str, enabled: bool) -> bool {
    ui.add_enabled(
        enabled,
        egui::Button::new(egui::RichText::new(glyph).color(theme::TEXT_3).size(9.0)).frame(false),
    )
    .clicked()
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

