//! Reusable egui widgets for plugin editors.
//!
//! Provides two rotary-knob families so plugin UIs share a consistent
//! look and compact layout: a range-mapped [`knob`] (classic palette)
//! and the theme-driven [`knob_themed`] family (lavender palette), of
//! which [`knob_unipolar`] / [`knob_bipolar`] are the default-styled
//! shorthands. Everything here is pure egui — helpers that bind these
//! widgets to plugin parameter types live downstream in
//! `resonance_plugin::editor_widgets`, keeping this crate free of
//! plugin-framework dependencies.
//!
//! Both families drag through [`knob_drag_unit`], so every knob in
//! every plugin answers the same mouse gesture the same way. A plugin
//! that needs a different size, a bipolar centre tick or an extra arc
//! zone configures [`ThemedKnob`] / [`KnobStyle`] rather than forking
//! the widget (ba todo #1266).
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

// ---------------------------------------------------------------------------
// Knob drag feel — one rule for the whole platform
// ---------------------------------------------------------------------------

/// Vertical drag sensitivity of every knob in every plugin editor, in
/// unit value (`0..1`) per pixel: a full-scale sweep is 200 px of drag.
pub const KNOB_DRAG_SPEED: f32 = 0.005;

/// Drag sensitivity while Shift is held (fine adjust), same units as
/// [`KNOB_DRAG_SPEED`]: 1000 px per full-scale sweep, five times finer
/// than a normal drag.
///
/// It has to be *several* times finer to be worth a modifier: ba todo
/// #1266 first set this to 0.004, which is only 20% finer than the
/// 0.005 normal speed and reads as the same gesture. 0.004 was the
/// shared kit's old *logarithmic-knob* sensitivity, never a fine-drag
/// value — the kit had no fine mode before #1266 added one.
pub const KNOB_DRAG_SPEED_FINE: f32 = 0.001;

/// Apply one frame of vertical drag to a knob value in unit (`0..1`)
/// space, and clamp.
///
/// `drag_y` is egui's raw `drag_delta().y` — positive is downward
/// pointer motion, which lowers the value. `fine` is the Shift
/// modifier.
///
/// Both knob families route their drag through here on purpose:
/// several first-party plugins are hosted in the same GUI runtime, so
/// the same gesture has to produce the same value change in all of
/// them. The chosen numbers are [`KNOB_DRAG_SPEED`] = 0.005 per pixel
/// and [`KNOB_DRAG_SPEED_FINE`] = 0.001 per pixel with Shift; before
/// ba todo #1266 the granular-delay editor carried a forked handler at
/// 0.008 / 0.002 and answered the same drag differently.
pub fn knob_drag_unit(unit: f32, drag_y: f32, fine: bool) -> f32 {
    let speed = if fine {
        KNOB_DRAG_SPEED_FINE
    } else {
        KNOB_DRAG_SPEED
    };
    (unit - drag_y * speed).clamp(0.0, 1.0)
}

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
    // Double-click to reset to default.
    if response.double_clicked() {
        *value = default;
        return true;
    }

    if !response.dragged() {
        return false;
    }
    let drag_y = response.drag_delta().y;
    if drag_y == 0.0 {
        return false;
    }
    let fine = response.ctx.input(|i| i.modifiers.shift);

    // Drag happens in unit space (shared with the themed knob), then
    // maps back onto the value range — linearly, or along the log axis.
    if logarithmic {
        let min = range.start().max(0.001);
        let log_min = min.ln();
        let log_span = range.end().ln() - log_min;
        let unit = if log_span.abs() < f32::EPSILON {
            0.0
        } else {
            (value.max(min).ln() - log_min) / log_span
        };
        *value = (log_min + knob_drag_unit(unit, drag_y, fine) * log_span).exp();
    } else {
        let unit = knob_drag_unit(normalize(*value, range), drag_y, fine);
        *value = range.start() + unit * (range.end() - range.start());
    }
    *value = value.clamp(*range.start(), *range.end());
    true
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

/// Geometry and type scale of one themed knob cell.
///
/// The dial is centred in the cell's top `diameter` px; the value and
/// label rows sit underneath it. A plugin that needs a different knob
/// size or type scale picks a style — it does not fork the widget.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KnobStyle {
    /// Dial diameter, px.
    pub diameter: f32,
    /// Horizontal padding added to the diameter to form the cell.
    pub pad_x: f32,
    /// Cell height added under the dial for the value + label rows.
    pub text_h: f32,
    /// Top of the value row, px below the dial.
    pub value_dy: f32,
    /// Top of the label row, px below the dial.
    pub label_dy: f32,
    /// Value row font size (monospace).
    pub value_font: f32,
    /// Label row font size (proportional).
    pub label_font: f32,
    /// The indicator line stops this far inside the dial edge.
    pub indicator_inset: f32,
}

impl KnobStyle {
    /// The default 52 px lavender cell used by [`knob_unipolar`] and
    /// [`knob_bipolar`].
    pub const LAVENDER: Self = Self {
        diameter: SIZE,
        pad_x: 8.0,
        text_h: CELL_H - SIZE,
        value_dy: 5.0,
        label_dy: 19.0,
        value_font: 10.5,
        label_font: 9.0,
        indicator_inset: 6.0,
    };

    /// The full cell this style occupies (dial + text rows). Callers
    /// that swap another widget in for a knob (e.g. a stepper) allocate
    /// this exact size so the swap causes no layout jump.
    pub fn cell(&self) -> Vec2 {
        Vec2::new(self.diameter + self.pad_x, self.diameter + self.text_h)
    }
}

impl Default for KnobStyle {
    fn default() -> Self {
        Self::LAVENDER
    }
}

/// One themed knob cell, in unit (`0..1`) value space.
///
/// Build with [`ThemedKnob::new`] and the modifiers below, then draw
/// with [`knob_themed`].
#[derive(Debug, Clone, Copy)]
pub struct ThemedKnob<'a> {
    /// Caption under the value row (rendered upper-case).
    pub label: &'a str,
    /// Current value, `0..1`.
    pub value_unit: f32,
    /// Pre-formatted value text (the widget never formats values — the
    /// plugin's parameter knows its own units).
    pub formatted_value: &'a str,
    /// Value a double-click resets to, `0..1`.
    pub default_unit: f32,
    /// Fill the active arc outward from the 12-o'clock centre, with a
    /// centre tick (pitch-style parameters).
    pub bipolar: bool,
    /// Unit position where a warm "over-unity" zone starts: the track
    /// is marked from there to full scale, and the active arc switches
    /// to the warm token beyond it (the granular delay's >100 %
    /// feedback region). Ignored when `bipolar`.
    pub warm_from: Option<f32>,
    /// Cell geometry and type scale.
    pub style: KnobStyle,
}

impl<'a> ThemedKnob<'a> {
    /// A default-styled unipolar knob.
    pub fn new(label: &'a str, value_unit: f32, formatted_value: &'a str, default_unit: f32) -> Self {
        Self {
            label,
            value_unit,
            formatted_value,
            default_unit,
            bipolar: false,
            warm_from: None,
            style: KnobStyle::LAVENDER,
        }
    }

    /// Fill outward from the centre instead of from the minimum.
    pub fn bipolar(mut self, bipolar: bool) -> Self {
        self.bipolar = bipolar;
        self
    }

    /// Mark an over-unity zone starting at this unit position.
    pub fn warm_from(mut self, warm_from: Option<f32>) -> Self {
        self.warm_from = warm_from;
        self
    }

    /// Use a non-default cell geometry / type scale.
    pub fn style(mut self, style: KnobStyle) -> Self {
        self.style = style;
        self
    }
}

/// Unipolar knob driving a 0..1 value. Returns the new value if changed.
pub fn knob_unipolar(
    ui: &mut egui::Ui,
    label: &str,
    value: f32, // 0..1
    formatted_value: &str,
    default: f32,
) -> Option<f32> {
    knob_themed(ui, &ThemedKnob::new(label, value, formatted_value, default))
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
    let knob = ThemedKnob::new(label, unit, formatted_value, default_unit).bipolar(true);
    let new = knob_themed(ui, &knob)?;
    Some(new * 2.0 - 1.0)
}

/// Draw a configured themed knob and handle its input. Returns the new
/// unit value when the drag or a double-click changed it.
pub fn knob_themed(ui: &mut egui::Ui, knob: &ThemedKnob<'_>) -> Option<f32> {
    let style = knob.style;
    let (rect, response) = ui.allocate_exact_size(style.cell(), egui::Sense::click_and_drag());
    let unit = knob.value_unit.clamp(0.0, 1.0);
    if !ui.is_rect_visible(rect) {
        return themed_knob_input(&response, unit, knob.default_unit);
    }

    let center = egui::pos2(rect.center().x, rect.top() + style.diameter * 0.5 + 1.0);
    let radius = style.diameter * 0.5 - 2.0;
    let painter = ui.painter_at(rect);

    // Dial face.
    painter.circle_filled(center, radius, theme::BG_1);
    painter.circle_stroke(center, radius, egui::Stroke::new(1.0, theme::LINE_2));

    // Track arc background (dim), then the over-unity zone marking.
    let arc_r = radius - 3.0;
    arc(&painter, center, arc_r, -135.0, 135.0, theme::LINE, 2.0);
    if let Some(f) = knob.warm_from {
        let from_deg = -135.0 + f.clamp(0.0, 1.0) * 270.0;
        arc(
            &painter,
            center,
            arc_r,
            from_deg,
            135.0,
            theme::WARM.gamma_multiply(0.45),
            2.0,
        );
    }

    // Active arc.
    if knob.bipolar {
        // Fill from centre (12 o'clock = 0°) outwards.
        let centre_deg = 0.0;
        let target_deg = (unit - 0.5) * 2.0 * 135.0;
        let (start, end, color) = if target_deg >= 0.0 {
            (centre_deg, target_deg, theme::ACCENT)
        } else {
            (target_deg, centre_deg, theme::WARM)
        };
        arc(&painter, center, arc_r, start, end, color, 2.4);
        // Centre tick.
        let (sx, sy) = polar(center, radius - 6.0, 0.0);
        let (ex, ey) = polar(center, radius - 1.0, 0.0);
        painter.line_segment(
            [egui::pos2(sx, sy), egui::pos2(ex, ey)],
            egui::Stroke::new(1.0, theme::TEXT_4),
        );
    } else {
        let target_deg = -135.0 + unit * 270.0;
        match knob.warm_from {
            // Split the active arc at the over-unity boundary: accent
            // below it, warm beyond.
            Some(f) if unit > f => {
                let split_deg = -135.0 + f.clamp(0.0, 1.0) * 270.0;
                arc(&painter, center, arc_r, -135.0, split_deg, theme::ACCENT, 2.4);
                arc(&painter, center, arc_r, split_deg, target_deg, theme::WARM, 2.4);
            }
            _ => arc(&painter, center, arc_r, -135.0, target_deg, theme::ACCENT, 2.4),
        }
    }

    // Indicator line.
    let angle = (-135.0 + unit * 270.0).to_radians();
    let inner = radius * 0.32;
    let outer = radius - style.indicator_inset;
    painter.line_segment(
        [
            egui::pos2(
                center.x + angle.sin() * inner,
                center.y - angle.cos() * inner,
            ),
            egui::pos2(
                center.x + angle.sin() * outer,
                center.y - angle.cos() * outer,
            ),
        ],
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
    let text_top = rect.top() + style.diameter;
    painter.text(
        egui::pos2(rect.center().x, text_top + style.value_dy),
        egui::Align2::CENTER_TOP,
        knob.formatted_value,
        egui::FontId::monospace(style.value_font),
        theme::TEXT_1,
    );
    painter.text(
        egui::pos2(rect.center().x, text_top + style.label_dy),
        egui::Align2::CENTER_TOP,
        knob.label.to_uppercase(),
        egui::FontId::proportional(style.label_font),
        theme::TEXT_3,
    );

    themed_knob_input(&response, unit, knob.default_unit)
}

/// Vertical drag / double-click handling of a themed knob, in unit
/// space. Shares [`knob_drag_unit`] with the classic knob family.
fn themed_knob_input(response: &Response, unit: f32, default_unit: f32) -> Option<f32> {
    if response.double_clicked() {
        return Some(default_unit.clamp(0.0, 1.0));
    }
    if response.dragged() {
        let drag_y = response.drag_delta().y;
        if drag_y != 0.0 {
            let fine = response.ctx.input(|i| i.modifiers.shift);
            return Some(knob_drag_unit(unit, drag_y, fine));
        }
    }
    None
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
    // One owned, exactly-sized Vec per arc is the floor here, same as
    // `draw_arc` above: epaint's `PathShape` owns `points: Vec<Pos2>`,
    // so the polyline has to be handed over by value — a reused scratch
    // buffer (or a `SmallVec`) would be copied into a fresh Vec anyway.
    // What it does buy is one `Shape` per arc instead of one per
    // segment, with proper joins.
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
