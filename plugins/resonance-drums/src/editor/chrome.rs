//! Chrome panels: top brand bar, tab/preset bar, and bottom status bar.
//!
//! These functions are called from [`super::app::DrumsEditorApp::ui`] and
//! paint the non-tab UI furniture surrounding the central body panel.

use std::sync::atomic::Ordering;

use wayland_plugin_gui::egui;

use crate::kit_loader::KitStatus;

use super::app::{DrumsEditorApp, DrumsTab};
use super::{kit_browser, theme, widgets};

pub(super) fn draw_chrome(ui: &mut egui::Ui, _app: &mut DrumsEditorApp) {
    ui.horizontal_centered(|ui| {
        let dot = |ui: &mut egui::Ui, color: egui::Color32| {
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 5.0, color);
        };
        dot(ui, egui::Color32::from_rgb(0xed, 0x6b, 0x5e));
        ui.add_space(4.0);
        dot(ui, egui::Color32::from_rgb(0xf4, 0xbe, 0x4f));
        ui.add_space(4.0);
        dot(ui, egui::Color32::from_rgb(0x61, 0xc4, 0x54));

        ui.add_space(14.0);
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
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new("⚙").color(theme::TEXT_3).size(13.0));
            ui.add_space(10.0);
            ui.label(egui::RichText::new("A").color(theme::TEXT_3).size(12.0));
            ui.add_space(10.0);
            ui.label(egui::RichText::new("?").color(theme::TEXT_3).size(13.0));
        });
    });
}

pub(super) fn draw_tab_bar(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    ui.horizontal_centered(|ui| {
        ui.label(
            egui::RichText::new("DRUMS")
                .color(theme::TEXT_3)
                .size(10.5)
                .strong(),
        );
        ui.add_space(8.0);

        let labels = ["Pads", "Mics", "Articulations", "Mod", "FX"];
        let selected_idx = match app.selected_tab {
            DrumsTab::Pads => 0,
            DrumsTab::Mics => 1,
            DrumsTab::Articulations => 2,
            DrumsTab::Mod => 3,
            DrumsTab::Fx => 4,
        };
        if let Some(i) = widgets::segmented(ui, &labels, selected_idx, false) {
            app.selected_tab = match i {
                0 => DrumsTab::Pads,
                1 => DrumsTab::Mics,
                2 => DrumsTab::Articulations,
                3 => DrumsTab::Mod,
                _ => DrumsTab::Fx,
            };
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // PADS voice-count badge — show total pads and how many were
            // last triggered (non-zero last_rr).
            let total = app.bridge.last_rr.len();
            let lit = app
                .bridge
                .last_rr
                .iter()
                .filter(|a| a.load(Ordering::Relaxed) != 0)
                .count();
            let badge_text = format!("{} · {} lit", total, lit);
            draw_pads_badge(ui, &badge_text);

            ui.add_space(8.0);

            // KIT preset pill — driven by installed kits.
            let pill = egui::Frame::default()
                .fill(theme::BG_2)
                .stroke(egui::Stroke::new(1.0, theme::LINE))
                .corner_radius(7.0)
                .inner_margin(egui::Margin::symmetric(10, 4));
            pill.show(ui, |ui| {
                ui.with_layout(
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        let installed = app.installed_kits.clone();
                        let current_name = current_kit_name(app);

                        // Resolve currently selected installed-kit index, if any.
                        let current_idx = installed
                            .iter()
                            .position(|i| i.name == current_name);

                        // Prev arrow.
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("◀")
                                        .color(theme::TEXT_3)
                                        .size(9.0),
                                )
                                .frame(false),
                            )
                            .clicked()
                        {
                            if let Some(idx) = current_idx {
                                if idx > 0 {
                                    let item = installed[idx - 1].clone();
                                    kit_browser::load_installed_kit(&app.bridge, &item);
                                }
                            } else if let Some(item) = installed.first() {
                                kit_browser::load_installed_kit(&app.bridge, item);
                            }
                        }

                        let display = if current_name.is_empty() {
                            "— no kit —".to_string()
                        } else {
                            current_name.clone()
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
                                for item in &installed {
                                    if ui
                                        .selectable_label(
                                            item.name == current_name,
                                            &item.name,
                                        )
                                        .clicked()
                                    {
                                        kit_browser::load_installed_kit(&app.bridge, item);
                                    }
                                }
                            });

                        // Next arrow.
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("▶")
                                        .color(theme::TEXT_3)
                                        .size(9.0),
                                )
                                .frame(false),
                            )
                            .clicked()
                        {
                            if let Some(idx) = current_idx {
                                if idx + 1 < installed.len() {
                                    let item = installed[idx + 1].clone();
                                    kit_browser::load_installed_kit(&app.bridge, &item);
                                }
                            } else if let Some(item) = installed.first() {
                                kit_browser::load_installed_kit(&app.bridge, item);
                            }
                        }
                    },
                );
            });

            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("KIT")
                    .color(theme::TEXT_3)
                    .size(10.0)
                    .strong(),
            );
        });
    });
}

pub(super) fn draw_status_bar(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
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
        mono(ui, "128", "samples");
        ui.add_space(14.0);

        // CPU / RAM / Streamed — no real measurements, render placeholders.
        plain(ui, "CPU", "3.1%");
        ui.add_space(14.0);
        plain(ui, "RAM", "214 MB");
        ui.add_space(14.0);
        plain(ui, "Streamed", "0 samples");

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new("−∞ dB")
                    .color(theme::TEXT_3)
                    .size(10.0)
                    .monospace(),
            );
            ui.add_space(8.0);
            let bar_w = 80.0;
            let bar_h = 3.0;
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(bar_w, bar_h * 2.0 + 2.0), egui::Sense::hover());
            let p = ui.painter_at(rect);
            p.rect_filled(
                egui::Rect::from_min_size(rect.left_top(), egui::vec2(bar_w, bar_h)),
                1.5,
                theme::BG_3,
            );
            p.rect_filled(
                egui::Rect::from_min_size(
                    rect.left_top() + egui::vec2(0.0, bar_h + 2.0),
                    egui::vec2(bar_w, bar_h),
                ),
                1.5,
                theme::BG_3,
            );
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

/// Draw the lavender PADS badge: `PADS  30 · 6 lit`.
fn draw_pads_badge(ui: &mut egui::Ui, count_text: &str) {
    let label = "PADS";
    let pad_x = 10.0;
    let gap = 6.0;
    let label_font = egui::FontId::proportional(10.0);
    let count_font = egui::FontId::monospace(11.0);

    let label_w = ui
        .painter()
        .layout_no_wrap(label.to_owned(), label_font.clone(), theme::ACCENT_SOFT)
        .size()
        .x;
    let count_w = ui
        .painter()
        .layout_no_wrap(
            count_text.to_owned(),
            count_font.clone(),
            theme::ACCENT_SOFT,
        )
        .size()
        .x;

    let inner_w = label_w + gap + count_w;
    let total = egui::vec2(inner_w + pad_x * 2.0, 22.0);
    let (rect, _) = ui.allocate_exact_size(total, egui::Sense::hover());

    let p = ui.painter_at(rect.expand(2.0));
    p.rect_filled(rect, 11.0, theme::ACCENT_DIM);
    p.rect_stroke(
        rect,
        11.0,
        egui::Stroke::new(1.0, theme::ACCENT),
        egui::StrokeKind::Inside,
    );
    let label_x = rect.left() + pad_x;
    let count_x = rect.right() - pad_x;
    let cy = rect.center().y;
    p.text(
        egui::pos2(label_x, cy),
        egui::Align2::LEFT_CENTER,
        label,
        label_font,
        theme::ACCENT_SOFT,
    );
    p.text(
        egui::pos2(count_x, cy),
        egui::Align2::RIGHT_CENTER,
        count_text,
        count_font,
        theme::ACCENT_SOFT,
    );
}
