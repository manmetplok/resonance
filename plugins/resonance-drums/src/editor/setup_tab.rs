//! The Setup tab (§6.4): kit-wide configuration.
//!
//! - MIC BANKS (E15): the three overhead slots (a setup each, from the
//!   kit's catalog, and `oh_N_level`), the room bank (on/off, setup,
//!   level) and the bleed banks (on/off, level, and what they add);
//! - STREAMING (E14): the preload, and what it costs and how it is doing;
//! - THE KIT: the library entry's facts — not its name, which the header
//!   shows (once);
//! - PADS: per pad its note (read-only), output port and choke group —
//!   the ports dimmed in Stereo, where they play nothing.
//!
//! A bank's setup is plugin state, not a param (a host cannot automate a
//! reload); picking one is still one undoable host edit, announced
//! through the state-excluded `mic_setup_rev`
//! ([`crate::KitBridge::announce_mic_setup_edit`]). Its on/off and level
//! are params, announced as themselves.

use plugin_gui_core::egui;
use resonance_common::drumkit_library::Entry;
use resonance_plugin::param::Param;

use crate::drum_map::NUM_PADS;
use crate::kit::{MAX_OVERHEAD_SLOTS, OUTPUT_PORT_NAMES};
use crate::mic_catalog::ManifestMicCatalog;
use crate::params::{
    BANK_ON_LABELS, CHOKE_KIT, MAX_CHOKE_GROUP, OUTPUT_CHOICE_LABELS, OUTPUT_KIT, OUTPUT_MODE_MULTI,
};
use crate::sample_info::format_bytes;
use crate::KitBridge;

use super::app::{column, DrumsEditorApp};
use super::controls::{self, Labels};
use super::pad_inspector::{choke_text, output_text};
use super::{pad_grid, probe, theme};

const GAP: f32 = 12.0;
const ROW_H: f32 = 26.0;
/// A bank row's label column.
const LABEL_W: f32 = 64.0;

pub(super) fn draw(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    let avail = ui.available_size();
    let left_w = ((avail.x - GAP) * 0.55).max(0.0);
    let right_w = (avail.x - GAP - left_w).max(0.0);
    let catalog = app.bridge.catalog.lock().clone();
    let entry = app.loaded_entry();

    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(GAP, 0.0);
        column(ui, egui::vec2(left_w, avail.y), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("setup_left_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, GAP);
                    draw_banks(ui, &app.bridge, &catalog, &mut app.labels);
                    draw_streaming(ui, &app.bridge);
                    draw_kit_facts(ui, &app.bridge, entry.as_deref());
                });
        });
        column(ui, egui::vec2(right_w, avail.y), |ui| draw_pad_table(ui, app));
    });
}

fn draw_banks(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    catalog: &ManifestMicCatalog,
    labels: &mut Labels,
) {
    let params = &bridge.params;
    let shown = controls::card().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        controls::heading(ui, "MIC BANKS");

        // Overheads: up to three setups at once, each with its level.
        let overheads = catalog.overhead_setups();
        sub_heading(ui, "Overheads");
        if overheads.is_empty() {
            ui.label(theme::hint_text("This kit has no overhead mics."));
        } else {
            let slots = bridge.overhead_slots();
            for (slot, current) in slots.iter().enumerate().take(MAX_OVERHEAD_SLOTS) {
                ui.horizontal(|ui| {
                    controls::row_label(ui, &format!("Slot {}", slot + 1), LABEL_W);
                    // Empty is "Off" for slots 2 and 3, and the kit's
                    // default setup for slot 1 — an option there too, so
                    // a pick can be taken back.
                    let empty = if slot > 0 { "Off" } else { "Kit default" };
                    let shown = if current.is_empty() {
                        empty.to_string()
                    } else {
                        catalog.label(current)
                    };
                    let w = super::body_width(ui, 0.0);
                    let options = std::iter::once(("", empty.to_string()))
                        .chain(overheads.iter().map(|k| (k.as_str(), catalog.label(k))));
                    if let Some(key) =
                        controls::combo(ui, &format!("setup.oh.{slot}"), w, &shown, current.as_str(), options)
                    {
                        bridge.set_overhead_slot(slot, key);
                        bridge.announce_mic_setup_edit();
                    }
                });
                if slot == 0 || !current.is_empty() {
                    level_row(ui, bridge, &format!("setup.oh.{slot}.level"), &params.oh_levels[slot], labels);
                }
            }
        }

        // Room.
        let rooms = catalog.room_setups();
        sub_heading(ui, "Room");
        if rooms.is_empty() {
            ui.label(theme::hint_text("This kit has no room mics."));
        } else {
            ui.horizontal(|ui| {
                controls::row_label(ui, "", LABEL_W);
                if controls::segmented(ui, bridge, "setup.room.on", &params.room_on, BANK_ON_LABELS) {
                    // A bank switch is a load: the watcher does it.
                    bridge.wake_articulation_watcher();
                }
            });
            let current = bridge.mic_banks.lock().room.clone();
            ui.horizontal(|ui| {
                controls::row_label(ui, "Setup", LABEL_W);
                let shown = if current.is_empty() {
                    format!("{} (first)", catalog.label(&rooms[0]))
                } else {
                    catalog.label(&current)
                };
                let w = super::body_width(ui, 0.0);
                // Kept while the room is off, for when it is on.
                let dim = (!params.room_enabled())
                    .then_some("The room is off: this setup plays once it is on");
                if let Some(key) = controls::combo_dimmed(
                    ui,
                    "setup.room.setup",
                    w,
                    &shown,
                    current.as_str(),
                    rooms.iter().map(|k| (k.as_str(), catalog.label(k))),
                    dim,
                ) {
                    bridge.set_room_setup(key);
                    bridge.announce_mic_setup_edit();
                }
            });
            level_row(ui, bridge, "setup.room.level", &params.room_level, labels);
        }

        // Bleed.
        sub_heading(ui, "Bleed");
        if !catalog.has_bleed() {
            ui.label(theme::hint_text("This kit has no bleed recordings."));
        } else {
            ui.horizontal(|ui| {
                controls::row_label(ui, "", LABEL_W);
                if controls::segmented(ui, bridge, "setup.bleed.on", &params.bleed_on, BANK_ON_LABELS) {
                    bridge.wake_articulation_watcher();
                }
            });
            level_row(ui, bridge, "setup.bleed.level", &params.bleed_level, labels);
            let kit = bridge.kit_pads.current();
            for source in &catalog.bleed {
                let mic = source
                    .setups
                    .first()
                    .map(|k| catalog.label(k))
                    .unwrap_or_else(|| source.position.clone());
                let pads: Vec<&str> = source.pads.iter().map(|&p| kit.pads[p].name.as_str()).collect();
                let l = ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!("{mic} → heard on {}", pads.join(", ")))
                            .color(theme::TEXT_3)
                            .size(10.0),
                    )
                    .wrap(),
                );
                probe(ui, format_args!("setup.bleed.source.{}", source.position), l.rect);
            }
        }
    });
    probe(ui, "setup.banks", shown.response.rect);
}

fn sub_heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(2.0);
    ui.label(egui::RichText::new(text).color(theme::TEXT_2).size(11.0).strong());
}

fn level_row(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    name: &str,
    param: &resonance_plugin::FloatParam,
    labels: &mut Labels,
) {
    ui.horizontal(|ui| {
        controls::row_label(ui, "Level", LABEL_W);
        let w = super::body_width(ui, 0.0);
        controls::fader(ui, bridge, name, param, labels.of(param), w, false);
    });
}

fn draw_streaming(ui: &mut egui::Ui, bridge: &KitBridge) {
    use std::sync::atomic::Ordering;
    let shown = controls::card().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        controls::heading(ui, "STREAMING");
        ui.horizontal(|ui| {
            controls::row_label(ui, "Preload", LABEL_W);
            if controls::segmented(
                ui,
                bridge,
                "setup.preload",
                &bridge.params.stream_preload,
                crate::stream::PRELOAD_LABELS,
            ) {
                // A preload change reloads the kit: the watcher does it.
                bridge.wake_articulation_watcher();
            }
        });
        ui.label(theme::hint_text(
            "How much of each sample stays in memory; the rest streams from disk. \
             Off keeps every sample whole.",
        ));
        let bytes = bridge.kit_bytes.load(Ordering::Relaxed);
        let ring = bridge.stream_ring_bytes.load(Ordering::Relaxed);
        let underruns = bridge.stream_underruns.load(Ordering::Relaxed);
        fact(ui, "setup.memory", "In memory", &if bytes == 0 { "—".into() } else { format_bytes(bytes) });
        fact(ui, "setup.rings", "Stream rings", &format_bytes(ring));
        let l = ui.label(
            egui::RichText::new(format!(
                "{underruns} underrun{}",
                if underruns == 1 { "" } else { "s" }
            ))
            .color(if underruns > 0 { theme::WARN } else { theme::TEXT_3 })
            .size(10.5),
        );
        probe(ui, "setup.underruns", l.rect);
    });
    probe(ui, "setup.streaming", shown.response.rect);
}

fn fact(ui: &mut egui::Ui, name: &str, label: &str, value: &str) {
    ui.horizontal(|ui| {
        controls::row_label(ui, label, 92.0);
        let l = ui.add(
            egui::Label::new(egui::RichText::new(value).color(theme::TEXT_2).size(10.5)).wrap(),
        );
        probe(ui, name, l.rect);
    });
}

fn draw_kit_facts(ui: &mut egui::Ui, bridge: &KitBridge, entry: Option<&Entry>) {
    let shown = controls::card().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
        controls::heading(ui, "THE KIT");
        let Some(e) = entry else {
            // Not a library kit: the built-in one, or a kit loaded from a
            // folder the library does not hold.
            let loaded = match &*bridge.kit_status.lock() {
                crate::kit_loader::KitStatus::Loaded { name, num_pads, .. } => {
                    Some((name.clone(), *num_pads))
                }
                _ => None,
            };
            match (bridge.kit_path.lock().clone(), loaded) {
                (Some(path), Some((_, pads))) => {
                    fact(ui, "kit.fact.source", "Source", "not in the library");
                    fact(ui, "kit.fact.pieces", "Pads", &pads.to_string());
                    let folder = path.parent().unwrap_or(&path);
                    location(ui, folder);
                }
                _ => {
                    ui.label(theme::hint_text(
                        "The built-in kit: one sample per pad, no mic setups. Open \
                         the Library to load a kit.",
                    ));
                }
            }
            return;
        };
        fact(ui, "kit.fact.source", "Source", super::library_panel::source_text(e.source));
        let pieces: Vec<&str> = e.pieces.iter().map(|p| p.name.as_str()).collect();
        fact(ui, "kit.fact.pieces", "Pieces", &format!("{} ({})", pieces.len(), pieces.join(", ")));
        fact(ui, "kit.fact.mics", "Mic setups", &e.mic_setups.len().to_string());
        fact(
            ui,
            "kit.fact.layers",
            "Layers / RR",
            &format!("{} velocity layers · {} round robin", e.layers_max, e.rr_max),
        );
        fact(ui, "kit.fact.samples", "Samples", &e.sample_count.to_string());
        fact(
            ui,
            "kit.fact.size",
            "Size",
            &e.size_bytes.map(format_bytes).unwrap_or_else(|| "not measured yet".into()),
        );
        location(ui, &e.dir);
    });
    probe(ui, "setup.kit", shown.response.rect);
}

/// Where the kit's folder is: the directory holding it, with the full
/// path on hover. The folder's own name is (usually) the kit's, which the
/// header already shows.
fn location(ui: &mut egui::Ui, folder: &std::path::Path) {
    let parent = folder
        .parent()
        .map_or_else(|| folder.display().to_string(), |p| p.display().to_string());
    ui.horizontal(|ui| {
        controls::row_label(ui, "Location", 92.0);
        let l = ui
            .add(egui::Label::new(egui::RichText::new(parent).color(theme::TEXT_2).size(10.5)).wrap())
            .on_hover_text(folder.display().to_string());
        probe(ui, "kit.fact.path", l.rect);
    });
}

/// Per pad: note (read-only — editable later, K10), output, choke.
fn draw_pad_table(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    let kit = app.bridge.kit_pads.current();
    let dim_ports = (app.params.output_mode.value() != OUTPUT_MODE_MULTI)
        .then_some(controls::STEREO_OUTPUT_WHY);
    let shown = controls::card().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.set_min_height(ui.available_height());
        controls::heading(ui, "PADS");
        let w = ui.available_width();
        let cols = [w * 0.32, w * 0.18, w * 0.25, w * 0.25];
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (title, cw) in ["PAD", "NOTE", "OUTPUT", "CHOKE"].iter().zip(cols) {
                let (r, _) = ui.allocate_exact_size(egui::vec2(cw, 14.0), egui::Sense::hover());
                ui.painter_at(r).text(
                    r.left_center(),
                    egui::Align2::LEFT_CENTER,
                    title,
                    egui::FontId::proportional(9.5),
                    theme::TEXT_3,
                );
            }
        });
        egui::ScrollArea::vertical()
            .id_salt("setup_pad_table")
            .auto_shrink([false, false])
            .show_rows(ui, ROW_H, NUM_PADS, |ui, range| {
                for pad in range {
                    let params = &app.params.pads[pad];
                    let kit_pad = &kit.pads[pad];
                    let row = ui.horizontal(|ui| {
                        ui.set_height(ROW_H - 4.0);
                        ui.spacing_mut().item_spacing.x = 0.0;
                        cell_text(ui, cols[0], &kit_pad.name, if kit_pad.present { theme::TEXT_2 } else { theme::TEXT_4 });
                        cell_text(ui, cols[1], pad_grid::note_label(pad), theme::TEXT_3);
                        let current = params.output.value();
                        let kit_port = OUTPUT_PORT_NAMES[kit_pad.output_group(pad) as usize];
                        if let Some(v) = controls::combo_dimmed(
                            ui,
                            &format!("setup.row.{pad}.output"),
                            cols[2] - 8.0,
                            &output_text(current, kit_port),
                            current,
                            (OUTPUT_KIT..=OUTPUT_CHOICE_LABELS.len() as i32 - 1)
                                .map(|v| (v, output_text(v, kit_port))),
                            dim_ports,
                        ) {
                            params.output.set_value(v);
                            app.bridge.announce_param_edit(params.output.id());
                        }
                        ui.add_space(8.0);
                        let current = params.choke.value();
                        let kit_group = kit_pad.choke_group(pad);
                        if let Some(v) = controls::combo(
                            ui,
                            &format!("setup.row.{pad}.choke"),
                            cols[3] - 8.0,
                            &choke_text(current, kit_group),
                            current,
                            (CHOKE_KIT..=MAX_CHOKE_GROUP).map(|v| (v, choke_text(v, kit_group))),
                        ) {
                            params.choke.set_value(v);
                            app.bridge.announce_param_edit(params.choke.id());
                        }
                    });
                    probe(ui, format_args!("setup.row.{pad}"), row.response.rect);
                }
            });
    });
    probe(ui, "setup.table", shown.response.rect);
}

fn cell_text(ui: &mut egui::Ui, width: f32, text: &str, color: egui::Color32) {
    controls::text_cell(ui, text, color, width, ROW_H - 4.0, egui::Sense::hover());
}
