//! The signal-flow control strip (ba todo #1141, design doc #264
//! req-3/req-5): `TIME → GRAINS → PITCH → FEEDBACK → SPACE → OUTPUT`
//! with the macro/texture knob hierarchy of the widget kit — 56 px
//! macro knobs (Time, Size, Density, Pitch, Feedback, Mix) vs 38 px
//! texture knobs, hierarchy by size, not chrome.
//!
//! The grouping table, widget kinds and option-label lists are pure
//! `'static` data (view-performance rules: static option lists are
//! built once, never per frame) and are unit-tested in
//! tests/editor_groups.rs: the groups cover every declared parameter
//! index exactly once and every label list matches its param's range.
//! The bespoke `draw` below renders those same groups per the
//! prototype's 'default' state; conditional disclosure (sync↔division
//! swap, Root/Scale reveal, route-gated shimmer) is the next todo —
//! until then every control renders unconditionally so all 29 params
//! stay reachable.

use egui::Ui;
use wayland_plugin_gui::egui;

use crate::params::GranularDelayParams;
use crate::sync::DIVISION_LABELS;
use crate::viz::GranularViz;

use super::theme;
use super::widgets::{
    division_stepper, feedback_knob, freeze_latch, macro_knob, param_chip, param_choice,
    param_segmented, param_segmented_vertical, texture_knob_labeled,
};

/// One titled group of parameter controls.
pub struct ParamGroup {
    pub name: &'static str,
    /// `param_at` indices (see `crate::params`).
    pub params: &'static [usize],
}

/// The signal-flow groups, in display order (design doc #264 req-3).
pub const GROUPS: &[ParamGroup] = &[
    ParamGroup {
        name: "Time",
        params: &[0, 2, 1, 3], // sync, time_ms, division, time_mode
    },
    ParamGroup {
        name: "Grains",
        // grain_size, density, density_sync, scheduler + the TEXTURE
        // sub-row: texture, spray, size_jitter, level_jitter, reverse
        params: &[7, 8, 9, 10, 14, 15, 16, 17, 18],
    },
    ParamGroup {
        name: "Pitch",
        params: &[11, 13, 12, 27, 28], // pitch, spread, quantize, root, scale
    },
    ParamGroup {
        name: "Feedback",
        // feedback, fb_route, fb_pitch (shimmer), filter_type, filter_hz
        params: &[4, 5, 6, 20, 21],
    },
    ParamGroup {
        name: "Space",
        params: &[23, 24, 22], // pan_spread, width, diffusion
    },
    ParamGroup {
        name: "Output",
        params: &[25, 26, 19], // mix, quality, freeze
    },
];

/// Display order of the groups (single signal-flow row).
pub const GROUP_ROWS: &[&[usize]] = &[&[0, 1, 2, 3, 4, 5]];

/// Macro-tier params (design doc #264 req-3): Time, Size, Density,
/// Pitch, Feedback, Mix render as 56 px macro knobs; every other
/// continuous param is a 38 px texture knob.
pub const MACRO_PARAMS: &[usize] = &[2, 7, 8, 11, 4, 25];

/// Widget flavour for one parameter index.
pub enum ControlKind {
    /// Continuous rotary knob.
    Knob,
    /// Boolean toggle (chip or latch).
    Toggle,
    /// Enumerated control with a cached static label list (segmented
    /// or combo).
    Choice(&'static [&'static str]),
}

pub const TIME_MODE_LABELS: &[&str] = &["Fade", "Repitch", "Grain"];
pub const SCHEDULER_LABELS: &[&str] = &["Sync", "Async", "Voice"];
pub const FB_ROUTE_LABELS: &[&str] = &["Wet→Buf", "Out Only", "Pong"];
pub const QUANTIZE_LABELS: &[&str] = &["Off", "Semi", "Scale"];
pub const FILTER_TYPE_LABELS: &[&str] = &["LP", "HP"];
pub const QUALITY_LABELS: &[&str] = &["Lo-Fi", "Norm", "HQ"];
pub const ROOT_LABELS: &[&str] = &[
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
/// Display names of `resonance_music_theory::Mode::ALL`, in order
/// (locked to the source of truth by tests/editor_groups.rs).
pub const SCALE_LABELS: &[&str] = &[
    "Chromatic",
    "Major",
    "Minor",
    "Dorian",
    "Phrygian",
    "Lydian",
    "Mixolydian",
    "Locrian",
    "Harmonic Minor",
    "Melodic Minor",
];

/// Widget mapping for every declared parameter index (see
/// `GranularDelayParams::param_at` for the index table).
pub fn control_kind(index: usize) -> ControlKind {
    match index {
        0 | 6 | 9 | 19 => ControlKind::Toggle, // sync, fb_pitch, density_sync, freeze
        1 => ControlKind::Choice(DIVISION_LABELS),
        3 => ControlKind::Choice(TIME_MODE_LABELS),
        5 => ControlKind::Choice(FB_ROUTE_LABELS),
        10 => ControlKind::Choice(SCHEDULER_LABELS),
        12 => ControlKind::Choice(QUANTIZE_LABELS),
        20 => ControlKind::Choice(FILTER_TYPE_LABELS),
        26 => ControlKind::Choice(QUALITY_LABELS),
        27 => ControlKind::Choice(ROOT_LABELS),
        28 => ControlKind::Choice(SCALE_LABELS),
        _ => ControlKind::Knob,
    }
}

/// Fixed group widths, px (prototype proportions at 1320 px). Grains
/// is the widest (texture sub-row); the sum + gaps fits the window.
const GROUP_W: [f32; 6] = [172.0, 330.0, 186.0, 246.0, 160.0, 186.0];
/// Horizontal gap between group frames.
const GROUP_GAP: f32 = 4.0;
/// Group frame content height inside the 246 px band.
const GROUP_H: f32 = 222.0;

/// Draw the whole signal-flow strip (bespoke layout per group; the
/// declarative [`GROUPS`] table above stays the tested coverage map).
pub fn draw(ui: &mut Ui, params: &GranularDelayParams, viz: &GranularViz) {
    let bpm = viz.read_bpm();
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(GROUP_GAP, 4.0);
        ui.add_space(4.0);
        group_frame(ui, GROUP_W[0], "TIME", |ui| draw_time(ui, params, bpm), true, params);
        group_frame(ui, GROUP_W[1], "GRAINS", |ui| draw_grains(ui, params), false, params);
        group_frame(ui, GROUP_W[2], "PITCH", |ui| draw_pitch(ui, params), false, params);
        group_frame(ui, GROUP_W[3], "FEEDBACK", |ui| draw_feedback(ui, params), false, params);
        group_frame(ui, GROUP_W[4], "SPACE", |ui| draw_space(ui, params), false, params);
        group_frame(ui, GROUP_W[5], "OUTPUT", |ui| draw_output(ui, params), false, params);
    });
}

/// One titled group frame: name + hairline (+ the SYNC chip on TIME),
/// then the group body.
fn group_frame(
    ui: &mut Ui,
    width: f32,
    title: &str,
    body: impl FnOnce(&mut Ui),
    sync_chip: bool,
    params: &GranularDelayParams,
) {
    let frame = egui::Frame::new()
        .fill(theme::BG_2)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(theme::RADIUS_PANEL)
        .inner_margin(egui::Margin::symmetric(8, 6));
    frame.show(ui, |ui| {
        ui.set_width(width - 18.0);
        ui.set_height(GROUP_H);
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(title)
                        .small()
                        .strong()
                        .color(theme::TEXT_3),
                );
                // SYNC chip lives in the TIME group title (req-4).
                let chip_w = if sync_chip { 52.0 } else { 0.0 };
                let tail = (ui.available_width() - chip_w - 6.0).max(0.0);
                if tail > 0.0 {
                    let (rect, _) = ui
                        .allocate_exact_size(egui::vec2(tail, 12.0), egui::Sense::hover());
                    ui.painter().line_segment(
                        [
                            egui::pos2(rect.left() + 4.0, rect.center().y),
                            egui::pos2(rect.right() - 2.0, rect.center().y),
                        ],
                        egui::Stroke::new(1.0, theme::LINE_2),
                    );
                }
                if sync_chip {
                    param_chip(ui, params, 0, "SYNC", true);
                }
            });
            ui.add_space(4.0);
            body(ui);
        });
    });
}

/// Small caption under a segmented control.
fn caption(ui: &mut Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .size(8.0)
            .color(theme::TEXT_4),
    );
}

/// TIME: Time macro knob + division stepper (the in-place sync swap is
/// the disclosure todo — both render for now so division stays
/// reachable), time-mode segmented below.
fn draw_time(ui: &mut Ui, params: &GranularDelayParams, bpm: f32) {
    ui.horizontal(|ui| {
        macro_knob(ui, params, 2);
        division_stepper(ui, params, 1, bpm);
    });
    ui.add_space(2.0);
    ui.vertical_centered(|ui| {
        param_segmented(ui, params, 3, TIME_MODE_LABELS);
        caption(ui, "time mode");
    });
}

/// GRAINS: Size + Density macro knobs (PER-BEAT chip under Density),
/// the vertical scheduler segmented, and the bordered TEXTURE sub-row.
fn draw_grains(ui: &mut Ui, params: &GranularDelayParams) {
    ui.horizontal(|ui| {
        macro_knob(ui, params, 7);
        ui.vertical(|ui| {
            macro_knob(ui, params, 8);
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                param_chip(ui, params, 9, "PER-BEAT", true);
            });
        });
        ui.add_space(6.0);
        ui.vertical(|ui| {
            ui.add_space(8.0);
            param_segmented_vertical(ui, params, 10, SCHEDULER_LABELS);
            caption(ui, "scheduler");
        });
    });
    // Bordered TEXTURE sub-row.
    let frame = egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(theme::RADIUS_CHIP)
        .inner_margin(egui::Margin::symmetric(6, 2));
    frame.show(ui, |ui| {
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new("TEXTURE")
                    .size(8.0)
                    .color(theme::TEXT_4),
            );
            ui.horizontal(|ui| {
                texture_knob_labeled(ui, params, 14, "Shape");
                texture_knob_labeled(ui, params, 15, "Spray");
                texture_knob_labeled(ui, params, 16, "Size J");
                texture_knob_labeled(ui, params, 17, "Lvl J");
                texture_knob_labeled(ui, params, 18, "Rev");
            });
        });
    });
}

/// PITCH: bipolar Pitch macro + Spread texture knob, Quantize
/// segmented, Root/Scale selects (always rendered until the disclosure
/// todo gates them behind Quantize == Scale).
fn draw_pitch(ui: &mut Ui, params: &GranularDelayParams) {
    ui.horizontal(|ui| {
        macro_knob(ui, params, 11);
        texture_knob_labeled(ui, params, 13, "Spread");
    });
    ui.vertical_centered(|ui| {
        param_segmented(ui, params, 12, QUANTIZE_LABELS);
        caption(ui, "quantize");
    });
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        param_choice(ui, params, 27, ROOT_LABELS, 52.0);
        param_choice(ui, params, 28, SCALE_LABELS, 92.0);
    });
}

/// FEEDBACK: Feedback macro knob (warm over-unity zone), vertical
/// route segmented + SHIMMER chip, damp filter knob + LP/HP.
fn draw_feedback(ui: &mut Ui, params: &GranularDelayParams) {
    ui.horizontal(|ui| {
        feedback_knob(ui, params, 4);
        ui.vertical(|ui| {
            ui.add_space(4.0);
            param_segmented_vertical(ui, params, 5, FB_ROUTE_LABELS);
            ui.add_space(4.0);
            param_chip(ui, params, 6, "SHIMMER", true);
        });
        ui.add_space(4.0);
        ui.vertical(|ui| {
            texture_knob_labeled(ui, params, 21, "Filter");
            ui.horizontal(|ui| {
                ui.add_space(2.0);
                param_segmented(ui, params, 20, FILTER_TYPE_LABELS);
            });
            caption(ui, "damp");
        });
    });
}

/// SPACE: Pan Spread, Width, Diffuse texture knobs (Diffuse is
/// designed-but-inert until its DSP lands — still wired to its param).
fn draw_space(ui: &mut Ui, params: &GranularDelayParams) {
    ui.horizontal(|ui| {
        texture_knob_labeled(ui, params, 23, "Pan Spr");
        texture_knob_labeled(ui, params, 24, "Width");
        texture_knob_labeled(ui, params, 22, "Diffuse");
    });
}

/// OUTPUT: Mix macro knob, the amber FREEZE latch, Quality segmented.
fn draw_output(ui: &mut Ui, params: &GranularDelayParams) {
    ui.horizontal(|ui| {
        macro_knob(ui, params, 25);
        ui.vertical(|ui| {
            ui.add_space(18.0);
            freeze_latch(ui, params, 19);
        });
    });
    ui.add_space(6.0);
    ui.vertical_centered(|ui| {
        param_segmented(ui, params, 26, QUALITY_LABELS);
        caption(ui, "quality");
    });
}
