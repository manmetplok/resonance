//! Chrome panels: top brand bar, tab/preset bar, and bottom status bar.
//!
//! These functions are called from [`super::app::WavetableEditorApp::ui`]
//! and paint the non-tab UI furniture surrounding the central tab body.

use wayland_plugin_gui::egui;

use crate::presets::PRESETS;

use super::app::{load_preset, peak_of, WavetableEditorApp, WtTab};
use super::{theme, widgets};

pub(super) fn draw_chrome(ui: &mut egui::Ui, _app: &mut WavetableEditorApp) {
    ui.horizontal_centered(|ui| {
        // Traffic-light dots (decorative).
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
            egui::RichText::new("Wavetable")
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

pub(super) fn draw_tab_bar(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    ui.horizontal_centered(|ui| {
        ui.label(
            egui::RichText::new("WAVETABLE")
                .color(theme::TEXT_3)
                .size(10.5)
                .strong(),
        );
        ui.add_space(8.0);

        let labels = ["Osc", "Env · Filter", "LFO", "Mod", "FX"];
        let selected_idx = match app.selected_tab {
            WtTab::Osc => 0,
            WtTab::EnvFilter => 1,
            WtTab::Lfo => 2,
            WtTab::Mod => 3,
            WtTab::Fx => 4,
        };
        if let Some(i) = widgets::segmented(ui, &labels, selected_idx, false) {
            app.selected_tab = match i {
                0 => WtTab::Osc,
                1 => WtTab::EnvFilter,
                2 => WtTab::Lfo,
                3 => WtTab::Mod,
                _ => WtTab::Fx,
            };
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Voices badge — paint manually so the right-to-left parent
            // doesn't expand the frame to fill the remaining width.
            let voices = app.snapshot.active_voice_count;
            let max_voices = app.params.max_voices.value();
            let badge_text = format!("{} / {}", voices, max_voices);
            draw_voices_badge(ui, &badge_text);

            ui.add_space(8.0);

            // Preset pill — explicit left-to-right inner layout so arrows
            // stay in the natural order (◀ name ▶).
            let pill = egui::Frame::default()
                .fill(theme::BG_2)
                .stroke(egui::Stroke::new(1.0, theme::LINE))
                .corner_radius(7.0)
                .inner_margin(egui::Margin::symmetric(10, 4));
            pill.show(ui, |ui| {
                ui.with_layout(
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
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
                            app.preset_idx = app.preset_idx.saturating_sub(1);
                            if let Some(entry) = PRESETS.get(app.preset_idx) {
                                load_preset(&app.params, entry.json);
                            }
                        }
                        let preset_name = PRESETS
                            .get(app.preset_idx)
                            .map(|p| p.name)
                            .unwrap_or("— select —");
                        egui::ComboBox::from_id_salt("wt_preset_combo")
                            .width(170.0)
                            .selected_text(
                                egui::RichText::new(preset_name)
                                    .color(theme::TEXT_1)
                                    .size(12.0),
                            )
                            .show_ui(ui, |ui| {
                                for (i, entry) in PRESETS.iter().enumerate() {
                                    if ui.selectable_label(false, entry.name).clicked() {
                                        app.preset_idx = i;
                                        load_preset(&app.params, entry.json);
                                    }
                                }
                            });
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
                            let next = app.preset_idx + 1;
                            if next < PRESETS.len() {
                                app.preset_idx = next;
                                if let Some(entry) = PRESETS.get(app.preset_idx) {
                                    load_preset(&app.params, entry.json);
                                }
                            }
                        }
                    },
                );
            });

            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("PRESET")
                    .color(theme::TEXT_3)
                    .size(10.0)
                    .strong(),
            );
        });
    });
}

pub(super) fn draw_status_bar(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    ui.horizontal_centered(|ui| {
        let mono = |ui: &mut egui::Ui, label: &str, value: &str| {
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
            ui.add_space(14.0);
        };

        mono(ui, "SR", "48000 Hz");
        mono(ui, "BUF", "256");
        let voices = app.snapshot.active_voice_count;
        mono(ui, "VOICES", &format!("{}", voices));

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Output meter (placeholder — peaks from scope buffer).
            let peak = peak_of(&app.snapshot.scope_samples);
            let bar_w = 80.0;
            let bar_h = 4.0;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(bar_w, bar_h), egui::Sense::hover());
            ui.painter().rect_filled(rect, 1.5, theme::BG_3);
            let fill_w = (peak * bar_w).clamp(0.0, bar_w);
            let fill = egui::Rect::from_min_size(rect.left_top(), egui::vec2(fill_w, bar_h));
            let color = if peak < 0.7 {
                theme::GOOD
            } else if peak < 0.95 {
                theme::WARM
            } else {
                theme::BAD
            };
            ui.painter().rect_filled(fill, 1.5, color);
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("OUT")
                    .color(theme::TEXT_3)
                    .size(10.0)
                    .strong(),
            );
        });
    });
}

fn draw_voices_badge(ui: &mut egui::Ui, count_text: &str) {
    let label = "VOICES";
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
