//! Right-panel per-pad detail view.
//!
//! Layout, top-to-bottom:
//!   • Pad title (Instrument Serif italic) + meta + Audition + Enabled chip
//!   • Sample stage (placeholder waveform canvas)
//!   • 4-knob row: Volume / Pan / OH Blend / Balance
//!   • Articulations chips (only when the pad supports articulation)
//!   • Close mics card (mic pickers + balance slider) + Overhead Blend card
//!
//! Wiring follows the existing param surface — no new params are introduced.
//! For pads without two close mics, the balance knob renders as a dim
//! placeholder so the knob grid stays a consistent 4-cell row.

use wayland_plugin_gui::egui;

use resonance_plugin::param::Param;

use crate::drum_map::PAD_MAPPINGS;
use crate::mic_catalog::ManifestMicCatalog;
use crate::params::DrumParams;
use crate::KitBridge;

use super::{reload_kit, theme, widgets};

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
    frame.show(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 10.0);

        let mapping = &PAD_MAPPINGS[selected_pad];
        let pad = &params.pads[selected_pad];

        draw_pad_head(ui, mapping, pad);
        draw_sample_stage(ui);
        draw_knob_grid(ui, pad, mapping);

        if mapping.has_articulation {
            draw_articulations(ui, bridge, pad, selected_pad);
        }

        draw_mic_and_oh_row(ui, bridge, catalog, pad, mapping, selected_pad);
    });
}

fn draw_pad_head(
    ui: &mut egui::Ui,
    mapping: &crate::drum_map::PadMapping,
    pad: &crate::params::PadParams,
) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(mapping.name)
                .italics()
                .color(theme::TEXT_1)
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

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Enabled chip — driven by the negated mute param.
            let enabled = !pad.mute.value();
            draw_enabled_chip(ui, pad, enabled);
            ui.add_space(8.0);
            let resp = ui.add(
                egui::Button::new(
                    egui::RichText::new("▶ Audition")
                        .color(theme::TEXT_2)
                        .size(11.0),
                )
                .fill(egui::Color32::TRANSPARENT)
                .stroke(egui::Stroke::new(1.0, theme::LINE))
                .corner_radius(6.0)
                .min_size(egui::vec2(0.0, 24.0)),
            );
            // Audition: there is no audition pipeline yet — this is a
            // visual control reserved for a future trigger.
            let _ = resp;
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

fn draw_sample_stage(ui: &mut egui::Ui) {
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
        // Top-right filename placeholder.
        p.text(
            rect.right_top() + egui::vec2(-10.0, 10.0),
            egui::Align2::RIGHT_TOP,
            "—",
            egui::FontId::monospace(10.0),
            theme::TEXT_2,
        );

        // Placeholder waveform — a soft decaying sinusoid centered.
        let n = 240usize;
        let mut pts: Vec<egui::Pos2> = Vec::with_capacity(n);
        for i in 0..n {
            let t = i as f32 / (n - 1) as f32;
            let env = (-t * 4.0).exp();
            let osc = (t * 18.0).sin();
            let y = mid_y - env * osc * (h * 0.32);
            let x = rect.left() + t * avail;
            pts.push(egui::pos2(x, y));
        }
        p.add(egui::Shape::line(
            pts.clone(),
            egui::Stroke::new(0.9, theme::ACCENT_SOFT),
        ));

        // Start/end markers as dashed yellow ticks at 4 px from the edge.
        let mk = |x: f32| {
            p.line_segment(
                [
                    egui::pos2(x, rect.top() + 4.0),
                    egui::pos2(x, rect.bottom() - 4.0),
                ],
                egui::Stroke::new(0.6, theme::WARM),
            );
        };
        mk(rect.left() + 6.0);
        mk(rect.right() - 6.0);

        // Bottom markers row.
        p.text(
            rect.left_bottom() + egui::vec2(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            "00:00.000",
            egui::FontId::monospace(9.5),
            theme::TEXT_4,
        );
        p.text(
            rect.right_bottom() + egui::vec2(-8.0, -8.0),
            egui::Align2::RIGHT_BOTTOM,
            "—",
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

        // 1: Volume (unipolar).
        let v = pad.volume.value();
        let fv = format!("{:.2}", v);
        if let Some(nv) = widgets::knob_unipolar(ui, "Volume", v, &fv, 0.8) {
            pad.volume.set_value(nv);
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

        // 3: OH Blend (unipolar).
        let oh = pad.oh_blend.value();
        let oh_fmt = format!("{:.2}", oh);
        if let Some(no) = widgets::knob_unipolar(ui, "OH Blend", oh, &oh_fmt, 1.0) {
            pad.oh_blend.set_value(no);
        }

        // 4: Balance (bipolar, warm). Only enabled when this pad has 2 mics.
        if mapping.close_mic_positions.len() == 2 {
            let bal_unit = pad.balance.value(); // 0..1
            let signed = bal_unit * 2.0 - 1.0;
            let bal_fmt = format!("{:+.2}", signed);
            if let Some(nb) = widgets::knob_bipolar(ui, "Balance", signed, &bal_fmt, 0.0) {
                pad.balance.set_value((nb + 1.0) * 0.5);
            }
        } else {
            draw_placeholder_knob(ui, "Balance");
        }
    });
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

fn draw_articulations(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    pad: &crate::params::PadParams,
    pad_idx: usize,
) {
    let frame = inline_group_frame();
    frame.show(ui, |ui| {
        ui.set_min_width(ui.available_width() - 28.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("ARTICULATIONS")
                    .color(theme::TEXT_3)
                    .size(10.5)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new("2 options")
                        .color(theme::TEXT_3)
                        .size(10.5)
                        .monospace(),
                );
            });
        });
        ui.add_space(2.0);
        let current = bridge.articulations.lock()[pad_idx];
        ui.horizontal(|ui| {
            if widgets::chip_button(ui, "mit Teppich", !current) && current {
                bridge.articulations.lock()[pad_idx] = false;
                pad.articulation.set_plain(0.0);
                reload_kit(bridge);
            }
            if widgets::chip_button(ui, "ohne Teppich", current) && !current {
                bridge.articulations.lock()[pad_idx] = true;
                pad.articulation.set_plain(1.0);
                reload_kit(bridge);
            }
        });
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
    let avail = ui.available_width();
    let half = (avail - 12.0) * 0.5;
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
        ui.set_min_width(ui.available_width() - 28.0);
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
                    .width(ui.available_width() - 4.0)
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

        // Balance slider when we have 2 close mics.
        if mapping.close_mic_positions.len() == 2 {
            ui.add_space(6.0);
            let (left_label, right_label) = mic_balance_labels(mapping.close_mic_positions);
            let bal_unit = pad.balance.value();
            let signed = bal_unit * 2.0 - 1.0;
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{} ◂▸ {}", left_label, right_label))
                        .color(theme::TEXT_3)
                        .size(10.0),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(format!("{:+.2}", signed))
                            .color(theme::TEXT_1)
                            .size(11.0)
                            .monospace(),
                    );
                });
            });
            let w = ui.available_width();
            if let Some(new_signed) = widgets::slider_bipolar_warm(ui, w, signed) {
                pad.balance.set_value((new_signed + 1.0) * 0.5);
            }
        }
    });
}

fn draw_oh_blend_card(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    catalog: &ManifestMicCatalog,
    pad: &crate::params::PadParams,
) {
    inline_group_frame().show(ui, |ui| {
        ui.set_min_width(ui.available_width() - 28.0);
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
                .width(ui.available_width() - 4.0)
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
        // OH amount slider.
        let oh = pad.oh_blend.value();
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("OH AMOUNT")
                    .color(theme::TEXT_3)
                    .size(10.0),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!("{:.2}", oh))
                        .color(theme::TEXT_1)
                        .size(11.0)
                        .monospace(),
                );
            });
        });
        let w = ui.available_width();
        if let Some(nv) = widgets::slider_unipolar(ui, w, oh) {
            pad.oh_blend.set_value(nv);
        }
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(
                "Scales this pad's contribution to the Overhead bus. Set \
                 to 0 to keep the hit out of overheads entirely.",
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
