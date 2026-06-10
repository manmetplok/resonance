//! Reusable egui widgets for plugin editors.
//!
//! Provides two rotary-knob families so plugin UIs share a consistent
//! look and compact layout: a range-mapped [`knob`] (classic palette)
//! and the theme-driven [`knob_unipolar`] / [`knob_bipolar`] pair
//! (lavender palette). Everything here is pure egui — helpers that bind
//! these widgets to plugin parameter types live downstream in
//! `resonance_plugin::editor_widgets`, keeping this crate free of
//! plugin-framework dependencies.
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
// Theme-driven rotary knob (lavender palette) — accent arc, value readout,
// label.
//
// Drawn as a circular dial with a sweep from −135° (min) to +135° (max).
// Bipolar knobs centre at the 12-o'clock position and fill outward from
// there in either accent (positive) or warm (negative).
//
// Vertical drag adjusts the value; Shift slows the drag for fine
// adjustment; double-click resets to the supplied default.
// ---------------------------------------------------------------------------

use crate::theme::lavender as theme;

/// One cell: knob + label + readout. Size is `SIZE` x `CELL_H` including the
/// label/value lines below.
const SIZE: f32 = 52.0;
const CELL_H: f32 = SIZE + 32.0;

/// Unipolar knob driving a 0..1 value. Returns the new value if changed.
pub fn knob_unipolar(
    ui: &mut egui::Ui,
    label: &str,
    value: f32, // 0..1
    formatted_value: &str,
    default: f32,
) -> Option<f32> {
    draw_themed_knob(ui, label, value, formatted_value, default, false)
}

/// Bipolar knob driving a -1..1 value (or any range mapped to that). Returns
/// the new value if changed.
pub fn knob_bipolar(
    ui: &mut egui::Ui,
    label: &str,
    value: f32, // -1..1
    formatted_value: &str,
    default: f32,
) -> Option<f32> {
    // Map -1..1 to 0..1 for arc geometry.
    let unit = (value + 1.0) * 0.5;
    let default_unit = (default + 1.0) * 0.5;
    let new = draw_themed_knob(ui, label, unit, formatted_value, default_unit, true)?;
    Some(new * 2.0 - 1.0)
}

fn draw_themed_knob(
    ui: &mut egui::Ui,
    label: &str,
    value_unit: f32,
    formatted_value: &str,
    default_unit: f32,
    bipolar: bool,
) -> Option<f32> {
    let cell = egui::vec2(SIZE + 8.0, CELL_H);
    let (rect, response) = ui.allocate_exact_size(cell, egui::Sense::click_and_drag());

    let knob_rect = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.top() + SIZE * 0.5 + 1.0),
        egui::vec2(SIZE, SIZE),
    );
    let painter = ui.painter_at(rect);

    // Outer ring (dial face).
    let center = knob_rect.center();
    let radius = SIZE * 0.5 - 2.0;
    painter.circle_filled(center, radius, theme::BG_1);
    painter.circle_stroke(center, radius, egui::Stroke::new(1.0, theme::LINE_2));

    // Track arc background (dim).
    arc(&painter, center, radius - 3.0, -135.0, 135.0, theme::LINE, 2.0);

    // Active arc.
    let unit = value_unit.clamp(0.0, 1.0);
    if bipolar {
        // Fill from centre (12 o'clock = 0°) outwards.
        let centre_deg = 0.0;
        let target_deg = (unit - 0.5) * 2.0 * 135.0;
        let (start, end, color) = if target_deg >= 0.0 {
            (centre_deg, target_deg, theme::ACCENT)
        } else {
            (target_deg, centre_deg, theme::WARM)
        };
        arc(&painter, center, radius - 3.0, start, end, color, 2.4);
        // Centre tick.
        let (sx, sy) = polar(center, radius - 6.0, 0.0);
        let (ex, ey) = polar(center, radius - 1.0, 0.0);
        painter.line_segment(
            [egui::pos2(sx, sy), egui::pos2(ex, ey)],
            egui::Stroke::new(1.0, theme::TEXT_4),
        );
    } else {
        let target_deg = -135.0 + unit * 270.0;
        arc(
            &painter,
            center,
            radius - 3.0,
            -135.0,
            target_deg,
            theme::ACCENT,
            2.4,
        );
    }

    // Indicator line.
    let angle = (-135.0 + unit * 270.0).to_radians();
    let inner = radius * 0.32;
    let outer = radius - 6.0;
    let ix = center.x + angle.sin() * inner;
    let iy = center.y - angle.cos() * inner;
    let ox = center.x + angle.sin() * outer;
    let oy = center.y - angle.cos() * outer;
    painter.line_segment(
        [egui::pos2(ix, iy), egui::pos2(ox, oy)],
        egui::Stroke::new(1.6, theme::TEXT_1),
    );

    // Hover ring.
    if response.hovered() {
        painter.circle_stroke(
            center,
            radius + 1.0,
            egui::Stroke::new(1.0, theme::ACCENT_SOFT),
        );
    }

    // Value + label below.
    let val_pos = egui::pos2(rect.center().x, knob_rect.bottom() + 4.0);
    painter.text(
        val_pos,
        egui::Align2::CENTER_TOP,
        formatted_value,
        egui::FontId::monospace(10.5),
        theme::TEXT_1,
    );
    let lab_pos = egui::pos2(rect.center().x, knob_rect.bottom() + 18.0);
    painter.text(
        lab_pos,
        egui::Align2::CENTER_TOP,
        label.to_uppercase(),
        egui::FontId::proportional(9.0),
        theme::TEXT_3,
    );

    // Interaction: vertical drag changes value.
    let mut new_value: Option<f32> = None;
    if response.dragged() {
        let drag = response.drag_delta().y;
        if drag.abs() > 0.0 {
            let modifiers = ui.input(|i| i.modifiers);
            let speed = if modifiers.shift { 0.002 } else { 0.008 };
            let next = (unit - drag * speed).clamp(0.0, 1.0);
            new_value = Some(next);
        }
    }
    if response.double_clicked() {
        new_value = Some(default_unit.clamp(0.0, 1.0));
    }
    new_value
}

fn arc(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    start_deg: f32,
    end_deg: f32,
    color: egui::Color32,
    stroke: f32,
) {
    let (a, b) = if start_deg <= end_deg {
        (start_deg, end_deg)
    } else {
        (end_deg, start_deg)
    };
    if (b - a).abs() < 0.1 {
        return;
    }
    let steps = (((b - a).abs() / 5.0).ceil() as usize).max(2);
    let mut points: Vec<egui::Pos2> = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let deg = a + (b - a) * t;
        let rad = deg.to_radians();
        // 0° = 12 o'clock, sweeping clockwise.
        let x = center.x + rad.sin() * radius;
        let y = center.y - rad.cos() * radius;
        points.push(egui::pos2(x, y));
    }
    painter.add(egui::Shape::line(points, egui::Stroke::new(stroke, color)));
}

fn polar(center: egui::Pos2, radius: f32, deg: f32) -> (f32, f32) {
    let rad = deg.to_radians();
    (center.x + rad.sin() * radius, center.y - rad.cos() * radius)
}
