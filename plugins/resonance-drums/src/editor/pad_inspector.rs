//! Right-panel per-pad detail view.
//!
//! Layout, top-to-bottom:
//!   • Pad title (Instrument Serif italic) + meta + Audition + Enabled chip
//!   • Sample stage (waveform of the take this pad plays at full velocity)
//!   • 4-knob row: Volume / Pan / OH Trim / Mic 2 Trim (levels in dB)
//!   • Articulations chips, labelled by the kit (only when the kit pairs
//!     the pad's piece with an alternate one, [`crate::pad_map`])
//!   • Close mics card (mic pickers + a dB trim per close mic) + Overhead card
//!
//! Wiring follows the existing param surface — no new params are introduced.
//! For pads without two close mics, the Mic 2 Trim knob renders as a dim
//! placeholder so the knob grid stays a consistent 4-cell row.

use std::sync::atomic::Ordering;

use plugin_gui_core::{egui, widgets};

use resonance_plugin::param::Param;

use crate::drum_map::PAD_MAPPINGS;
use crate::mic_catalog::ManifestMicCatalog;
use crate::params::{DrumParams, MicSlot};
use crate::rr_display;
use crate::sample_info::PadSampleInfo;
use crate::KitBridge;

use super::{reload_kit, theme};

const PANEL_RADIUS: f32 = theme::RADIUS_PANEL;

/// Render the per-pad detail view inside the right-hand column.
pub fn draw(
    ui: &mut egui::Ui,
    params: &DrumParams,
    bridge: &KitBridge,
    catalog: &ManifestMicCatalog,
    selected_pad: usize,
) {
    let frame = egui::Frame::default()
        .fill(theme::BG_2)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(PANEL_RADIUS)
        .inner_margin(egui::Margin::same(14));
    let shown = frame.show(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 10.0);

        let mapping = &PAD_MAPPINGS[selected_pad];
        let pad = &params.pads[selected_pad];
        // What the current kit plays on this pad: its name, whether it
        // has the piece at all, its articulation pair.
        let kit = bridge.kit_pads.current();
        let kit_pad = &kit.pads[selected_pad];

        // Sample identity is published by whoever built the live kit; the
        // clone keeps the bridge lock off the whole inspector frame.
        let sample_info = bridge
            .pad_samples
            .lock()
            .get(selected_pad)
            .cloned()
            .flatten();

        // Which round-robin take last fired for this pad, published by the
        // audio thread on every note-on.
        let rr = rr_display::unpack(bridge.last_rr[selected_pad].load(Ordering::Relaxed));

        draw_pad_head(ui, bridge, mapping, kit_pad, pad, rr);
        draw_sample_stage(ui, sample_info.as_ref(), kit_pad.present);
        draw_knob_grid(ui, pad, mapping);

        if let Some(articulation) = &kit_pad.articulation {
            draw_articulations(ui, bridge, pad, articulation);
        }

        draw_mic_and_oh_row(ui, bridge, catalog, pad, mapping, selected_pad);
    });
    super::probe(ui, "inspector", shown.response.rect);
}

fn draw_pad_head(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    mapping: &crate::drum_map::PadMapping,
    kit_pad: &crate::pad_map::KitPad,
    pad: &crate::params::PadParams,
    rr: Option<rr_display::RoundRobin>,
) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(kit_pad.name.as_str())
                .italics()
                .color(if kit_pad.present {
                    theme::TEXT_1
                } else {
                    theme::TEXT_4
                })
                .size(20.0),
        );
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new(format!(
                "MIDI {} · {}",
                mapping.note,
                midi_note_name(mapping.note),
            ))
            .color(theme::TEXT_3)
            .size(11.0)
            .monospace(),
        );
        ui.add_space(10.0);
        if !kit_pad.present {
            let shown = ui
                .label(
                    egui::RichText::new(NOT_IN_KIT)
                        .color(theme::WARM)
                        .size(11.0),
                )
                .on_hover_text(
                    "The selected kit has no recording for this pad, so it is \
                     silent. The built-in samples play only when no kit is \
                     selected.",
                );
            super::probe(ui, "inspector.not_in_kit", shown.rect);
        }
        // Round robin: which take of how many the last hit used. Updates
        // as takes cycle — the editor repaints ~10× a second (ba #1329).
        match rr {
            Some(rr) => {
                ui.label(
                    egui::RichText::new(rr.label())
                        .color(if rr.cycles() {
                            theme::ACCENT_SOFT
                        } else {
                            theme::TEXT_3
                        })
                        .size(11.0)
                        .monospace(),
                )
                .on_hover_text(if rr.cycles() {
                    "Round robin: the take that fired on the last hit, and how \
                     many this velocity layer holds."
                } else {
                    "This velocity layer has a single take, so every hit plays \
                     the same sample."
                });
            }
            None => {
                ui.label(
                    egui::RichText::new("take — of —")
                        .color(theme::TEXT_4)
                        .size(11.0)
                        .monospace(),
                )
                .on_hover_text("Round robin: play this pad to see which take fires.");
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Enabled chip — driven by the negated mute param.
            let enabled = !pad.mute.value();
            draw_enabled_chip(ui, pad, enabled);
            ui.add_space(8.0);
            // Audition: hand the pad's note to the audio thread through
            // the bridge's trigger queue (ba todo #1328). The audio side
            // feeds it to the same `note_on` a MIDI hit takes, so this
            // sounds like playing the pad — and it works with the
            // transport stopped, because the plugin renders regardless.
            let clicked = ui
                .add_enabled(
                    kit_pad.present,
                    egui::Button::new(
                        egui::RichText::new("▶ Audition")
                            .color(theme::TEXT_2)
                            .size(11.0),
                    )
                    .fill(egui::Color32::TRANSPARENT)
                    .stroke(egui::Stroke::new(1.0, theme::LINE_2))
                    .corner_radius(6.0)
                    .min_size(egui::vec2(0.0, 24.0)),
                )
                .on_hover_text("Play this pad once, at a firm velocity.")
                .on_disabled_hover_text(NOT_IN_KIT)
                .clicked();
            if clicked {
                bridge.audition(mapping.note);
            }
        });
    });
    ui.add_space(2.0);
    let p = ui.painter();
    let r = ui.min_rect();
    let y = r.bottom() - 2.0;
    p.line_segment(
        [egui::pos2(r.left(), y), egui::pos2(r.right(), y)],
        egui::Stroke::new(1.0, theme::LINE_2),
    );
}

fn draw_enabled_chip(ui: &mut egui::Ui, pad: &crate::params::PadParams, enabled: bool) {
    let frame = egui::Frame::default()
        .fill(theme::BG_1)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(10, 4));
    let resp = frame
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(
                    egui::vec2(8.0, 8.0),
                    egui::Sense::hover(),
                );
                let dot = if enabled { theme::GOOD } else { theme::TEXT_4 };
                ui.painter().circle_filled(r.center(), 4.0, dot);
                ui.label(
                    egui::RichText::new(if enabled { "Enabled" } else { "Muted" })
                        .color(theme::TEXT_2)
                        .size(11.0),
                );
            });
        })
        .response;

    if resp.interact(egui::Sense::click()).clicked() {
        let muted = pad.mute.value();
        pad.mute.set_plain(if muted { 0.0 } else { 1.0 });
    }
}

/// What the inspector says about a pad whose piece the kit lacks (D7).
pub(crate) const NOT_IN_KIT: &str = "Not in this kit";

/// SAMPLE stage: the waveform of the take this pad plays at full velocity.
///
/// Everything drawn here comes from [`PadSampleInfo`], measured from the
/// decoded take by whoever built the kit. When no info has been published
/// for this pad — no kit loaded yet, or the pad has no bank in this kit —
/// the stage says so instead of drawing an invented shape (ba todo #1276).
fn draw_sample_stage(ui: &mut egui::Ui, info: Option<&PadSampleInfo>, present: bool) {
    let frame = egui::Frame::default()
        .fill(theme::BG_1)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::ZERO);

    frame.show(ui, |ui| {
        let avail = ui.available_width();
        let h = 110.0;
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(avail, h), egui::Sense::hover());
        let p = ui.painter_at(rect);

        // Faint grid.
        for x_step in 0..((avail / 20.0).ceil() as i32) {
            let x = rect.left() + x_step as f32 * 20.0;
            p.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(0.5, theme::LINE_2),
            );
        }
        let mid_y = rect.center().y;
        for x in (rect.left() as i32..rect.right() as i32).step_by(3) {
            p.line_segment(
                [
                    egui::pos2(x as f32, mid_y),
                    egui::pos2(x as f32 + 1.0, mid_y),
                ],
                egui::Stroke::new(0.5, theme::TEXT_4),
            );
        }

        // Top-left label.
        p.text(
            rect.left_top() + egui::vec2(10.0, 10.0),
            egui::Align2::LEFT_TOP,
            "SAMPLE",
            egui::FontId::proportional(10.0),
            theme::TEXT_3,
        );

        let Some(info) = info else {
            p.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                if present {
                    "no sample detail available — load a kit"
                } else {
                    "Not in this kit — this pad is silent"
                },
                egui::FontId::proportional(11.0),
                theme::TEXT_4,
            );
            return;
        };

        // Top-right: which bank the shown take came from.
        p.text(
            rect.right_top() + egui::vec2(-10.0, 10.0),
            egui::Align2::RIGHT_TOP,
            info.source_text(),
            egui::FontId::monospace(10.0),
            theme::TEXT_2,
        );

        // Waveform — the published min/max envelope of the real take.
        let half = h * 0.36;
        let buckets = info.envelope.len();
        if buckets > 0 {
            let bucket_w = avail / buckets as f32;
            for (i, (lo, hi)) in info.envelope.iter().enumerate() {
                let x = rect.left() + i as f32 * bucket_w;
                let y_hi = mid_y - hi.clamp(-1.0, 1.0) * half;
                let y_lo = mid_y - lo.clamp(-1.0, 1.0) * half;
                p.line_segment(
                    [egui::pos2(x, y_hi), egui::pos2(x, y_lo.max(y_hi + 0.5))],
                    egui::Stroke::new(bucket_w.max(0.9), theme::ACCENT_SOFT),
                );
            }
        }

        // Start/end markers as ticks at the take's boundaries.
        let mk = |x: f32| {
            p.line_segment(
                [
                    egui::pos2(x, rect.top() + 4.0),
                    egui::pos2(x, rect.bottom() - 4.0),
                ],
                egui::Stroke::new(0.6, theme::WARM),
            );
        };
        mk(rect.left() + 1.0);
        mk(rect.right() - 1.0);

        // Bottom row: real duration on the left, layer/take identity right.
        p.text(
            rect.left_bottom() + egui::vec2(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            info.duration_text().unwrap_or_else(|| "—".to_string()),
            egui::FontId::monospace(9.5),
            theme::TEXT_4,
        );
        p.text(
            rect.right_bottom() + egui::vec2(-8.0, -8.0),
            egui::Align2::RIGHT_BOTTOM,
            info.layer_text(),
            egui::FontId::monospace(9.5),
            theme::TEXT_4,
        );
    });
}

fn draw_knob_grid(
    ui: &mut egui::Ui,
    pad: &crate::params::PadParams,
    mapping: &crate::drum_map::PadMapping,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(18.0, 0.0);

        // 1: Volume (dB, along the param's own travel).
        if let Some(nv) = level_knob(ui, "Volume", &pad.volume) {
            pad.volume.set_normalized(nv);
        }

        // 2: Pan (bipolar).
        let pan = pad.pan.value();
        let pan_fmt = if pan.abs() < 0.01 {
            "C".to_string()
        } else if pan > 0.0 {
            format!("R {:.0}", pan * 100.0)
        } else {
            format!("L {:.0}", -pan * 100.0)
        };
        if let Some(np) = widgets::knob_bipolar(ui, "Pan", pan, &pan_fmt, 0.0) {
            pad.pan.set_value(np);
        }

        // 3: Overhead trim (dB).
        let oh = pad.trim(MicSlot::Overhead);
        if let Some(no) = level_knob(ui, "OH Trim", oh) {
            oh.set_normalized(no);
        }

        // 4: Second close mic's trim (dB). Only when this pad has 2 mics;
        // the first mic's trim is on the MICS card.
        if mapping.close_mic_positions.len() == 2 {
            let mic2 = pad.trim(MicSlot::Close2);
            if let Some(nb) = level_knob(ui, "Mic 2 Trim", mic2) {
                mic2.set_normalized(nb);
            }
        } else {
            draw_placeholder_knob(ui, "Mic 2 Trim");
        }
    });
}

/// A knob over a dB level param's own travel (its skew), reading in dB.
/// Returns the new normalized value. (The K5 editor rebuild replaces the
/// knob grid; this keeps the v2 levels honest until then.)
fn level_knob(
    ui: &mut egui::Ui,
    label: &str,
    param: &resonance_plugin::FloatParam,
) -> Option<f32> {
    let text = crate::level::db_label(param.value());
    widgets::knob_unipolar(
        ui,
        label,
        param.normalized_value(),
        &text,
        param.default_normalized(),
    )
}

fn draw_placeholder_knob(ui: &mut egui::Ui, label: &str) {
    let size = 60.0;
    let h = size + 32.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, h), egui::Sense::hover());
    let p = ui.painter_at(rect);
    let center = egui::pos2(rect.center().x, rect.top() + size * 0.5);
    let radius = size * 0.5 - 4.0;
    p.circle_filled(center, radius, theme::BG_1);
    p.circle_stroke(center, radius, egui::Stroke::new(1.0, theme::LINE_2));
    p.text(
        center,
        egui::Align2::CENTER_CENTER,
        "—",
        egui::FontId::proportional(16.0),
        theme::TEXT_4,
    );
    p.text(
        egui::pos2(rect.center().x, rect.top() + size + 4.0),
        egui::Align2::CENTER_TOP,
        "—",
        egui::FontId::monospace(10.5),
        theme::TEXT_4,
    );
    p.text(
        egui::pos2(rect.center().x, rect.top() + size + 18.0),
        egui::Align2::CENTER_TOP,
        label.to_uppercase(),
        egui::FontId::proportional(9.0),
        theme::TEXT_4,
    );
}

/// Articulation chips. The chips are a view of the pad's articulation
/// *parameter* — they read it and write it, and the reload happens
/// because the parameter moved, not because a chip was clicked. That is
/// the same path host automation and `set_plugin_param` take (ba todo
/// #1325), so the three cannot drift apart.
///
/// The parameter's values are generic (0 = primary piece, 1 = alternate);
/// the chips carry the kit's labels for them ("punch" / "deep").
fn draw_articulations(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    pad: &crate::params::PadParams,
    articulation: &crate::pad_map::PadArticulation,
) {
    let frame = inline_group_frame();
    frame.show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("ARTICULATIONS")
                    .color(theme::TEXT_3)
                    .size(10.5)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(articulation.label.as_str())
                        .color(theme::TEXT_3)
                        .size(10.5)
                        .monospace(),
                );
            });
        });
        ui.add_space(2.0);
        let current = pad.articulation.value();
        ui.horizontal(|ui| {
            let chips = [
                (crate::articulation::ARTICULATION_PRIMARY, &articulation.primary_label),
                (crate::articulation::ARTICULATION_ALT, &articulation.alt_label),
            ];
            for (index, label) in chips {
                let clicked = super::probed(ui, &format!("articulation.{index}"), |ui| {
                    widgets::chip_button(ui, label, index == current)
                });
                if clicked && index != current {
                    pad.articulation.set_value(index);
                    // The parameter is the source of truth; the reload is
                    // the watcher's job. Ping it so the click lands now
                    // instead of at its next poll.
                    bridge.wake_articulation_watcher();
                }
            }
        });
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new(
                "Reloads the pad's samples. Automatable — the host's \
                 \"Pad Articulation\" parameter is this control.",
            )
            .color(theme::TEXT_4)
            .size(10.0),
        );
    });
}

fn draw_mic_and_oh_row(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    catalog: &ManifestMicCatalog,
    pad: &crate::params::PadParams,
    mapping: &crate::drum_map::PadMapping,
    pad_idx: usize,
) {
    // Floored (`body_width`): a compositor can hand the editor a rect
    // narrower than its declared minimum (ba todo #1377), and
    // `set_min_width`/`set_max_width` below panic on a negative size.
    let half = super::body_width(ui, 12.0) * 0.5;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(12.0, 0.0);
        ui.vertical(|ui| {
            ui.set_min_width(half);
            ui.set_max_width(half);
            draw_close_mics_card(ui, bridge, catalog, pad, mapping, pad_idx);
        });
        ui.vertical(|ui| {
            ui.set_min_width(half);
            ui.set_max_width(half);
            draw_oh_blend_card(ui, bridge, catalog, pad);
        });
    });
}

fn draw_close_mics_card(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    catalog: &ManifestMicCatalog,
    pad: &crate::params::PadParams,
    mapping: &crate::drum_map::PadMapping,
    pad_idx: usize,
) {
    inline_group_frame().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("CLOSE MICS")
                    .color(theme::TEXT_3)
                    .size(10.5)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let count = mapping.close_mic_positions.len();
                ui.label(
                    egui::RichText::new(format!("{} routed", count))
                        .color(theme::TEXT_3)
                        .size(10.5)
                        .monospace(),
                );
            });
        });
        ui.add_space(2.0);

        if mapping.close_mic_positions.is_empty() {
            ui.label(theme::hint_text("No close mic (overhead only)."));
            return;
        }

        // Mic dropdowns — one per position.
        let mut choices_to_apply: Vec<(String, String)> = Vec::new();
        for position in mapping.close_mic_positions {
            let available = catalog.close_setups(position);
            let current = bridge
                .pad_choices
                .lock()
                .get(pad_idx)
                .and_then(|c| c.close_setups.get(*position).cloned())
                .or_else(|| available.first().cloned())
                .unwrap_or_else(|| "(none)".to_string());

            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(humanize_position(position).to_uppercase())
                        .color(theme::TEXT_3)
                        .size(9.5),
                );
                egui::ComboBox::from_id_salt(format!("pad_{}_mic_{}", pad_idx, position))
                    .width(super::body_width(ui, 4.0))
                    .selected_text(
                        egui::RichText::new(current.clone())
                            .color(theme::TEXT_1)
                            .size(11.0)
                            .monospace(),
                    )
                    .show_ui(ui, |ui| {
                        if available.is_empty() {
                            ui.label(theme::hint_text("(load a kit first)"));
                        }
                        for key in &available {
                            if ui
                                .selectable_label(*key == current, key.as_str())
                                .clicked()
                            {
                                choices_to_apply.push((position.to_string(), key.clone()));
                            }
                        }
                    });
            });
        }
        if !choices_to_apply.is_empty() {
            {
                let mut guard = bridge.pad_choices.lock();
                for (position, key) in choices_to_apply {
                    guard[pad_idx].close_setups.insert(position, key);
                }
            }
            reload_kit(bridge);
        }

        // A dB trim per close mic the pad has (they replace v1's balance).
        if !mapping.close_mic_positions.is_empty() {
            let (left_label, right_label) = mic_balance_labels(mapping.close_mic_positions);
            let slots: &[(MicSlot, &str)] = if mapping.close_mic_positions.len() == 2 {
                &[(MicSlot::Close1, left_label), (MicSlot::Close2, right_label)]
            } else {
                &[(MicSlot::Close1, "CLOSE")]
            };
            for &(slot, label) in slots {
                ui.add_space(6.0);
                let trim = pad.trim(slot);
                trim_slider(ui, &format!("{label} TRIM"), trim);
            }
        }
    });
}

/// A labelled slider over a dB trim param's own travel.
fn trim_slider(ui: &mut egui::Ui, label: &str, param: &resonance_plugin::FloatParam) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(label)
                .color(theme::TEXT_3)
                .size(10.0),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(crate::level::db_label(param.value()))
                    .color(theme::TEXT_1)
                    .size(11.0)
                    .monospace(),
            );
        });
    });
    let w = ui.available_width();
    if let Some(nv) = widgets::slider_unipolar(ui, w, param.normalized_value()) {
        param.set_normalized(nv);
    }
}

fn draw_oh_blend_card(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    catalog: &ManifestMicCatalog,
    pad: &crate::params::PadParams,
) {
    inline_group_frame().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("OVERHEAD BLEND")
                    .color(theme::TEXT_3)
                    .size(10.5)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let current = bridge.overhead_setup_key.lock().clone();
                ui.label(
                    egui::RichText::new(if current.is_empty() {
                        "—".to_string()
                    } else {
                        current.clone()
                    })
                    .color(theme::TEXT_3)
                    .size(10.5)
                    .monospace(),
                );
            });
        });
        ui.add_space(2.0);

        // Overhead-setup picker.
        let setups = catalog.overhead_setups();
        let current = bridge.overhead_setup_key.lock().clone();
        let mut new_choice: Option<String> = None;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("SETUP")
                    .color(theme::TEXT_3)
                    .size(9.5),
            );
            egui::ComboBox::from_id_salt("oh_setup_inspector")
                .width(super::body_width(ui, 4.0))
                .selected_text(
                    egui::RichText::new(if current.is_empty() {
                        "(load a kit first)".to_string()
                    } else {
                        current.clone()
                    })
                    .color(theme::TEXT_1)
                    .size(11.0)
                    .monospace(),
                )
                .show_ui(ui, |ui| {
                    if setups.is_empty() {
                        ui.label(theme::hint_text("(load a kit first)"));
                    }
                    for key in &setups {
                        if ui
                            .selectable_label(*key == current, key.as_str())
                            .clicked()
                        {
                            new_choice = Some(key.clone());
                        }
                    }
                });
        });
        if let Some(key) = new_choice {
            *bridge.overhead_setup_key.lock() = key;
            reload_kit(bridge);
        }

        ui.add_space(6.0);
        trim_slider(ui, "OH TRIM", pad.trim(MicSlot::Overhead));
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(
                "Trims this pad's overhead take. Pull it to -inf to keep \
                 the hit out of the overheads entirely.",
            )
            .color(theme::TEXT_3)
            .size(10.5),
        );
    });
}

fn inline_group_frame() -> egui::Frame {
    egui::Frame::default()
        .fill(theme::BG_1)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(14, 12))
}

fn humanize_position(position: &str) -> &'static str {
    match position {
        "KickIn" => "Kick In",
        "KickOut" => "Kick Out",
        "SNTop" => "Snare Top",
        "SNBtm" => "Snare Btm",
        "Hat" => "Hi-Hat",
        "Tom01" => "Tom 1",
        "Tom02" => "Tom 2",
        "TomFloor" => "Tom Floor",
        _ => "Mic",
    }
}

fn mic_balance_labels(positions: &[&str]) -> (&'static str, &'static str) {
    match positions {
        ["KickIn", "KickOut"] => ("In", "Out"),
        ["SNTop", "SNBtm"] => ("Top", "Btm"),
        _ => ("A", "B"),
    }
}

fn midi_note_name(note: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    // MIDI 60 = C4 convention.
    let octave = note as i32 / 12 - 1;
    let n = note as usize % 12;
    format!("{}{}", NAMES[n], octave)
}
