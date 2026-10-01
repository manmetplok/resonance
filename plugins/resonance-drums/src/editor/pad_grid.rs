//! Left-panel pad list with kit card, search filter, and grouped pad rows.
//!
//! Per-row layout (left to right):
//!   • Status LED (purple/green/dim)
//!   • Pad name — the kit's (`_meta.pieces`, [`crate::pad_map`]); a pad
//!     whose piece the kit lacks is dimmed (D7), and it is silent
//!   • Round-robin readout ("2/3" — take that last fired, of how many)
//!   • MIDI note badge
//!   • Mute "M" button

use std::sync::atomic::Ordering;

use plugin_gui_core::egui;

use resonance_plugin::param::Param;

use crate::drum_map::{NUM_PADS, PAD_MAPPINGS};
use crate::kit::OutputGroup;
use crate::kit_loader::KitStatus;
use crate::params::DrumParams;
use crate::rr_display;
use crate::KitBridge;

use super::theme;

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
    pad_filter: &mut String,
    selected_pad: &mut usize,
) {
    let panel = egui::Frame::default()
        .fill(theme::BG_2)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(theme::RADIUS_PANEL)
        .inner_margin(egui::Margin::same(12));

    panel.show(ui, |ui| {
        // No forced width here: the column is already sized by the
        // `ui.allocate_ui` the caller wraps this panel in (`app.rs`). A
        // fixed 296px used to be asked for regardless, which overflowed
        // the column at narrower window widths (ba drums-plugin-rework.md
        // §1.3).
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
        draw_kit_card(ui, bridge);

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

fn draw_kit_card(ui: &mut egui::Ui, bridge: &KitBridge) {
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
            // The ghost Browse button (opened the download overlay) and
            // the Load kit button next to it used to live here, doing
            // nothing visible until found. Both moved to the header,
            // labelled for what they do (chrome.rs: Download kits… /
            // Open kit file…).
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
    let kit = bridge.kit_pads.current();
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
            .filter(|&i| PAD_MAPPINGS[i].output_group == *group && matches(&kit.pads[i].name))
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
            let rr = rr_display::unpack(bridge.last_rr[i].load(Ordering::Relaxed));
            let kit_pad = &kit.pads[i];
            let row = PadRow {
                name: &kit_pad.name,
                present: kit_pad.present,
                selected,
                rr,
            };
            draw_pad_row(ui, params, mapping, i, row, |idx| {
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

/// What one pad row shows besides its slot.
struct PadRow<'a> {
    /// The kit's name for the pad.
    name: &'a str,
    /// False when the kit lacks the pad's piece: the row is dimmed.
    present: bool,
    selected: bool,
    /// The round-robin state the audio thread last published for this
    /// pad: `None` until the pad fires, then which take of how many,
    /// shown as a compact `2/3` before the note badge (ba todo #1329 —
    /// the row used to reduce it to "has fired at all").
    rr: Option<rr_display::RoundRobin>,
}

/// One pad row.
fn draw_pad_row(
    ui: &mut egui::Ui,
    params: &DrumParams,
    mapping: &crate::drum_map::PadMapping,
    pad_idx: usize,
    row: PadRow<'_>,
    mut on_select: impl FnMut(usize),
) {
    let PadRow {
        name,
        present,
        selected,
        rr,
    } = row;
    let row_h = 22.0;
    let avail_w = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(avail_w, row_h),
        egui::Sense::click(),
    );
    super::probe(ui, format!("pad_row.{pad_idx}"), rect);
    if !present {
        // Dimmed: the kit has no recording for this pad (D7).
        super::probe(ui, format!("pad_row.{pad_idx}.absent"), rect);
    }

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
    } else if !present {
        theme::BG_3
    } else if rr.is_some() {
        theme::GOOD
    } else {
        theme::TEXT_4
    };
    p.circle_filled(egui::pos2(led_x, led_y), 3.0, led_color);

    // Name.
    let name_x = led_x + 12.0;
    let name_color = match (present, selected) {
        (false, _) => theme::TEXT_4,
        (true, true) => theme::TEXT_1,
        (true, false) => theme::TEXT_2,
    };
    p.text(
        egui::pos2(name_x, led_y),
        egui::Align2::LEFT_CENTER,
        name,
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
    // Round-robin readout: which take last fired, of how many.
    if let Some(rr) = rr {
        let rr_font = egui::FontId::monospace(9.5);
        let rr_color = if rr.cycles() {
            theme::ACCENT_SOFT
        } else {
            theme::TEXT_4
        };
        p.text(
            egui::pos2(badge_rect.left() - 6.0, led_y),
            egui::Align2::RIGHT_CENTER,
            rr.compact(),
            rr_font,
            rr_color,
        );
    }

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
    let status = bridge.kit_status.lock().clone();
    match status {
        // The pads the kit fills, not the slots: a kit without toms
        // plays fewer (D7).
        KitStatus::Loaded { num_pads, .. } => {
            let kit = bridge.kit_pads.current();
            let present = if kit.from_kit {
                kit.pads.iter().filter(|pad| pad.present).count()
            } else {
                num_pads
            };
            format!("{present} of {num_pads} pads")
        }
        KitStatus::Loading { .. } => "loading…".to_string(),
        KitStatus::Error { .. } => "error".to_string(),
        KitStatus::Empty => "defaults".to_string(),
    }
}
