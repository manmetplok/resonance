//! The lavender widget kit of the redesigned control strip (ba todo
//! #1138, design doc #264 req-3/req-5): macro (56 px) and texture
//! (38 px) knobs — hierarchy by size, not chrome — with bipolar
//! centre-out arcs and the feedback knob's warm over-unity zone;
//! horizontal/vertical segmented controls; and chip toggles. Plus the
//! legacy toggle/combo bindings the grouped strip still uses until the
//! signal-flow layout todo lands.
//!
//! The knobs themselves are the shared `wayland_plugin_gui::widgets`
//! themed knob, configured with the two [`KnobStyle`]s below — this
//! module used to carry a fork of the whole widget (drawing primitives
//! included) with its own drag sensitivity, so the same mouse gesture
//! moved a granular knob further than a knob in any other plugin
//! (ba todo #1266). The granular-specific composites — the division
//! stepper and the freeze latch — live in [`super::controls`].
//!
//! Every setter goes through `Param::set_plain`, the same GUI→host
//! path the other editors use (the CLAP bridge picks the new value up
//! and emits the host param event), so host automation and editor
//! edits stay consistent. All colors are design-system v1 lavender
//! tokens (alpha shaping via `gamma_multiply` only — no new raw
//! colors).

use egui::Ui;
use wayland_plugin_gui::egui;
use wayland_plugin_gui::widgets::{knob_themed, KnobStyle, ThemedKnob};

use crate::params::GranularDelayParams;

use super::theme;

/// Macro-tier knob diameter (Time, Size, Density, Pitch, Feedback, Mix).
pub const MACRO_KNOB_SIZE: f32 = 56.0;
/// Texture-tier knob diameter (everything else).
pub const TEXTURE_KNOB_SIZE: f32 = 38.0;

/// Extra cell height under a knob for the value + label lines.
const KNOB_TEXT_H: f32 = 30.0;

/// Macro-tier cell: 56 px dial, full-size value + label type.
pub const MACRO_KNOB_STYLE: KnobStyle = granular_knob_style(MACRO_KNOB_SIZE, 10.5, 9.0);
/// Texture-tier cell: 38 px dial, smaller type (hierarchy by size, not
/// chrome).
pub const TEXTURE_KNOB_STYLE: KnobStyle = granular_knob_style(TEXTURE_KNOB_SIZE, 9.5, 8.0);

/// The granular strip's cell proportions at a given dial size: a wider
/// gutter and tighter text rows than the shared default, so the six
/// groups fit the 1320 px window.
const fn granular_knob_style(diameter: f32, value_font: f32, label_font: f32) -> KnobStyle {
    KnobStyle {
        diameter,
        pad_x: 12.0,
        text_h: KNOB_TEXT_H,
        value_dy: 4.0,
        label_dy: 17.0,
        value_font,
        label_font,
        indicator_inset: 5.0,
    }
}

// ---------------------------------------------------------------------------
// Knobs
// ---------------------------------------------------------------------------

/// Position of a plain parameter value on its linear range, in the
/// unit (`0..1`) space the shared themed knob works in.
pub fn unit_of_plain(min: f32, max: f32, value: f32) -> f32 {
    let span = (max - min).max(f32::EPSILON);
    ((value - min) / span).clamp(0.0, 1.0)
}

/// The plain value a unit knob position maps back to.
pub fn plain_of_unit(min: f32, max: f32, unit: f32) -> f32 {
    (min + unit.clamp(0.0, 1.0) * (max - min)).clamp(min, max)
}

/// Macro-tier (56 px) knob bound to a param; bipolar centre-out arc
/// when the param range spans zero (Pitch).
pub fn macro_knob(ui: &mut Ui, params: &GranularDelayParams, index: usize) {
    knob_param(ui, params, index, MACRO_KNOB_STYLE, None, None);
}

/// Texture-tier (38 px) knob bound to a param.
pub fn texture_knob(ui: &mut Ui, params: &GranularDelayParams, index: usize) {
    knob_param(ui, params, index, TEXTURE_KNOB_STYLE, None, None);
}

/// Texture-tier knob with a custom (short) caption for the 38 px cell
/// (e.g. `SIZE J` instead of the param's full `Size Jitter`).
pub fn texture_knob_labeled(
    ui: &mut Ui,
    params: &GranularDelayParams,
    index: usize,
    label: &str,
) {
    knob_param(ui, params, index, TEXTURE_KNOB_STYLE, None, Some(label));
}

/// Macro knob with the 100–110 % over-unity arc zone marked in the
/// warm token (the Feedback knob, design doc #264 req-5).
pub fn feedback_knob(ui: &mut Ui, params: &GranularDelayParams, index: usize) {
    let p = params.param_at(index);
    // Over-unity begins at plain 1.0 (100 %).
    let warm_from = unit_of_plain(p.min_plain() as f32, p.max_plain() as f32, 1.0);
    knob_param(ui, params, index, MACRO_KNOB_STYLE, Some(warm_from), None);
}

/// Shared param-bound knob body: linear plain↔unit mapping onto the
/// shared themed knob, which owns the drag feel (drag to edit, Shift =
/// fine, double-click resets to the param default).
fn knob_param(
    ui: &mut Ui,
    params: &GranularDelayParams,
    index: usize,
    style: KnobStyle,
    warm_from: Option<f32>,
    label: Option<&str>,
) {
    let p = params.param_at(index);
    let min = p.min_plain() as f32;
    let max = p.max_plain() as f32;
    let val = p.get_plain() as f32;
    let display = p.display(val as f64);

    let knob = ThemedKnob::new(
        label.unwrap_or(p.name()),
        unit_of_plain(min, max, val),
        &display,
        unit_of_plain(min, max, p.default_plain() as f32),
    )
    // Bipolar arc when the param range spans zero (Pitch).
    .bipolar(min < 0.0 && max > 0.0)
    .warm_from(warm_from)
    .style(style);

    if let Some(unit) = knob_themed(ui, &knob) {
        p.set_plain(f64::from(plain_of_unit(min, max, unit)));
    }
}

// ---------------------------------------------------------------------------
// Segmented controls
// ---------------------------------------------------------------------------

/// Horizontal segmented control bound to an int param (FADE/REPITCH/
/// GRAIN, OFF/SEMI/SCALE, LO-FI/NORM/HQ, WET→BUF/OUT ONLY/PONG).
/// `labels` must be a cached static list covering `0..labels.len()`.
pub fn param_segmented(
    ui: &mut Ui,
    params: &GranularDelayParams,
    index: usize,
    labels: &'static [&'static str],
) {
    segmented(ui, params, index, labels, false);
}

/// Vertical segmented control (the SYNC/ASYNC/VOICE scheduler column).
pub fn param_segmented_vertical(
    ui: &mut Ui,
    params: &GranularDelayParams,
    index: usize,
    labels: &'static [&'static str],
) {
    segmented(ui, params, index, labels, true);
}

fn segmented(
    ui: &mut Ui,
    params: &GranularDelayParams,
    index: usize,
    labels: &'static [&'static str],
    vertical: bool,
) {
    let p = params.param_at(index);
    let current = (p.get_plain().round() as usize).min(labels.len().saturating_sub(1));
    let draw_segments = |ui: &mut Ui| {
        ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
        for (i, label) in labels.iter().enumerate() {
            if segment_chip(ui, label, i == current) && i != current {
                p.set_plain(i as f64);
            }
        }
    };
    if vertical {
        ui.vertical(draw_segments);
    } else {
        ui.horizontal(draw_segments);
    }
}

/// One segment pill; returns true when clicked.
fn segment_chip(ui: &mut Ui, label: &str, selected: bool) -> bool {
    let font = egui::FontId::proportional(8.5);
    let galley = ui.painter().layout_no_wrap(
        label.to_uppercase(),
        font.clone(),
        theme::TEXT_1,
    );
    let size = egui::vec2(galley.size().x + 12.0, 16.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter_at(rect);
        let (fill, text_color, stroke) = if selected {
            (theme::ACCENT_DIM, theme::ACCENT_SOFT, theme::ACCENT)
        } else if response.hovered() {
            (theme::BG_3, theme::TEXT_2, theme::LINE)
        } else {
            (theme::BG_1, theme::TEXT_3, theme::LINE_2)
        };
        painter.rect_filled(rect, theme::RADIUS_CHIP, fill);
        painter.rect_stroke(
            rect,
            theme::RADIUS_CHIP,
            egui::Stroke::new(1.0, stroke),
            egui::StrokeKind::Inside,
        );
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label.to_uppercase(),
            font,
            text_color,
        );
    }
    response.clicked()
}

// ---------------------------------------------------------------------------
// Chips
// ---------------------------------------------------------------------------

/// Small pill toggle bound to a bool param (SYNC, PER-BEAT, SHIMMER).
/// With `enabled` false the chip renders greyed and ignores input
/// (route-gated shimmer, design doc #264 req-4).
pub fn param_chip(
    ui: &mut Ui,
    params: &GranularDelayParams,
    index: usize,
    label: &str,
    enabled: bool,
) {
    let p = params.param_at(index);
    let on = p.get_plain() >= 0.5;
    let font = egui::FontId::proportional(8.5);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_uppercase(), font.clone(), theme::TEXT_1);
    let size = egui::vec2(galley.size().x + 14.0, 16.0);
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter_at(rect);
        let (fill, text_color, stroke) = if !enabled {
            (theme::BG_1, theme::TEXT_4, theme::LINE_2)
        } else if on {
            (theme::ACCENT_DIM, theme::ACCENT_SOFT, theme::ACCENT)
        } else if response.hovered() {
            (theme::BG_3, theme::TEXT_2, theme::LINE)
        } else {
            (theme::BG_1, theme::TEXT_3, theme::LINE_2)
        };
        painter.rect_filled(rect, theme::RADIUS_CHIP, fill);
        painter.rect_stroke(
            rect,
            theme::RADIUS_CHIP,
            egui::Stroke::new(1.0, stroke),
            egui::StrokeKind::Inside,
        );
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label.to_uppercase(),
            font,
            text_color,
        );
    }
    if enabled && response.clicked() {
        p.set_plain(if on { 0.0 } else { 1.0 });
    }
}

// ---------------------------------------------------------------------------
// Combo binding (Root/Scale selects)
// ---------------------------------------------------------------------------

/// Compact combo box for enumerated parameters (the Root/Scale
/// selects). `labels` must be a cached static list (view-performance
/// rules: no per-frame option building) covering the param's plain
/// range `0..labels.len()`.
pub fn param_choice(
    ui: &mut Ui,
    params: &GranularDelayParams,
    index: usize,
    labels: &'static [&'static str],
    width: f32,
) {
    let p = params.param_at(index);
    let current = (p.get_plain().round() as usize).min(labels.len().saturating_sub(1));
    egui::ComboBox::from_id_salt(p.id())
        .width(width)
        .selected_text(
            egui::RichText::new(labels[current]).size(10.0),
        )
        .show_ui(ui, |ui| {
            for (i, label) in labels.iter().enumerate() {
                if ui.selectable_label(i == current, *label).clicked() {
                    p.set_plain(i as f64);
                }
            }
        });
}
