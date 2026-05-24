//! Left-panel pad list with kit card, search filter, and grouped pad rows.
//!
//! Per-row layout (left to right):
//!   • Status LED (purple/green/dim)
//!   • Pad name
//!   • MIDI note badge
//!   • Mute "M" button

use std::sync::atomic::Ordering;

use wayland_plugin_gui::egui;

use resonance_plugin::param::Param;

use crate::download::WorkerHandle;
use crate::drum_map::{NUM_PADS, PAD_MAPPINGS};
use crate::kit::OutputGroup;
use crate::kit_loader::KitStatus;
use crate::params::DrumParams;
use crate::KitBridge;

use super::download_panel::DownloadPanelState;
use super::{kit_browser, theme};

/// Group label used in the pad list. Derived from `OutputGroup` so adding
/// a new pad type to the map automatically falls into the right section.
fn group_label(g: OutputGroup) -> &'static str {
    match g {
        OutputGroup::Kick => "KICK",
        OutputGroup::Snare => "SNARE",
        OutputGroup::Hats => "HI-HAT",
        OutputGroup::Toms => "TOMS",
        OutputGroup::Cymbals => "CYMBALS",
        OutputGroup::Main => "PERC",
    }
}

/// Render the left-panel pad list. Returns the new selected pad index if
/// the user clicked a row.
pub fn draw(
    ui: &mut egui::Ui,
    params: &DrumParams,
    bridge: &KitBridge,
    download_panel: &mut DownloadPanelState,
    pad_filter: &mut String,
    selected_pad: &mut usize,
    _download_worker: &WorkerHandle,
) {
    let panel = egui::Frame::default()
        .fill(theme::BG_2)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(theme::RADIUS_PANEL)
        .inner_margin(egui::Margin::same(12));

    panel.show(ui, |ui| {
        ui.set_min_width(296.0);
        ui.set_max_width(296.0);
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 10.0);

        // PADS header.
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("PADS")
                    .color(theme::TEXT_3)
                    .size(10.5)
                    .strong(),
            );
        });

        // Kit card.
        draw_kit_card(ui, bridge, download_panel);

        // Search input.
        draw_search(ui, pad_filter);

        // Pad list — scrollable.
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 1.0);
        egui::ScrollArea::vertical()
            .id_salt("pad_list_scroll")
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                draw_pad_list(ui, params, bridge, pad_filter, selected_pad);
            });
    });
}

fn draw_kit_card(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    download_panel: &mut DownloadPanelState,
) {
    let frame = egui::Frame::default()
        .fill(theme::BG_1)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(12, 10));

    frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            // Thumbnail (D monogram).
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover());
            let p = ui.painter_at(rect);
            p.rect_filled(rect, 7.0, theme::BG_2);
            p.rect_stroke(
                rect,
                7.0,
                egui::Stroke::new(1.0, theme::LINE),
                egui::StrokeKind::Inside,
            );
            // Faux gradient: a small accent dot top-left and warm dot bottom-right.
            p.circle_filled(
                rect.left_top() + egui::vec2(12.0, 10.0),
                8.0,
                theme::ACCENT_DIM,
            );
            // First-letter monogram.
            let name = current_kit_name(bridge);
            let letter = name.chars().next().unwrap_or('D').to_string();
            p.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                letter.to_uppercase(),
                egui::FontId::proportional(20.0),
                theme::ACCENT_SOFT,
            );

            ui.add_space(10.0);

            // Name + meta.
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 1.0);
                let display_name = if name.is_empty() {
                    "no kit".to_string()
                } else {
                    name.clone()
                };
                ui.label(
                    egui::RichText::new(display_name)
                        .italics()
                        .color(theme::TEXT_1)
                        .size(13.5),
                );
                let meta = format_kit_meta(bridge);
                ui.label(
                    egui::RichText::new(meta)
                        .color(theme::TEXT_3)
                        .size(10.0)
                        .monospace(),
                );
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 4.0);
                    let ghost = |ui: &mut egui::Ui, label: &str| -> egui::Response {
                        let btn = egui::Button::new(
                            egui::RichText::new(label)
                                .color(theme::TEXT_2)
                                .size(10.5),
                        )
                        .fill(egui::Color32::TRANSPARENT)
                        .stroke(egui::Stroke::new(1.0, theme::LINE))
                        .corner_radius(6.0)
                        .min_size(egui::vec2(64.0, 22.0));
                        ui.add(btn)
                    };
                    if ghost(ui, "Browse").clicked() {
                        download_panel.open = true;
                    }
                    if ghost(ui, "Load kit").clicked() {
                        kit_browser::load_kit_clicked(bridge);
                    }
                });
            });
        });
    });
}

fn draw_search(ui: &mut egui::Ui, pad_filter: &mut String) {
    let frame = egui::Frame::default()
        .fill(theme::BG_1)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(10, 4));
    frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("🔍")
                    .color(theme::TEXT_3)
                    .size(11.0),
            );
            ui.add_space(2.0);
            let edit = egui::TextEdit::singleline(pad_filter)
                .hint_text(
                    egui::RichText::new("Filter pads…")
                        .color(theme::TEXT_4)
                        .size(11.5),
                )
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .text_color(theme::TEXT_1)
                .font(egui::TextStyle::Body);
            ui.add(edit);
        });
    });
}

fn draw_pad_list(
    ui: &mut egui::Ui,
    params: &DrumParams,
    bridge: &KitBridge,
    pad_filter: &str,
    selected_pad: &mut usize,
) {
    let filter = pad_filter.trim().to_lowercase();
    let matches = |name: &str| -> bool {
        if filter.is_empty() {
            return true;
        }
        name.to_lowercase().contains(&filter)
    };

    // Stable iteration order: group by OutputGroup, then by source order.
    let groups: [OutputGroup; 6] = [
        OutputGroup::Kick,
        OutputGroup::Snare,
        OutputGroup::Hats,
        OutputGroup::Toms,
        OutputGroup::Cymbals,
        OutputGroup::Main,
    ];

    let mut shown_in_group = 0usize;
    for group in groups.iter() {
        let group_idx: Vec<usize> = (0..NUM_PADS)
            .filter(|&i| {
                PAD_MAPPINGS[i].output_group == *group && matches(PAD_MAPPINGS[i].name)
            })
            .collect();
        if group_idx.is_empty() {
            continue;
        }

        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(group_label(*group))
                .color(theme::TEXT_4)
                .size(9.5)
                .strong(),
        );

        for i in group_idx {
            let mapping = &PAD_MAPPINGS[i];
            let selected = *selected_pad == i;
            let has_sound = bridge.last_rr[i].load(Ordering::Relaxed) != 0;
            draw_pad_row(ui, params, mapping, i, selected, has_sound, |idx| {
                *selected_pad = idx;
            });
            shown_in_group += 1;
        }
    }

    if shown_in_group == 0 {
        ui.add_space(8.0);
        ui.label(theme::hint_text("No matching pads."));
    }
}

fn draw_pad_row(
    ui: &mut egui::Ui,
    params: &DrumParams,
    mapping: &crate::drum_map::PadMapping,
    pad_idx: usize,
    selected: bool,
    has_sound: bool,
    mut on_select: impl FnMut(usize),
) {
    let row_h = 22.0;
    let avail_w = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(avail_w, row_h),
        egui::Sense::click(),
    );

    // Background pill.
    let p = ui.painter_at(rect);
    if selected {
        p.rect_filled(rect, 4.0, theme::ACCENT_DIM);
    } else if response.hovered() {
        p.rect_filled(rect, 4.0, theme::BG_1);
    }

    // LED.
    let led_x = rect.left() + 8.0;
    let led_y = rect.center().y;
    let led_color = if selected {
        theme::ACCENT
    } else if has_sound {
        theme::GOOD
    } else {
        theme::TEXT_4
    };
    p.circle_filled(egui::pos2(led_x, led_y), 3.0, led_color);

    // Name.
    let name_x = led_x + 12.0;
    let name_color = if selected { theme::TEXT_1 } else { theme::TEXT_2 };
    p.text(
        egui::pos2(name_x, led_y),
        egui::Align2::LEFT_CENTER,
        mapping.name,
        egui::FontId::proportional(11.5),
        name_color,
    );

    // MIDI note badge (right side, before mute button).
    let mute = params.pads[pad_idx].mute.value();
    let mute_size = 16.0;
    let mute_x = rect.right() - 6.0 - mute_size;
    let badge_h = 14.0;
    let badge_text = format!("{}", mapping.note);
    let badge_font = egui::FontId::monospace(10.0);
    let badge_w = ui
        .painter()
        .layout_no_wrap(badge_text.clone(), badge_font.clone(), theme::TEXT_3)
        .size()
        .x
        + 10.0;
    let badge_rect = egui::Rect::from_center_size(
        egui::pos2(
            mute_x - 6.0 - badge_w * 0.5,
            led_y,
        ),
        egui::vec2(badge_w, badge_h),
    );
    let (badge_bg, badge_stroke, badge_fg) = if selected {
        (theme::ACCENT_DIM, theme::ACCENT, theme::ACCENT_SOFT)
    } else {
        (theme::BG_1, theme::LINE_2, theme::TEXT_3)
    };
    p.rect_filled(badge_rect, 3.0, badge_bg);
    p.rect_stroke(
        badge_rect,
        3.0,
        egui::Stroke::new(1.0, badge_stroke),
        egui::StrokeKind::Inside,
    );
    p.text(
        badge_rect.center(),
        egui::Align2::CENTER_CENTER,
        &badge_text,
        badge_font,
        badge_fg,
    );

    // Mute button (paints itself, then we allocate a small response on
    // top so clicks toggle).
    let mute_rect = egui::Rect::from_center_size(
        egui::pos2(mute_x + mute_size * 0.5, led_y),
        egui::vec2(mute_size, mute_size),
    );
    let mute_resp = ui.interact(
        mute_rect,
        ui.id().with(("mute", pad_idx)),
        egui::Sense::click(),
    );
    let (m_bg, m_fg, m_stroke) = if mute {
        (theme::BAD, egui::Color32::from_rgb(0x1a, 0x0e, 0x10), theme::BAD)
    } else if mute_resp.hovered() {
        (theme::BG_3, theme::TEXT_2, theme::LINE)
    } else {
        (egui::Color32::TRANSPARENT, theme::TEXT_4, theme::LINE_2)
    };
    p.rect_filled(mute_rect, 3.0, m_bg);
    p.rect_stroke(
        mute_rect,
        3.0,
        egui::Stroke::new(1.0, m_stroke),
        egui::StrokeKind::Inside,
    );
    p.text(
        mute_rect.center(),
        egui::Align2::CENTER_CENTER,
        "M",
        egui::FontId::proportional(8.5),
        m_fg,
    );
    if mute_resp.clicked() {
        params.pads[pad_idx]
            .mute
            .set_plain(if mute { 0.0 } else { 1.0 });
    } else if response.clicked() && !mute_resp.hovered() {
        on_select(pad_idx);
    } else if response.clicked() {
        // Clicked on mute area — leave selection unchanged.
    }
}

fn current_kit_name(bridge: &KitBridge) -> String {
    let status = bridge.kit_status.lock();
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

fn format_kit_meta(bridge: &KitBridge) -> String {
    let status = bridge.kit_status.lock();
    match &*status {
        KitStatus::Loaded { num_pads, .. } => format!("{} pads", num_pads),
        KitStatus::Loading { .. } => "loading…".to_string(),
        KitStatus::Error { .. } => "error".to_string(),
        KitStatus::Empty => "defaults".to_string(),
    }
}
