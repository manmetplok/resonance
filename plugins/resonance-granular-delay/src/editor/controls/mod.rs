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
//! The bespoke `draw` below renders those groups per the prototype,
//! with the req-4 conditional disclosure (ba todo #1142) layered on —
//! sync swaps the Time knob for the division stepper in-place,
//! Quantize = Scale reveals Root/Scale in reserved space, and the
//! SHIMMER chip greys out on the Output-only route. All disclosure is
//! a pure function of the param values; every param stays reachable
//! in some param state and nothing is orphaned (division/time are the
//! two faces of the same tap).
//!
//! The two composites that only make sense here — the division stepper
//! that swaps in for the Time knob and the amber FREEZE latch — sit in
//! the submodules beside this file; the generic knobs/segments/chips
//! they are laid out with come from [`super::widgets`], which is now a
//! thin binding over the shared `wayland_plugin_gui` kit.

use egui::Ui;
use wayland_plugin_gui::egui;

use crate::params::GranularDelayParams;
use crate::choice::ChoiceParam;
use crate::dsp::{DampingFilter, FbRoute, QualityTier, Scheduler, TimeMode};
use crate::quantize::PitchQuantize;
use crate::sync::DIVISION_LABELS;
use crate::viz::GranularViz;

use super::theme;
use super::widgets::{
    feedback_knob, macro_knob, param_chip, param_choice, param_segmented,
    param_segmented_vertical, texture_knob_labeled,
};

// The two granular-specific composites of the strip. They are controls,
// not generic widgets, so they live here rather than in the widget kit
// (ba todo #1266).
mod division_stepper;
mod freeze_latch;

pub use division_stepper::{division_stepper, DivisionReadout};
pub use freeze_latch::freeze_latch;

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
        // grain_size, density, density_sync, density_division, align,
        // scheduler + the TEXTURE sub-row: texture, spray, size_jitter,
        // level_jitter, reverse
        params: &[7, 8, 9, 30, 29, 10, 14, 15, 16, 17, 18],
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

// The six mode choices keep their cached static slices (the view-perf
// rule: no per-frame allocation), but the slices are the enums' own
// label tables — the enum next to the DSP is the one place that knows
// what each integer means (ba todo #1267).
pub const TIME_MODE_LABELS: &[&str] = TimeMode::LABELS;
pub const SCHEDULER_LABELS: &[&str] = Scheduler::LABELS;
pub const FB_ROUTE_LABELS: &[&str] = FbRoute::LABELS;
pub const QUANTIZE_LABELS: &[&str] = PitchQuantize::LABELS;
pub const FILTER_TYPE_LABELS: &[&str] = DampingFilter::LABELS;
pub const QUALITY_LABELS: &[&str] = QualityTier::LABELS;
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
        // sync, fb_pitch, density_sync, freeze, align
        0 | 6 | 9 | 19 | 29 => ControlKind::Toggle,
        // division (delay tap) and density_division (grain rate)
        1 | 30 => ControlKind::Choice(DIVISION_LABELS),
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
        group_frame(ui, GROUP_W[1], "GRAINS", |ui| draw_grains(ui, params, bpm), false, params);
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

/// TIME: with SYNC off the Time macro knob; with SYNC on it swaps
/// in-place for the division stepper (ba todo #1142, design doc #264
/// req-4 — both widgets share the same cell, so no layout jump).
/// Time-mode segmented below.
fn draw_time(ui: &mut Ui, params: &GranularDelayParams, bpm: f32) {
    ui.vertical_centered(|ui| {
        if params.sync.value() {
            division_stepper(ui, params, 1, bpm, DivisionReadout::DelayMs);
        } else {
            macro_knob(ui, params, 2);
        }
    });
    ui.add_space(2.0);
    ui.vertical_centered(|ui| {
        param_segmented(ui, params, 3, TIME_MODE_LABELS);
        caption(ui, "time mode");
    });
}

/// GRAINS: Size + Density macro knobs (the PER-BEAT and ALIGN chips
/// under Density), the vertical scheduler segmented, and the bordered
/// TEXTURE sub-row.
///
/// PER-BEAT is the same conditional disclosure as SYNC in the TIME
/// group (req-4, ba todo #1322): with it on, the Density knob swaps
/// in-place for the density-division stepper — same cell, no layout
/// jump — and the readout shows the resulting grains per second at the
/// host tempo.
fn draw_grains(ui: &mut Ui, params: &GranularDelayParams, bpm: f32) {
    ui.horizontal(|ui| {
        macro_knob(ui, params, 7);
        ui.vertical(|ui| {
            if params.density_sync.value() {
                division_stepper(ui, params, 30, bpm, DivisionReadout::GrainsPerSecond);
            } else {
                macro_knob(ui, params, 8);
            }
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                param_chip(ui, params, 9, "PER-BEAT", true);
                // WSOLA onset alignment (ba todo #1320): a plain bool
                // param, so the same switch is reachable over
                // set_plugin_param.
                param_chip(ui, params, 29, "ALIGN", true);
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
/// segmented; the Root/Scale selects reveal only in Scale mode
/// (ba todo #1142, req-4). Their row space stays reserved so the
/// reveal never reflows the strip.
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
    if params.pitch_quantize.value() == 2 {
        ui.horizontal(|ui| {
            param_choice(ui, params, 27, ROOT_LABELS, 52.0);
            param_choice(ui, params, 28, SCALE_LABELS, 92.0);
        });
    } else {
        // Reserved footprint of the collapsed Root/Scale row.
        ui.allocate_exact_size(egui::vec2(150.0, 18.0), egui::Sense::hover());
    }
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
            // Shimmer has no effect on the Output-only route: the chip
            // renders disabled (greyed, ignores input) there (ba todo
            // #1142, req-4).
            let shimmer_enabled = params.fb_route.value() != 1;
            param_chip(ui, params, 6, "SHIMMER", shimmer_enabled);
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

/// SPACE: Pan Spread, Width, Diffuse texture knobs.
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
