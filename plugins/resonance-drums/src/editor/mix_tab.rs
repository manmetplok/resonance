//! The Mix tab (§6.3): the mixer view of the kit.
//!
//! - OUTPUTS: a strip per output port with its meter (the sampler's
//!   per-port block peak) and the pads that play on it, and the master
//!   level beside them. In Stereo mode the six ports beside Main are
//!   silent, and their strips say so.
//! - PADS: one row per pad — level, pan, mute, output — virtualised
//!   (`ScrollArea::show_rows`), so only the visible rows are laid out.
//! - GLOBAL: polyphony, velocity curve, velocity humanize, round robin,
//!   output mode.
//!
//! There is no release-time control: the plugin has no such param (the
//! release fades are fixed, E2), and a control that moved nothing would
//! be a fake.

use plugin_gui_core::egui;
use resonance_plugin::param::Param;

use crate::drum_map::NUM_PADS;
use crate::kit::{NUM_OUTPUT_PORTS, OUTPUT_PORT_NAMES};
use crate::params::{
    port_of_output_choice, OUTPUT_CHOICE_LABELS, OUTPUT_KIT, OUTPUT_MODE_LABELS,
    OUTPUT_MODE_MULTI, ROUND_ROBIN_LABELS,
};

use super::app::{column, DrumsEditorApp};
use super::controls::{self, PAN_TAG};
use super::pad_inspector::output_text;
use super::{probe, theme};

/// Width of the GLOBAL column.
const GLOBAL_W: f32 = 270.0;
const GAP: f32 = 12.0;
/// Height of the OUTPUTS card's strips.
const STRIP_H: f32 = 112.0;
/// One pad-table row.
const ROW_H: f32 = 24.0;

pub(super) fn draw(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    let avail = ui.available_size();
    let global_w = GLOBAL_W.min(avail.x * 0.4);
    let left_w = (avail.x - global_w - GAP).max(0.0);
    let ports = app.tick_port_meters();

    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(GAP, 0.0);
        column(ui, egui::vec2(left_w, avail.y), |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
            draw_outputs(ui, app, ports);
            draw_pad_table(ui, app);
        });
        column(ui, egui::vec2(global_w, avail.y), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("mix_global_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| draw_global(ui, app));
        });
    });
}

/// Which port pad `pad` plays its close mics on, as the sampler would in
/// Multi mode: its `pad_N_output`, else the kit's port for it.
fn pad_port(app: &DrumsEditorApp, kit: &crate::pad_map::KitPads, pad: usize) -> usize {
    port_of_output_choice(app.params.pads[pad].output.value())
        .unwrap_or_else(|| kit.pads[pad].output_group(pad) as usize)
}

fn draw_outputs(ui: &mut egui::Ui, app: &mut DrumsEditorApp, ports: [f32; NUM_OUTPUT_PORTS]) {
    let multi = app.params.output_mode.value() == OUTPUT_MODE_MULTI;
    let kit = app.bridge.kit_pads.current();
    let mut counts = [0usize; NUM_OUTPUT_PORTS];
    for pad in 0..NUM_PADS {
        if kit.pads[pad].present {
            counts[pad_port(app, &kit, pad)] += 1;
        }
    }
    let shown = controls::card().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            controls::heading(ui, "OUTPUTS");
            ui.label(theme::hint_text(if multi {
                "Multi: each port is its own stereo out"
            } else {
                "Stereo: the whole kit plays on Main"
            }));
        });
        ui.add_space(4.0);
        let master_w = 74.0;
        let strips_w = super::body_width(ui, master_w + 8.0);
        let strip_w = (strips_w / NUM_OUTPUT_PORTS as f32).max(30.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
            for port in 0..NUM_OUTPUT_PORTS {
                let live = multi || port == crate::kit::MAIN_PORT_INDEX;
                let rect = draw_strip(ui, port, strip_w, ports[port], live, counts[port], multi);
                probe(ui, format!("mix.strip.{port}"), rect);
            }
            ui.add_space(8.0);
            draw_master(ui, app, master_w);
        });
    });
    probe(ui, "mix.outputs", shown.response.rect);
}

/// One port's strip: name, meter, peak and the pads routed to it.
fn draw_strip(
    ui: &mut egui::Ui,
    port: usize,
    width: f32,
    level: f32,
    live: bool,
    pads: usize,
    multi: bool,
) -> egui::Rect {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, STRIP_H), egui::Sense::hover());
    let p = ui.painter_at(rect);
    let text_color = if live { theme::TEXT_2 } else { theme::TEXT_4 };
    p.text(
        egui::pos2(rect.center().x, rect.top()),
        egui::Align2::CENTER_TOP,
        OUTPUT_PORT_NAMES[port],
        egui::FontId::proportional(10.0),
        text_color,
    );
    let meter = egui::Rect::from_min_max(
        egui::pos2(rect.center().x - 5.0, rect.top() + 16.0),
        egui::pos2(rect.center().x + 5.0, rect.bottom() - 30.0),
    );
    p.rect_filled(meter, 2.0, theme::BG_1);
    p.rect_stroke(meter, 2.0, egui::Stroke::new(1.0, theme::LINE), egui::StrokeKind::Inside);
    let fill = meter_fraction(level) * meter.height();
    if fill > 0.0 {
        let bar = egui::Rect::from_min_max(egui::pos2(meter.left(), meter.bottom() - fill), meter.max);
        p.rect_filled(bar, 2.0, if level >= 1.0 { theme::BAD } else { theme::GOOD });
    }
    p.text(
        egui::pos2(rect.center().x, rect.bottom() - 26.0),
        egui::Align2::CENTER_TOP,
        db_text(level),
        egui::FontId::monospace(9.0),
        text_color,
    );
    // Pads routed here (close mics); Overhead carries the overheads, and
    // in Stereo nothing but Main sounds.
    let sub = if !live {
        "silent".to_string()
    } else if !multi {
        "all pads".to_string()
    } else if port == crate::kit::OVERHEAD_PORT_INDEX {
        "OH · amb".to_string()
    } else {
        format!("{pads} pad{}", if pads == 1 { "" } else { "s" })
    };
    p.text(
        egui::pos2(rect.center().x, rect.bottom() - 13.0),
        egui::Align2::CENTER_TOP,
        sub,
        egui::FontId::proportional(9.0),
        theme::TEXT_3,
    );
    response.on_hover_text(if live {
        "This port's peak level"
    } else {
        "Silent in Stereo mode: switch Output Mode to Multi to use this port"
    });
    rect
}

/// The master level: a knob over `master_level`, beside the strips.
fn draw_master(ui: &mut egui::Ui, app: &mut DrumsEditorApp, width: f32) {
    let param = &app.params.master_volume;
    ui.allocate_ui_with_layout(
        egui::vec2(width, STRIP_H),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 4.0);
            ui.label(egui::RichText::new("Master").color(theme::TEXT_1).size(10.0));
            let text = app.labels.of(param);
            controls::knob(ui, &app.bridge, "mix.master", "Level", param, text, false);
        },
    );
}

/// The per-pad table: name, level, pan, mute, output. Only the rows in
/// view are laid out (`show_rows`).
fn draw_pad_table(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    let kit = app.bridge.kit_pads.current();
    let shown = controls::card().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 2.0);
        let w = ui.available_width();
        // name | level | pan | mute | output
        let cols = [w * 0.19, w * 0.34, w * 0.25, 26.0, (w * 0.22 - 26.0).max(40.0)];
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (title, cw) in ["PAD", "LEVEL", "PAN", "", "OUTPUT"].iter().zip(cols) {
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
            .id_salt("mix_pad_table")
            .auto_shrink([false, false])
            .show_rows(ui, ROW_H, NUM_PADS, |ui, range| {
                for pad in range {
                    draw_pad_row(ui, app, &kit, pad, cols);
                }
            });
    });
    probe(ui, "mix.table", shown.response.rect);
}

fn draw_pad_row(
    ui: &mut egui::Ui,
    app: &mut DrumsEditorApp,
    kit: &crate::pad_map::KitPads,
    pad: usize,
    cols: [f32; 5],
) {
    let params = &app.params.pads[pad];
    let present = kit.pads[pad].present;
    let row = ui.horizontal(|ui| {
        ui.set_height(ROW_H - 4.0);
        ui.spacing_mut().item_spacing.x = 0.0;
        // Name: click selects the pad (and shows it on the Pads tab).
        let color = match (present, app.selected_pad == pad) {
            (false, _) => theme::TEXT_4,
            (true, true) => theme::ACCENT_SOFT,
            (true, false) => theme::TEXT_2,
        };
        let name = controls::text_cell(
            ui,
            &kit.pads[pad].name,
            color,
            cols[0],
            ROW_H - 4.0,
            egui::Sense::click(),
        )
        .on_hover_text("Select this pad");
        if name.clicked() {
            app.selected_pad = pad;
        }
        let r = name.rect;
        probe(ui, format!("mix.row.{pad}.name"), r);

        let text = app.labels.of(&params.volume);
        let w = cols[1] - 6.0;
        controls::table_fader(ui, &app.bridge, &format!("mix.row.{pad}.level"), &params.volume, text, w, false);
        ui.add_space(6.0);
        let text = app.labels.with(&params.pan, PAN_TAG, controls::pan_text);
        let w = cols[2] - 6.0;
        controls::table_fader(ui, &app.bridge, &format!("mix.row.{pad}.pan"), &params.pan, text, w, true);
        ui.add_space(6.0);
        ui.allocate_ui_with_layout(
            egui::vec2(cols[3], ROW_H - 4.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| controls::toggle(ui, &app.bridge, &format!("mix.row.{pad}.mute"), "M", &params.mute),
        );
        let current = params.output.value();
        let kit_port = OUTPUT_PORT_NAMES[kit.pads[pad].output_group(pad) as usize];
        if let Some(v) = controls::combo(
            ui,
            &format!("mix.row.{pad}.output"),
            cols[4] - 8.0,
            &output_text(current, kit_port),
            current,
            (OUTPUT_KIT..=OUTPUT_CHOICE_LABELS.len() as i32 - 1).map(|v| (v, output_text(v, kit_port))),
        ) {
            params.output.set_value(v);
            app.bridge.announce_param_edit(params.output.id());
        }
    });
    probe(ui, format!("mix.row.{pad}"), row.response.rect);
}

/// GLOBAL: how the kit plays.
fn draw_global(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    let shown = controls::card().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        controls::heading(ui, "GLOBAL");
        let w = ui.available_width();
        let (params, bridge, labels) = (&app.params, &app.bridge, &mut app.labels);

        caption(ui, "Polyphony", "voices sounding at once (a hit uses one per mic)");
        let text = labels.of(&params.polyphony);
        controls::int_fader(ui, bridge, "global.polyphony", &params.polyphony, text, w);

        caption(ui, "Velocity curve", "harder ← linear → softer");
        let text = labels.of(&params.velocity_curve);
        controls::fader(ui, bridge, "global.velocity_curve", &params.velocity_curve, text, w, true);

        caption(ui, "Velocity humanize", "random ± MIDI steps per hit");
        let text = labels.of(&params.velocity_humanize);
        controls::fader(
            ui,
            bridge,
            "global.velocity_humanize",
            &params.velocity_humanize,
            text,
            w,
            false,
        );

        caption(ui, "Round robin", "how a layer's takes are walked");
        controls::segmented(ui, bridge, "global.round_robin", &params.round_robin_mode, ROUND_ROBIN_LABELS);

        caption(ui, "Output mode", "");
        controls::segmented(ui, bridge, "global.output_mode", &params.output_mode, OUTPUT_MODE_LABELS);
        let explain = if params.output_mode.value() == OUTPUT_MODE_MULTI {
            "Multi: each pad plays on its output port, the overheads and \
             ambience on Overhead — route them to sub-tracks in the host."
        } else {
            "Stereo: the whole kit plays on Main; the other six ports are silent."
        };
        let l = ui.add(
            egui::Label::new(egui::RichText::new(explain).color(theme::TEXT_3).size(10.0)).wrap(),
        );
        probe(ui, "global.output_mode.explain", l.rect);
    });
    probe(ui, "mix.global", shown.response.rect);
}

/// A control's caption, with its hint on hover and (when it fits) beside.
fn caption(ui: &mut egui::Ui, label: &str, hint: &str) {
    ui.add_space(2.0);
    let r = ui
        .label(egui::RichText::new(label).color(theme::TEXT_2).size(10.5))
        .on_hover_text(hint);
    probe(ui, format!("caption.{label}"), r.rect);
}

/// -60 dBFS .. 0 dBFS → 0..1.
pub(crate) fn meter_fraction(peak: f32) -> f32 {
    if peak <= 0.0 {
        return 0.0;
    }
    ((20.0 * peak.log10() + 60.0) / 60.0).clamp(0.0, 1.0)
}

fn db_text(peak: f32) -> String {
    if peak <= 0.0 {
        "−∞".to_string()
    } else {
        format!("{:.1}", 20.0 * peak.log10())
    }
}
