//! Reusable egui widgets for plugin editors.
//!
//! Provides a rotary knob and a labelled control column so plugin UIs
//! share a consistent look and compact layout. Everything here is pure
//! egui — helpers that bind these widgets to plugin parameter types
//! live downstream in `resonance_plugin::editor_widgets`, keeping this
//! crate free of plugin-framework dependencies.
//!
//! Most builders take 8 arguments (label, value, range, step, formatter,
//! …) because the widgets need to expose every visual + interaction
//! knob the plugin editors set per-call; the alternative — wrapping
//! them in a config struct — adds boilerplate without any readability
//! win, so we allow the `too_many_arguments` lint module-wide.
#![allow(clippy::too_many_arguments)]

use egui::{self, Color32, Pos2, Rect, Response, Sense, Stroke, Vec2};
use std::f32::consts::PI;

// ---------------------------------------------------------------------------
// Rotary knob
// ---------------------------------------------------------------------------

/// Arc sweep: 270° starting at 135° (bottom-left) to -45° (bottom-right).
const ARC_START: f32 = 135.0 * PI / 180.0;
const ARC_END: f32 = ARC_START + 270.0 * PI / 180.0;

/// Knob colours — intentionally a fixed palette so all plugins look the same.
const TRACK_COLOR: Color32 = Color32::from_rgb(0x30, 0x30, 0x38);
const ARC_COLOR: Color32 = Color32::from_rgb(0x4a, 0x9e, 0xcf);
const DOT_COLOR: Color32 = Color32::WHITE;
const TEXT_COLOR: Color32 = Color32::from_rgb(0xcc, 0xcc, 0xd0);
const LABEL_COLOR: Color32 = Color32::from_rgb(0x88, 0x88, 0x90);
const SUBLABEL_COLOR: Color32 = Color32::from_rgb(0x66, 0x66, 0x70);

/// Draw a rotary knob for a floating-point value.
///
/// Returns `true` if the value changed.
///
/// - `value`: current value, mutated in place on drag.
/// - `range`: allowed min/max.
/// - `default`: value to reset to on double-click.
/// - `label`: title above the knob.
/// - `value_text`: formatted string shown below the knob.
/// - `logarithmic`: if true, drag maps logarithmically.
pub fn knob(
    ui: &mut egui::Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    default: f32,
    label: &str,
    sub_label: &str,
    value_text: &str,
    logarithmic: bool,
) -> bool {
    let knob_radius = 20.0f32;
    let total_width = 64.0f32;
    let total_height = knob_radius * 2.0 + 36.0; // knob + label + value

    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(total_width, total_height),
        Sense::click_and_drag(),
    );

    let changed = handle_knob_input(&response, value, &range, default, logarithmic);

    if ui.is_rect_visible(rect) {
        draw_knob(
            ui,
            rect,
            *value,
            &range,
            knob_radius,
            label,
            sub_label,
            value_text,
        );
    }

    changed
}

fn handle_knob_input(
    response: &Response,
    value: &mut f32,
    range: &std::ops::RangeInclusive<f32>,
    default: f32,
    logarithmic: bool,
) -> bool {
    let mut changed = false;

    // Double-click to reset to default.
    if response.double_clicked() {
        *value = default;
        return true;
    }

    if response.dragged() {
        let delta_y = -response.drag_delta().y;
        let sensitivity = if logarithmic { 0.004 } else { 0.005 };

        if logarithmic {
            let min = range.start().max(0.001);
            let max = *range.end();
            let log_min = min.ln();
            let log_max = max.ln();
            let log_v = value.max(min).ln();
            let normalized = (log_v - log_min) / (log_max - log_min);
            let new_norm = (normalized + delta_y * sensitivity).clamp(0.0, 1.0);
            *value = (log_min + new_norm * (log_max - log_min)).exp();
        } else {
            let span = range.end() - range.start();
            *value += delta_y * sensitivity * span;
        }
        *value = value.clamp(*range.start(), *range.end());
        changed = true;
    }

    changed
}

fn draw_knob(
    ui: &egui::Ui,
    rect: Rect,
    value: f32,
    range: &std::ops::RangeInclusive<f32>,
    radius: f32,
    label: &str,
    sub_label: &str,
    value_text: &str,
) {
    let painter = ui.painter_at(rect);

    // Label above the knob.
    let label_pos = Pos2::new(rect.center().x, rect.min.y + 2.0);
    painter.text(
        label_pos,
        egui::Align2::CENTER_TOP,
        label,
        egui::FontId::proportional(10.0),
        LABEL_COLOR,
    );

    // Sub-label below the label.
    let sub_y = if sub_label.is_empty() { 0.0 } else { 10.0 };
    if !sub_label.is_empty() {
        let sub_pos = Pos2::new(rect.center().x, rect.min.y + 13.0);
        painter.text(
            sub_pos,
            egui::Align2::CENTER_TOP,
            sub_label,
            egui::FontId::proportional(8.0),
            SUBLABEL_COLOR,
        );
    }

    // Knob center.
    let center = Pos2::new(rect.center().x, rect.min.y + 14.0 + sub_y + radius);
    let track_width = 3.0f32;
    let arc_width = 3.5f32;

    // Background arc.
    draw_arc(
        &painter,
        center,
        radius,
        ARC_START,
        ARC_END,
        track_width,
        TRACK_COLOR,
    );

    // Value arc.
    let normalized = normalize(value, range);
    let value_angle = ARC_START + normalized * (ARC_END - ARC_START);
    if normalized > 0.001 {
        draw_arc(
            &painter,
            center,
            radius,
            ARC_START,
            value_angle,
            arc_width,
            ARC_COLOR,
        );
    }

    // Indicator dot at the value position.
    let dot_radius_offset = radius - 1.0;
    let dot_x = center.x + dot_radius_offset * value_angle.cos();
    let dot_y = center.y - dot_radius_offset * value_angle.sin();
    painter.circle_filled(Pos2::new(dot_x, dot_y), 2.5, DOT_COLOR);

    // Value text below the knob.
    let value_pos = Pos2::new(rect.center().x, center.y + radius + 4.0);
    painter.text(
        value_pos,
        egui::Align2::CENTER_TOP,
        value_text,
        egui::FontId::monospace(9.0),
        TEXT_COLOR,
    );
}

fn normalize(value: f32, range: &std::ops::RangeInclusive<f32>) -> f32 {
    let span = range.end() - range.start();
    if span.abs() < f32::EPSILON {
        return 0.0;
    }
    ((value - range.start()) / span).clamp(0.0, 1.0)
}

fn draw_arc(
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    start: f32,
    end: f32,
    width: f32,
    color: Color32,
) {
    let segments = 48;
    let span = end - start;
    let step = span / segments as f32;
    // One owned, exactly-sized Vec per arc per frame is the floor here:
    // epaint's `PathShape` stores `points: Vec<Pos2>` by value, so a
    // borrowed/reused buffer would have to be cloned into it anyway.
    // What we avoid is the old shape per segment — 48 `line_segment`
    // calls pushed 48 `Shape`s into the paint list per arc; a single
    // `Shape::line` polyline is one shape, one allocation, and
    // tessellates with proper joins instead of butt-end overlaps.
    let points: Vec<Pos2> = (0..=segments)
        .map(|i| {
            let angle = start + step * i as f32;
            Pos2::new(
                center.x + radius * angle.cos(),
                center.y - radius * angle.sin(),
            )
        })
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new(width, color)));
}

// ---------------------------------------------------------------------------
// Control column (label + content frame)
// ---------------------------------------------------------------------------

/// Standard per-control column width for slider-based layouts.
pub const COL_WIDTH: f32 = 104.0;

/// Framed column with a bold label and optional sub-label above the content.
pub fn control_column<R>(
    ui: &mut egui::Ui,
    label: &str,
    sub: &str,
    panel_color: Color32,
    border_color: Color32,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let mut out = None;
    egui::Frame::group(ui.style())
        .fill(panel_color)
        .stroke(Stroke::new(1.0, border_color))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_min_width(COL_WIDTH);
                ui.set_max_width(COL_WIDTH);
                ui.spacing_mut().slider_width = COL_WIDTH - 14.0;
                ui.label(
                    egui::RichText::new(label)
                        .strong()
                        .size(11.0)
                        .color(Color32::from_rgb(0xcc, 0xcc, 0xd0)),
                );
                if !sub.is_empty() {
                    ui.label(
                        egui::RichText::new(sub)
                            .size(9.0)
                            .color(Color32::from_rgb(0x88, 0x88, 0x90)),
                    );
                }
                ui.add_space(2.0);
                out = Some(contents(ui));
            });
        });
    ui.add_space(4.0);
    out.expect("control_column closure always runs")
}
