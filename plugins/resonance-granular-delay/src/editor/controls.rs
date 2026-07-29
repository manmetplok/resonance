//! Control-surface data and rendering: the §9 parameter groups
//! (ba todo #1079, doc #252 §9) and the per-parameter widget mapping.
//!
//! The grouping table, widget kinds and option-label lists are pure
//! `'static` data (view-performance rules: static option lists are
//! built once, never per frame) and are unit-tested in
//! tests/editor_groups.rs: the groups cover every declared parameter
//! index exactly once and every label list matches its param's range.

use wayland_plugin_gui::egui;

use crate::params::GranularDelayParams;
use crate::sync::DIVISION_LABELS;

use super::theme;
use super::widgets::{param_choice, param_knob, param_toggle};

/// One titled group of parameter controls.
pub struct ParamGroup {
    pub name: &'static str,
    /// `param_at` indices (see `crate::params`).
    pub params: &'static [usize],
}

/// The §9 control groups, in display order (ba todo #1079).
pub const GROUPS: &[ParamGroup] = &[
    ParamGroup {
        name: "Time",
        params: &[0, 1, 2, 3], // sync, division, time_ms, time_mode
    },
    ParamGroup {
        name: "Grains",
        // grain_size, density, density_sync, scheduler, texture,
        // spray, size_jitter, level_jitter, reverse_prob
        params: &[7, 8, 9, 10, 14, 15, 16, 17, 18],
    },
    ParamGroup {
        name: "Pitch",
        // pitch, pitch_quantize, root, scale, spread_cents, fb_pitch
        params: &[11, 12, 27, 28, 13, 6],
    },
    ParamGroup {
        name: "Feedback",
        // feedback, fb_route, filter_type, filter_hz, freeze
        params: &[4, 5, 20, 21, 19],
    },
    ParamGroup {
        name: "Space",
        params: &[23, 24, 22], // pan_spread, width, diffusion
    },
    ParamGroup {
        name: "Output",
        params: &[25, 26], // mix, quality
    },
];

/// Widget flavour for one parameter index.
pub enum ControlKind {
    /// Continuous rotary knob.
    Knob,
    /// Boolean toggle.
    Toggle,
    /// Enumerated combo with a cached static label list.
    Choice(&'static [&'static str]),
}

pub const TIME_MODE_LABELS: &[&str] = &["Fade", "Repitch", "Per-Grain"];
pub const SCHEDULER_LABELS: &[&str] = &["Sync", "Async", "Pitch-Sync"];
pub const FB_ROUTE_LABELS: &[&str] = &["Wet→Buffer", "Output Only", "Ping-Pong"];
pub const QUANTIZE_LABELS: &[&str] = &["Off", "Semitones", "Scale"];
pub const FILTER_TYPE_LABELS: &[&str] = &["LP", "HP"];
pub const QUALITY_LABELS: &[&str] = &["Lo-fi", "Normal", "HQ"];
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

/// Row layout of the group frames (indices into [`GROUPS`]): Time and
/// Pitch share the first row, the wide Grains group takes the second,
/// Feedback/Space/Output share the third.
pub const GROUP_ROWS: &[&[usize]] = &[&[0, 2], &[1], &[3, 4, 5]];

/// Draw the whole control surface.
pub fn draw(ui: &mut egui::Ui, params: &GranularDelayParams) {
    for row in GROUP_ROWS {
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            for &g in row.iter() {
                let group = &GROUPS[g];
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new(group.name.to_uppercase())
                                .small()
                                .strong()
                                .color(theme::TEXT_3),
                        );
                        ui.horizontal(|ui| {
                            for &index in group.params {
                                match control_kind(index) {
                                    ControlKind::Knob => param_knob(ui, params, index),
                                    ControlKind::Toggle => param_toggle(ui, params, index),
                                    ControlKind::Choice(labels) => {
                                        param_choice(ui, params, index, labels)
                                    }
                                }
                            }
                        });
                    });
                });
                ui.add_space(4.0);
            }
        });
        ui.add_space(4.0);
    }
}
