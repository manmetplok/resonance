//! Chrome panels: top brand bar, tab/preset bar, and bottom status bar.
//!
//! These functions are called from [`super::app::DrumsEditorApp::ui`] and
//! paint the non-tab UI furniture surrounding the central body panel.

use std::sync::atomic::Ordering;

use plugin_gui_core::egui;

use resonance_common::registry::InstalledItem;

use crate::kit_loader::KitStatus;
use crate::rr_display;
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
                kit_browser::load_kit_clicked(&app.bridge);
            }
            ui.add_space(6.0);
            if ui.button("Download kits…").clicked() {
                download_panel::open(&mut app.download_panel, &app.download_worker);
            }
        });
    });
}

/// Tab bar. The editor has exactly one view (Pads), so there is no tab
/// strip here to switch it with — the "DRUMS" label plus the PADS badge
/// and KIT pill are what remains. It used to carry five tabs, four of
/// which rendered a "not built yet" placeholder, plus a single-option
/// `Pads` segmented control whose click was discarded because it was the
/// only option: a control the audit found drawn and interactive while
/// doing nothing, the exact defect `editor_honesty.rs` guards against
/// elsewhere (ba todo #1327).
pub(super) fn draw_tab_bar(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    ui.horizontal_centered(|ui| {
        ui.label(
            egui::RichText::new("DRUMS")
                .color(theme::TEXT_3)
                .size(10.5)
                .strong(),
        );

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // PADS badge — total pads, how many have fired, and how many
            // of those actually have takes to cycle. The per-pad "take N
            // of M" readouts live in the pad list and the inspector.
            let total = app.bridge.last_rr.len();
            let fired: Vec<_> = app
                .bridge
                .last_rr
                .iter()
                .filter_map(|a| rr_display::unpack(a.load(Ordering::Relaxed)))
                .collect();
            let cycling = fired.iter().filter(|rr| rr.cycles()).count();
            let badge_text = format!("{} · {} lit", total, fired.len());
            draw_pads_badge(ui, &badge_text)
                .on_hover_text(format!(
                    "{} of {} pads have played; {} of those cycle through \
                     multiple round-robin takes.",
                    fired.len(),
                    total,
                    cycling,
                ));

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
                        let current_idx = current_installed_index(&app.bridge, &installed);

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

                        // When the loaded kit is one of the installed
                        // ones, show the registry's own name for it
                        // (§1.4: the loader's status name is the
                        // manifest's parent directory — "drummica" — not
                        // the registry's "Drummica"). Otherwise fall back
                        // to whatever the loader reported, e.g. a kit
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
                                        .selectable_label(
                                            Some(idx) == current_idx,
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

/// Which installed kit, if any, is the one loaded in `bridge`.
///
/// Matching by name does not work: the loaded name is the manifest's
/// *parent* directory (`kit_loader/mod.rs` — "drummica"), while the
/// registry's name is the kit's top directory ("Drummica"), so the combo
/// never highlighted the loaded kit and ◀/▶ always reloaded the first one
/// (ba drums-plugin-rework.md §1.4). The registry's `path` is always an
/// ancestor of the loaded manifest path, though — downloaded or imported,
/// the manifest lives one or two levels under the top directory — so
/// matching that way is stable regardless of what either side names
/// things.
fn current_installed_index(
    bridge: &crate::KitBridge,
    installed: &[InstalledItem],
) -> Option<usize> {
    let kit_path = bridge.kit_path.lock().clone()?;
    installed
        .iter()
        .position(|item| kit_path.starts_with(&item.path))
}

/// Draw the lavender PADS badge: `PADS  30 · 6 lit`.
fn draw_pads_badge(ui: &mut egui::Ui, count_text: &str) -> egui::Response {
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
    let (rect, response) = ui.allocate_exact_size(total, egui::Sense::hover());

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
    response
}
