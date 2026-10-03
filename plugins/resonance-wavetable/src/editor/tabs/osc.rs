//! OSC tab: wavetable viewer + osc selector + per-osc controls (with the
//! per-osc warp and the oscillator interaction), then unison, sub/noise and
//! global.

use resonance_plugin::editor_widgets;
use plugin_gui_core::{egui, widgets};

use crate::dsp::warp::{Warp, WarpMode};
use crate::dsp::wavetable::USER_WAVETABLE_INDEX;
use crate::editor::display_waves::{self, DisplayTable};
use crate::editor::theme;
use crate::editor::viz::{frame_strip, waveform};
use crate::editor::WavetableEditorApp;

use super::{choice_cycle, choice_segmented, float_knob, float_slider, int_knob, readout};

pub fn draw(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    ui.spacing_mut().item_spacing = egui::vec2(12.0, 10.0);

    // Two-column body: left = wave panel, right = params panel.
    ui.columns(2, |cols| {
        draw_osc_panel(&mut cols[0], app);
        draw_params_panel(&mut cols[1], app);
    });

    ui.add_space(2.0);

    // Bottom row: Unison (five knobs, so half the width) beside the Sub ·
    // Noise and Global cards, which share the other half.
    ui.columns(2, |cols| {
        draw_unison_card(&mut cols[0], app);
        cols[1].columns(2, |cols| {
            draw_sub_noise_card(&mut cols[0], app);
            draw_global_card(&mut cols[1], app);
        });
    });
}

fn panel_frame() -> egui::Frame {
    egui::Frame::default()
        .fill(theme::BG_2)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(theme::RADIUS_PANEL)
        .inner_margin(egui::Margin::same(12))
}

/// Helper: render `body` inside a panel frame that fills the column width.
fn panel<R>(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let avail_w = ui.available_width();
    let mut out = None;
    panel_frame().show(ui, |ui| {
        ui.set_min_width(avail_w - 24.0); // subtract inner margin*2
        out = Some(body(ui));
    });
    out.expect("body always runs")
}

fn panel_title(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .color(theme::TEXT_3)
            .size(10.5)
            .strong(),
    );
}

fn draw_osc_panel(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    panel(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(10.0, 8.0);

        // Header: Osc1/Osc2 segmented + balance.
        ui.horizontal(|ui| {
            let labels = ["Osc 1", "Osc 2"];
            if let Some(i) = widgets::segmented(ui, &labels, app.selected_osc) {
                app.selected_osc = i;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(readout(&app.params.osc_balance))
                        .monospace()
                        .color(theme::TEXT_1)
                        .size(11.0),
                );
                ui.add_space(8.0);
                float_slider(ui, 110.0, &app.params.osc_balance);
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("BALANCE")
                        .color(theme::TEXT_3)
                        .size(9.5)
                        .strong(),
                );
            });
        });

        let osc = app.selected_osc;
        // A clone of the Arc, so `app` stays free for the load bookkeeping.
        let params = app.params.clone();
        let (osc_params, warp_params, live_pos) = if osc == 0 {
            (&params.osc1, &params.osc1_warp, app.snapshot.osc1_position_live)
        } else {
            (&params.osc2, &params.osc2_warp, app.snapshot.osc2_position_live)
        };

        let user = app.user_tables.info(osc);
        // A "Load…" this editor asked for has finished: select the user
        // table if it landed. Selecting only then means a failed import
        // leaves the oscillator on whatever it was playing.
        if let Some(requested) = app.pending_user_load[osc] {
            if user.generation >= requested {
                if user.generation == requested && user.error.is_none() && user.is_loaded() {
                    // The user's own Load…, now landed: one host edit.
                    let p = &osc_params.wavetable;
                    editor_widgets::commit_plain(ui.ctx(), p, USER_WAVETABLE_INDEX as f64);
                }
                app.pending_user_load[osc] = None;
            }
        }

        let wt_idx = osc_params.wavetable.value() as usize;
        let position = osc_params.position.value();
        let table = DisplayTable::for_selection(wt_idx, &user);
        // The display runs the frame through the same phase map the DSP
        // reads with, so the drawn cycle is the warped one.
        let warp = Warp::resolve(
            WarpMode::from_int(warp_params.mode.value()),
            warp_params.amount.value(),
        );

        // Wave display.
        let avail = ui.available_width();
        let (_id, rect) = ui.allocate_space(egui::vec2(avail, 170.0));
        waveform::draw(ui, rect, &table, position, live_pos, &warp);

        // Frame strip.
        let (_id2, strip_rect) = ui.allocate_space(egui::vec2(avail, 24.0));
        frame_strip::draw(ui, strip_rect, &table, position);

        // Wavetable category row. ▶ reaches the user slot only once this
        // oscillator has a table to put there.
        let last = if user.is_loaded() {
            USER_WAVETABLE_INDEX
        } else {
            USER_WAVETABLE_INDEX - 1
        };
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Button::new(egui::RichText::new("◀").color(theme::TEXT_3).size(10.0))
                        .frame(false),
                )
                .clicked()
                && wt_idx > 0
            {
                let p = &osc_params.wavetable;
                editor_widgets::commit_plain(ui.ctx(), p, (wt_idx - 1) as f64);
            }
            ui.label(
                egui::RichText::new(display_waves::selection_name(wt_idx, &user))
                    .color(theme::TEXT_1)
                    .size(12.0)
                    .strong(),
            );
            if ui
                .add(
                    egui::Button::new(egui::RichText::new("▶").color(theme::TEXT_3).size(10.0))
                        .frame(false),
                )
                .clicked()
                && wt_idx < last
            {
                let p = &osc_params.wavetable;
                editor_widgets::commit_plain(ui.ctx(), p, (wt_idx + 1) as f64);
            }
            ui.add_space(6.0);
            if ui
                .add_enabled(
                    !user.loading,
                    egui::Button::new(egui::RichText::new("Load…").size(11.0)),
                )
                .on_hover_text("Import a WAV as this oscillator's wavetable")
                .clicked()
            {
                load_clicked(app, osc);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let frames = table.frame_count();
                let frame_idx = ((position * (frames.saturating_sub(1)).max(1) as f32)
                    .round() as usize)
                    .min(frames.saturating_sub(1));
                ui.label(
                    egui::RichText::new(format!("FRAME {} / {}", frame_idx + 1, frames))
                        .color(theme::TEXT_3)
                        .size(10.0)
                        .monospace(),
                );
            });
        });

        // User-table status: the loaded file, a load in flight, or why the
        // last one failed (including a project whose file has gone missing,
        // which plays bundled table 0 until it is re-imported).
        let status = if user.loading {
            Some(("Loading…".to_string(), theme::TEXT_3))
        } else if let Some(e) = &user.error {
            Some((format!("User table: {e}"), theme::WARN))
        } else if user.is_loaded() {
            Some((
                format!("User table: {} · {} frames", user.name, user.num_frames()),
                theme::TEXT_3,
            ))
        } else {
            None
        };
        if let Some((text, color)) = status {
            ui.label(egui::RichText::new(text).color(color).size(10.5));
        }
    });
}

/// "Load…": pick a WAV and import it into oscillator `osc` in the background.
fn load_clicked(app: &mut WavetableEditorApp, osc: usize) {
    // Sync rfd dialog on the UI thread — the Wayland runtime's editor
    // thread, or the AppKit main thread under the Cocoa runtime, where a
    // modal panel is the supported path and the runtime's reentrancy
    // guard skips nested paints (macos-editor-plan.md §3h). The import
    // itself runs on a loader thread; the table is selected once it lands.
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Wavetable (WAV)", &["wav"])
        .pick_file()
    else {
        return;
    };
    let generation = app
        .user_tables
        .request_file(osc, path.to_string_lossy().into_owned());
    app.pending_user_load[osc] = Some(generation);
}

fn draw_params_panel(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    panel(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);

        let (osc_params, warp_params) = if app.selected_osc == 0 {
            (&app.params.osc1, &app.params.osc1_warp)
        } else {
            (&app.params.osc2, &app.params.osc2_warp)
        };
        let title = if app.selected_osc == 0 { "Osc 1" } else { "Osc 2" };
        let wt_idx = osc_params.wavetable.value() as usize;
        let user = app.user_tables.info(app.selected_osc);
        let table = DisplayTable::for_selection(wt_idx, &user);

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(title)
                    .color(theme::TEXT_1)
                    .size(13.0)
                    .strong(),
            );
            ui.label(
                egui::RichText::new(format!(
                    "{} · {} frames",
                    display_waves::selection_name(wt_idx, &user),
                    table.frame_count()
                ))
                .color(theme::TEXT_3)
                .size(11.0),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let enabled = osc_params.enabled.value();
                let led_color = if enabled { theme::GOOD } else { theme::TEXT_4 };
                let frame = egui::Frame::default()
                    .fill(theme::BG_1)
                    .stroke(egui::Stroke::new(1.0, theme::LINE_2))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(9, 4));
                let resp = frame
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(
                                egui::vec2(8.0, 8.0),
                                egui::Sense::hover(),
                            );
                            ui.painter().circle_filled(r.center(), 4.0, led_color);
                            ui.label(
                                egui::RichText::new(if enabled { "Enabled" } else { "Bypassed" })
                                    .color(theme::TEXT_2)
                                    .size(11.0),
                            );
                        });
                    })
                    .response;
                if resp.interact(egui::Sense::click()).clicked() {
                    let next = if enabled { 0.0 } else { 1.0 };
                    editor_widgets::commit_plain(ui.ctx(), &osc_params.enabled, next);
                }
            });
        });
        ui.separator();

        // Knob row. Wraps rather than overflowing when the editor is
        // narrowed toward its minimum width: six cells no longer fit there.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
            float_knob(ui, "Position", &osc_params.position);
            int_knob(ui, "Coarse", &osc_params.coarse);
            float_knob(ui, "Fine", &osc_params.fine);
            float_knob(ui, "Level", &osc_params.level);
            float_knob(ui, "Pan", &osc_params.pan);
            float_knob(ui, "Warp", &warp_params.amount);
        });

        // This oscillator's warp mode; its amount is the last knob above.
        choice_segmented(ui, &warp_params.mode);

        // Interaction row. Global rather than per-oscillator, but it is
        // about how these two oscillators combine, so it lives with them.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);
            ui.label(
                egui::RichText::new("OSC MIX")
                    .color(theme::TEXT_3)
                    .size(9.5)
                    .strong(),
            );
            choice_segmented(ui, &app.params.osc_mix.mode);
            float_knob(ui, "Amount", &app.params.osc_mix.amount);
        });
    });
}

fn draw_unison_card(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    panel(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(10.0, 8.0);
        ui.horizontal(|ui| {
            panel_title(ui, "UNISON");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let v = app.params.unison.voices.value();
                ui.label(
                    egui::RichText::new(format!("{} voice{}", v, if v == 1 { "" } else { "s" }))
                        .color(theme::TEXT_3)
                        .size(10.5)
                        .monospace(),
                );
            });
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
            int_knob(ui, "Voices", &app.params.unison.voices);
            float_knob(ui, "Detune", &app.params.unison.detune);
            float_knob(ui, "Spread", &app.params.unison.spread);
            // Analog instability sits with unison: both are per-sub-voice
            // character (start phase and pitch drift act on every sub-voice).
            float_knob(ui, "Phase", &app.params.analog.phase_random);
            float_knob(ui, "Analog", &app.params.analog.drift);
        });
    });
}

fn draw_sub_noise_card(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    panel(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(10.0, 8.0);
        panel_title(ui, "SUB · NOISE");
        // Wraps at the editor's minimum width, where a quarter of the body
        // is narrower than three knob cells.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
            float_knob(ui, "Sub", &app.params.sub.level);
            float_knob(ui, "Noise", &app.params.noise.level);
            float_knob(ui, "Color", &app.params.noise.color);
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 0.0);
            choice_cycle(ui, &app.params.sub.waveform);
            choice_cycle(ui, &app.params.sub.octave);
            choice_cycle(ui, &app.params.noise.noise_type);
        });
    });
}

fn draw_global_card(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    panel(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(10.0, 8.0);
        ui.horizontal(|ui| {
            panel_title(ui, "GLOBAL");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let max = app.params.max_voices.value();
                ui.label(
                    egui::RichText::new(format!("poly · {} max", max))
                        .color(theme::TEXT_3)
                        .size(10.5)
                        .monospace(),
                );
            });
        });
        // Wraps at the editor's minimum width, like the Sub · Noise row.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
            float_knob(ui, "Master", &app.params.master_volume);
            float_knob(ui, "Glide", &app.params.glide_time);
            int_knob(ui, "Max", &app.params.max_voices);
        });

        // Glide toggle below the knob row.
        let on = app.params.glide_enabled.value();
        let resp = ui
            .horizontal(|ui| {
                let (rect, r) = ui.allocate_exact_size(
                    egui::vec2(32.0, 18.0),
                    egui::Sense::click(),
                );
                let pill_color = if on { theme::ACCENT } else { theme::BG_3 };
                ui.painter().rect_filled(rect, 9.0, pill_color);
                ui.painter().rect_stroke(
                    rect,
                    9.0,
                    egui::Stroke::new(
                        1.0,
                        if on { theme::ACCENT } else { theme::LINE },
                    ),
                    egui::StrokeKind::Inside,
                );
                let knob_x = if on { rect.right() - 9.0 } else { rect.left() + 9.0 };
                let knob_color = if on {
                    egui::Color32::WHITE
                } else {
                    theme::TEXT_3
                };
                ui.painter()
                    .circle_filled(egui::pos2(knob_x, rect.center().y), 6.0, knob_color);
                ui.label(
                    egui::RichText::new("Glide on")
                        .color(theme::TEXT_1)
                        .size(11.0),
                );
                r
            })
            .inner;
        if resp.clicked() {
            let next = if on { 0.0 } else { 1.0 };
            editor_widgets::commit_plain(ui.ctx(), &app.params.glide_enabled, next);
        }
    });
}
