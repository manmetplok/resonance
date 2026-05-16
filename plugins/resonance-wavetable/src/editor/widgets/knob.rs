//! Rotary knob widget — accent arc, value readout, label.
//!
//! Drawn as a circular dial with a sweep from −135° (min) to +135° (max).
//! Bipolar knobs centre at the 12-o'clock position and fill outward from
//! there in either accent (positive) or warm (negative).
//!
//! Vertical drag adjusts the value; Shift slows the drag for fine
//! adjustment; double-click resets to the supplied default.

use std::f32::consts::PI;
use wayland_plugin_gui::egui;

use crate::editor::theme;

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
    draw(ui, label, value, formatted_value, default, false)
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
    let new = draw(ui, label, unit, formatted_value, default_unit, true)?;
    Some(new * 2.0 - 1.0)
}

fn draw(
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

// Silence unused PI import warning if egui already pulls it in elsewhere.
const _: f32 = PI;
