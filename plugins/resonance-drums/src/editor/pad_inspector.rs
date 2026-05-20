//! Per-pad inspector / detail panel.
//!
//! Shows volume/pan/mute, the articulation toggle for snare-class pads,
//! per-position close-mic dropdowns, the close-mic balance slider for
//! pads with two mic positions, and the overhead-blend slider.

use wayland_plugin_gui::egui;

use resonance_plugin::param::Param;

use crate::drum_map::PAD_MAPPINGS;
use crate::mic_catalog::ManifestMicCatalog;
use crate::params::DrumParams;
use crate::KitBridge;

use super::{reload_kit, theme};

/// Render the per-pad detail view inside the central panel.
pub fn draw(
    ui: &mut egui::Ui,
    params: &DrumParams,
    bridge: &KitBridge,
    catalog: &ManifestMicCatalog,
    selected_pad: usize,
) {
    let pad_idx = selected_pad;
    let mapping = &PAD_MAPPINGS[pad_idx];
    ui.label(
        egui::RichText::new(mapping.name)
            .size(theme::TITLE_SIZE)
            .strong()
            .color(theme::ACCENT),
    );
    ui.label(
        egui::RichText::new(format!("MIDI note {}", mapping.note))
            .size(theme::SECTION_LABEL_SIZE)
            .color(theme::TEXT_DIM),
    );
    ui.add_space(6.0);

    let pad = &params.pads[pad_idx];

    // Volume + mute on one row.
    ui.horizontal(|ui| {
        let mut vol = pad.volume.value();
        if ui
            .add(egui::Slider::new(&mut vol, 0.0..=1.0).text("Volume"))
            .changed()
        {
            pad.volume.set_value(vol);
        }
        let mut muted = pad.mute.value();
        if ui.checkbox(&mut muted, "Mute").changed() {
            pad.mute.set_plain(if muted { 1.0 } else { 0.0 });
        }
    });

    // Pan slider.
    let mut pan = pad.pan.value();
    if ui
        .add(egui::Slider::new(&mut pan, -1.0..=1.0).text("Pan"))
        .changed()
    {
        pad.pan.set_value(pan);
    }

    // Articulation toggle (mit/ohne Teppich) — only for pads that have one.
    if mapping.has_articulation {
        ui.add_space(theme::SECTION_GAP);
        ui.separator();
        ui.label(theme::section_label("ARTICULATION"));
        let current_art = bridge.articulations.lock()[pad_idx];
        let label = if current_art {
            "ohne Teppich (snare wires off)"
        } else {
            "mit Teppich (snare wires on)"
        };
        let mut toggled = current_art;
        if ui.checkbox(&mut toggled, label).changed() {
            bridge.articulations.lock()[pad_idx] = toggled;
            // Also update the param so it persists in the plugin state.
            pad.articulation.set_plain(if toggled { 1.0 } else { 0.0 });
            reload_kit(bridge);
        }
    }

    ui.add_space(theme::SECTION_GAP);
    ui.separator();
    ui.label(theme::section_label("CLOSE MICS"));

    // Close-mic dropdowns — one per position this pad type uses.
    // Cymbal-class pads have no positions and render a hint instead.
    if mapping.close_mic_positions.is_empty() {
        ui.label(theme::hint_text(
            "No close mic for this drum (overhead only)",
        ));
    } else {
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

            ui.horizontal(|ui| {
                ui.label(theme::hint_text(*position));
                egui::ComboBox::from_id_salt(format!("pad_{}_mic_{}", pad_idx, position))
                    .selected_text(current.clone())
                    .show_ui(ui, |ui| {
                        if available.is_empty() {
                            ui.label(theme::hint_text("(load a kit first)"));
                        }
                        for key in &available {
                            if ui.selectable_label(*key == current, key.as_str()).clicked() {
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

        // Balance slider only for pads that use two close-mic positions
        // (kick In/Out, snare Top/Btm). Label matches the pad type so
        // the UX is self-explanatory.
        if mapping.close_mic_positions.len() == 2 {
            let (left_label, right_label) = match mapping.close_mic_positions {
                ["KickIn", "KickOut"] => ("In", "Out"),
                ["SNTop", "SNBtm"] => ("Top", "Btm"),
                [a, b] => (a as &str, b as &str),
                _ => ("A", "B"),
            };
            let mut balance = pad.balance.value();
            if ui
                .add(
                    egui::Slider::new(&mut balance, 0.0..=1.0)
                        .text(format!("{} <-> {}", left_label, right_label))
                        .custom_formatter(|x, _| format!("{:.2}", x)),
                )
                .changed()
            {
                pad.balance.set_value(balance);
            }
        }
    }

    ui.add_space(theme::SECTION_GAP);
    ui.separator();
    ui.label(theme::section_label("OVERHEAD BLEND"));

    let mut oh = pad.oh_blend.value();
    if ui
        .add(
            egui::Slider::new(&mut oh, 0.0..=1.0)
                .text("OH amount")
                .custom_formatter(|x, _| format!("{:.2}", x)),
        )
        .changed()
    {
        pad.oh_blend.set_value(oh);
    }
    ui.label(
        egui::RichText::new(
            "Scales this pad's contribution to the Overhead output port. \
             Set to 0 to keep the hit out of the overhead bus entirely.",
        )
        .size(theme::SECTION_LABEL_SIZE)
        .color(theme::TEXT_DIM),
    );
}
