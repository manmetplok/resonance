//! The selected pad's inspector (§6.2): **one control per param**, and a
//! control only for what this pad has.
//!
//! Top to bottom:
//!
//! - the pad's name and note, ▶ (play it at the velocity beside it)
//!   and Mute — there is no Solo: the plugin has no solo param, and a
//!   switch that only the editor understood would be a fake;
//! - the waveform of the take the pad **last played**, with its layer,
//!   take and velocity ([`crate::last_hit`]); before the first hit, the
//!   loudest layer's first take, labelled as such;
//! - Level, Pan, Tune, Hold, Decay, Start;
//! - the articulation chips with the kit's labels, only when the kit
//!   pairs the pad with an alternate piece;
//! - MICS: a picker (`brand · mic`, not a raw key) and a dB trim per close
//!   mic the pad *has*, then the overhead, bleed and room trims for the
//!   banks it has — no placeholder for a mic it lacks;
//! - ROUTING: output port and choke group.
//!
//! The global overhead setup picker that used to sit here (a kit-wide
//! setting in a per-pad card) is on the Setup tab now.

use plugin_gui_core::{egui, widgets};
use resonance_plugin::param::Param;

use crate::drum_map::PAD_MAPPINGS;
use crate::kit::OUTPUT_PORT_NAMES;
use crate::last_hit::LastHit;
use crate::mic_catalog::ManifestMicCatalog;
use crate::params::{
    choke_label, MicSlot, PadParams, CHOKE_KIT, MAX_CHOKE_GROUP, OUTPUT_CHOICE_LABELS, OUTPUT_KIT,
};
use crate::sample_info::PadSampleInfo;
use crate::KitBridge;

use super::controls::{self, Labels, PAN_TAG};
use super::{pad_grid, probe, probed, reload_kit, theme};

/// What the inspector keeps between frames.
pub(crate) struct InspectorState<'a> {
    /// ▶'s velocity, MIDI 1..=127.
    pub audition_velocity: &'a mut u8,
    pub labels: &'a mut Labels,
}

/// What the inspector says about a pad whose piece the kit lacks (D7).
pub(crate) const NOT_IN_KIT: &str = "Not in this kit";

/// Width of a MICS row's label column.
const MIC_LABEL_W: f32 = 76.0;

/// Draw the inspector for `pad`.
pub(crate) fn draw(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    catalog: &ManifestMicCatalog,
    pad: usize,
    state: &mut InspectorState<'_>,
) {
    let params = &bridge.params.pads[pad];
    let kit = bridge.kit_pads.current();
    let kit_pad = &kit.pads[pad];
    // A snapshot for the price of a refcount: the loader swaps the whole
    // list on each load.
    let samples = bridge.pad_samples.lock().clone();
    let info = samples.get(pad).and_then(Option::as_ref);
    let hit = bridge.last_hits.pad(pad);

    let shown = controls::card().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);

        draw_head(ui, bridge, pad, kit_pad, params, state);
        draw_sample_stage(ui, info, hit, kit_pad.present, catalog);
        draw_knobs(ui, bridge, params, state.labels);

        if let Some(articulation) = &kit_pad.articulation {
            draw_articulations(ui, bridge, params, articulation);
        }

        draw_mics(ui, bridge, catalog, pad, params, info, state.labels);
        draw_routing(ui, bridge, pad, kit_pad, params);
    });
    probe(ui, "inspector", shown.response.rect);
}

fn draw_head(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    pad: usize,
    kit_pad: &crate::pad_map::KitPad,
    params: &PadParams,
    state: &mut InspectorState<'_>,
) {
    let note = PAD_MAPPINGS[pad].note;
    egui::Sides::new().shrink_left().show(
        ui,
        |ui| {
            let name = ui.add(
                egui::Label::new(
                    egui::RichText::new(kit_pad.name.as_str())
                        .italics()
                        .color(if kit_pad.present {
                            theme::TEXT_1
                        } else {
                            theme::TEXT_4
                        })
                        .size(19.0),
                )
                .truncate(),
            );
            probe(ui, "inspector.name", name.rect);
            let meta = ui.add(
                egui::Label::new(
                    egui::RichText::new(pad_grid::note_label(pad))
                        .color(theme::TEXT_3)
                        .size(11.0)
                        .monospace(),
                )
                .truncate(),
            );
            probe(ui, "inspector.note", meta.rect);
        },
        |ui| {
            // ▶ plays the pad through the same note-on a MIDI hit takes
            // (the bridge's audition queue, ba todo #1328), at the
            // velocity to its right.
            let mut velocity = *state.audition_velocity as i32;
            let drag = ui
                .add(
                    egui::DragValue::new(&mut velocity)
                        .range(1..=127)
                        .prefix("v")
                        .speed(0.5),
                )
                .on_hover_text("The velocity ▶ plays at (drag, or type 1–127)");
            probe(ui, "inspector.velocity", drag.rect);
            *state.audition_velocity = velocity.clamp(1, 127) as u8;
            let play = ui
                .add_enabled(
                    kit_pad.present,
                    egui::Button::new(egui::RichText::new("▶").color(theme::TEXT_1).size(12.0))
                        .min_size(egui::vec2(28.0, 22.0)),
                )
                .on_hover_text("Play this pad once, at the velocity beside it")
                .on_disabled_hover_text(NOT_IN_KIT);
            probe(ui, "inspector.play", play.rect);
            if play.clicked() {
                bridge.audition_at(note, *state.audition_velocity as f32 / 127.0);
            }
        },
    );
    ui.horizontal(|ui| {
        controls::toggle(ui, bridge, "inspector.mute", "Mute", &params.mute);
        if !kit_pad.present {
            let shown = ui
                .label(egui::RichText::new(NOT_IN_KIT).color(theme::WARM).size(11.0))
                .on_hover_text(
                    "The selected kit has no recording for this pad, so it is \
                     silent. The built-in samples play only when no kit is \
                     selected.",
                );
            probe(ui, "inspector.not_in_kit", shown.rect);
        }
    });
}

/// The waveform of the take the pad last played.
///
/// Everything drawn here comes from [`PadSampleInfo`], measured from the
/// decoded takes by whoever built the kit, and from the sampler's last
/// hit. With no info for the pad — no kit loaded yet, or no recording in
/// this kit — the stage says so instead of drawing an invented shape (ba
/// todo #1276).
fn draw_sample_stage(
    ui: &mut egui::Ui,
    info: Option<&PadSampleInfo>,
    hit: Option<LastHit>,
    present: bool,
    catalog: &ManifestMicCatalog,
) {
    let frame = egui::Frame::default()
        .fill(theme::BG_1)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(8.0);
    let shown = frame.show(ui, |ui| {
        let w = ui.available_width();
        let h = 96.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
        let p = ui.painter_at(rect);
        let mid_y = rect.center().y;
        p.line_segment(
            [egui::pos2(rect.left(), mid_y), egui::pos2(rect.right(), mid_y)],
            egui::Stroke::new(0.5, theme::LINE),
        );

        let Some(info) = info else {
            p.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                if present {
                    "no sample detail yet — load a kit"
                } else {
                    "Not in this kit — this pad is silent"
                },
                egui::FontId::proportional(11.0),
                theme::TEXT_4,
            );
            return;
        };

        // The take the last hit played, when this kit has it; else the
        // full-velocity one, said to be that.
        let played = hit.and_then(|h| info.take(h.layer, h.take).map(|t| (h, t)));
        let (envelope, frames, readout) = match played {
            Some((h, take)) => (
                &take.envelope,
                take.frames,
                format!("last hit v{} · {}", h.velocity, h.cell_text()),
            ),
            None => (
                &info.envelope,
                info.frames,
                format!("not played yet — showing the loudest: {}", info.layer_text()),
            ),
        };

        let half = h * 0.36;
        let buckets = envelope.len();
        if buckets > 0 {
            let bucket_w = w / buckets as f32;
            let color = if played.is_some() {
                theme::ACCENT_SOFT
            } else {
                theme::TEXT_3
            };
            for (i, (lo, hi)) in envelope.iter().enumerate() {
                let x = rect.left() + (i as f32 + 0.5) * bucket_w;
                let y_hi = mid_y - hi.clamp(-1.0, 1.0) * half;
                let y_lo = mid_y - lo.clamp(-1.0, 1.0) * half;
                p.line_segment(
                    [egui::pos2(x, y_hi), egui::pos2(x, y_lo.max(y_hi + 0.5))],
                    egui::Stroke::new(bucket_w.max(0.9), color),
                );
            }
        }

        // Which mic the shown take is from (`brand · mic`), its length,
        // and what was played.
        let source = if info.setup_key.is_empty() {
            "built-in sample".to_string()
        } else {
            catalog.label(&info.setup_key)
        };
        p.text(
            rect.left_top() + egui::vec2(8.0, 6.0),
            egui::Align2::LEFT_TOP,
            source,
            egui::FontId::proportional(10.0),
            theme::TEXT_3,
        );
        if info.sample_rate > 0.0 {
            p.text(
                rect.right_top() + egui::vec2(-8.0, 6.0),
                egui::Align2::RIGHT_TOP,
                format!("{:.0} ms", frames as f32 / info.sample_rate * 1000.0),
                egui::FontId::monospace(9.5),
                theme::TEXT_3,
            );
        }
        let r = p.text(
            rect.left_bottom() + egui::vec2(8.0, -6.0),
            egui::Align2::LEFT_BOTTOM,
            readout,
            egui::FontId::monospace(9.5),
            if played.is_some() {
                theme::TEXT_2
            } else {
                theme::TEXT_3
            },
        );
        probe(ui, "inspector.hit", r);
    });
    probe(ui, "inspector.sample", shown.response.rect);
}

/// Level, Pan, Tune, Hold, Decay, Start — wrapping onto a second row when
/// the column is narrow.
fn draw_knobs(ui: &mut egui::Ui, bridge: &KitBridge, params: &PadParams, labels: &mut Labels) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(10.0, 6.0);
        let knobs: [(&str, &str, &resonance_plugin::FloatParam, bool); 6] = [
            ("knob.level", "Level", &params.volume, false),
            ("knob.pan", "Pan", &params.pan, true),
            ("knob.tune", "Tune", &params.tune, true),
            ("knob.hold", "Hold", &params.hold, false),
            ("knob.decay", "Decay", &params.decay, false),
            ("knob.start", "Start", &params.start, false),
        ];
        for (name, caption, param, bipolar) in knobs {
            let text = if name == "knob.pan" {
                labels.with(param, PAN_TAG, controls::pan_text)
            } else {
                labels.of(param)
            };
            controls::knob(ui, bridge, name, caption, param, text, bipolar);
        }
    });
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
    params: &PadParams,
    articulation: &crate::pad_map::PadArticulation,
) {
    ui.horizontal(|ui| {
        controls::heading(ui, "ARTICULATION");
        let current = params.articulation.value();
        let chips = [
            (crate::articulation::ARTICULATION_PRIMARY, &articulation.primary_label),
            (crate::articulation::ARTICULATION_ALT, &articulation.alt_label),
        ];
        for (index, label) in chips {
            let clicked = probed(ui, &format!("articulation.{index}"), |ui| {
                widgets::chip_button(ui, label, index == current)
            });
            if clicked && index != current {
                params.articulation.set_value(index);
                // A user's edit: one undoable change in the host, not a
                // value the host only follows.
                bridge.announce_param_edit(params.articulation.id());
                // The parameter is the source of truth; the reload is the
                // watcher's job. Ping it so the click lands now instead of
                // at its next poll.
                bridge.wake_articulation_watcher();
            }
        }
        ui.label(
            egui::RichText::new(articulation.label.as_str())
                .color(theme::TEXT_3)
                .size(10.0),
        )
        .on_hover_text("Reloads the pad's samples. Automatable as \"Pad Articulation\".");
    });
}

/// The pad's mics: a picker and a trim for each close mic it has, then a
/// trim for its overheads, bleed and room — each only when the pad holds
/// that bank.
fn draw_mics(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    catalog: &ManifestMicCatalog,
    pad: usize,
    params: &PadParams,
    info: Option<&PadSampleInfo>,
    labels: &mut Labels,
) {
    let Some(banks) = info.map(|i| &i.banks) else {
        return;
    };
    controls::heading(ui, "MICS");
    let mut pick: Option<(String, String)> = None;
    for (bank, (position, setup)) in banks.close.iter().enumerate().take(2) {
        let built_in = setup.is_empty();
        ui.horizontal(|ui| {
            controls::row_label(
                ui,
                if built_in { "Sample" } else { position_name(position) },
                MIC_LABEL_W,
            );
            let choices = catalog.close_setups(position);
            if choices.len() > 1 {
                let w = super::body_width(ui, 0.0);
                if let Some(key) = controls::combo(
                    ui,
                    &format!("mic.{bank}"),
                    w,
                    &catalog.label(setup),
                    setup.as_str(),
                    choices.iter().map(|k| (k.as_str(), catalog.label(k))),
                ) {
                    pick = Some((position.clone(), key.to_string()));
                }
            } else {
                let text = if built_in {
                    "built-in".to_string()
                } else {
                    catalog.label(setup)
                };
                let l = ui.add(
                    egui::Label::new(egui::RichText::new(text).color(theme::TEXT_2).size(11.0))
                        .truncate(),
                );
                probe(ui, format!("mic.{bank}"), l.rect);
            }
        });
        trim_row(ui, bridge, &format!("trim.mic{}", bank + 1), "", params.trim(MicSlot::close(bank)), labels);
    }
    if let Some((position, key)) = pick {
        bridge.pad_choices.lock()[pad]
            .close_setups
            .insert(position, key);
        // A mic change reloads that pad only (E4).
        reload_kit(bridge);
    }
    if banks.overhead {
        trim_row(ui, bridge, "trim.oh", "Overheads", params.trim(MicSlot::Overhead), labels);
    }
    if banks.bleed {
        trim_row(ui, bridge, "trim.bleed", "Bleed", params.trim(MicSlot::Bleed), labels);
    }
    if banks.room {
        trim_row(ui, bridge, "trim.room", "Room", params.trim(MicSlot::Room), labels);
    }
}

/// A trim fader under (or beside) its label.
fn trim_row(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    name: &str,
    label: &str,
    param: &resonance_plugin::FloatParam,
    labels: &mut Labels,
) {
    ui.horizontal(|ui| {
        controls::row_label(ui, label, MIC_LABEL_W);
        let w = super::body_width(ui, 0.0);
        controls::fader(ui, bridge, name, param, labels.of(param), w, false);
    });
}

/// Output port and choke group.
fn draw_routing(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    pad: usize,
    kit_pad: &crate::pad_map::KitPad,
    params: &PadParams,
) {
    controls::heading(ui, "ROUTING");
    let multi = bridge.params.output_mode.value() == crate::params::OUTPUT_MODE_MULTI;
    ui.horizontal(|ui| {
        controls::row_label(ui, "Output", MIC_LABEL_W);
        let current = params.output.value();
        let kit_port = OUTPUT_PORT_NAMES[kit_pad.output_group(pad) as usize];
        if let Some(v) = controls::combo(
            ui,
            "routing.output",
            140.0,
            &output_text(current, kit_port),
            current,
            (OUTPUT_KIT..=OUTPUT_CHOICE_LABELS.len() as i32 - 1).map(|v| (v, output_text(v, kit_port))),
        ) {
            params.output.set_value(v);
            bridge.announce_param_edit(params.output.id());
        }
        if !multi {
            let l = ui
                .add(egui::Label::new(theme::hint_text("in Stereo, all on Main")).truncate())
                .on_hover_text(
                    "Output Mode is Stereo (Mix tab): every pad plays on Main. The \
                     pad's port applies in Multi.",
                );
            probe(ui, "routing.output.stereo", l.rect);
        }
    });
    ui.horizontal(|ui| {
        controls::row_label(ui, "Choke", MIC_LABEL_W);
        let current = params.choke.value();
        let kit_group = kit_pad.choke_group(pad);
        if let Some(v) = controls::combo(
            ui,
            "routing.choke",
            140.0,
            &choke_text(current, kit_group),
            current,
            (CHOKE_KIT..=MAX_CHOKE_GROUP).map(|v| (v, choke_text(v, kit_group))),
        ) {
            params.choke.set_value(v);
            bridge.announce_param_edit(params.choke.id());
        }
    });
}

/// `pad_N_output` as a choice reads: `Kit (Snare)`, `Main`, …
pub(crate) fn output_text(value: i32, kit_port: &str) -> String {
    if value == OUTPUT_KIT {
        format!("Kit ({kit_port})")
    } else {
        OUTPUT_CHOICE_LABELS
            .get(value.max(0) as usize)
            .copied()
            .unwrap_or("?")
            .to_string()
    }
}

/// `pad_N_choke` as a choice reads: `Kit (group 1)`, `None`, `Group 3`.
pub(crate) fn choke_text(value: i32, kit_group: Option<u8>) -> String {
    if value == CHOKE_KIT {
        match kit_group {
            Some(g) => format!("Kit (group {g})"),
            None => "Kit (none)".to_string(),
        }
    } else {
        choke_label(value)
    }
}

/// A close-mic position as a person reads it.
fn position_name(position: &str) -> &str {
    match position {
        "KickIn" => "Kick In",
        "KickOut" => "Kick Out",
        "SNTop" => "Snare Top",
        "SNBtm" => "Snare Btm",
        "Hat" => "Hi-Hat",
        "Tom01" => "Tom 1",
        "Tom02" => "Tom 2",
        "TomFloor" => "Floor Tom",
        other => other,
    }
}
