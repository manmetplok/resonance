//! The lavender widget kit of the redesigned control strip (ba todo
//! #1138, design doc #264 req-3/req-5): macro (56 px) and texture
//! (38 px) knobs — hierarchy by size, not chrome — with bipolar
//! centre-out arcs and the feedback knob's warm over-unity zone;
//! horizontal/vertical segmented controls; the division stepper that
//! swaps in-place for the Time macro knob; chip toggles; and the large
//! amber freeze latch. Plus the legacy toggle/combo bindings the
//! grouped strip still uses until the signal-flow layout todo lands.
//!
//! Every setter goes through `Param::set_plain`, the same GUI→host
//! path the other editors use (the CLAP bridge picks the new value up
//! and emits the host param event), so host automation and editor
//! edits stay consistent. All colors are design-system v1 lavender
//! tokens (alpha shaping via `gamma_multiply` only — no new raw
//! colors).

use egui::Ui;
use wayland_plugin_gui::egui;

use crate::params::GranularDelayParams;
use crate::sync::DIVISION_LABELS;

use super::theme;

/// Macro-tier knob diameter (Time, Size, Density, Pitch, Feedback, Mix).
pub const MACRO_KNOB_SIZE: f32 = 56.0;
/// Texture-tier knob diameter (everything else).
pub const TEXTURE_KNOB_SIZE: f32 = 38.0;

/// Extra cell height under a knob for the value + label lines.
const KNOB_TEXT_H: f32 = 30.0;

/// The full cell size a knob of `size` occupies (the division stepper
/// matches this so the sync swap causes no layout jump).
pub fn knob_cell(size: f32) -> egui::Vec2 {
    egui::vec2(size + 12.0, size + KNOB_TEXT_H)
}

// ---------------------------------------------------------------------------
// Knobs
// ---------------------------------------------------------------------------

/// Macro-tier (56 px) knob bound to a param; bipolar centre-out arc
/// when the param range spans zero (Pitch).
pub fn macro_knob(ui: &mut Ui, params: &GranularDelayParams, index: usize) {
    knob_param(ui, params, index, MACRO_KNOB_SIZE, None, None);
}

/// Texture-tier (38 px) knob bound to a param.
pub fn texture_knob(ui: &mut Ui, params: &GranularDelayParams, index: usize) {
    knob_param(ui, params, index, TEXTURE_KNOB_SIZE, None, None);
}

/// Texture-tier knob with a custom (short) caption for the 38 px cell
/// (e.g. `SIZE J` instead of the param's full `Size Jitter`).
pub fn texture_knob_labeled(
    ui: &mut Ui,
    params: &GranularDelayParams,
    index: usize,
    label: &str,
) {
    knob_param(ui, params, index, TEXTURE_KNOB_SIZE, None, Some(label));
}

/// Macro knob with the 100–110 % over-unity arc zone marked in the
/// warm token (the Feedback knob, design doc #264 req-5).
pub fn feedback_knob(ui: &mut Ui, params: &GranularDelayParams, index: usize) {
    let p = params.param_at(index);
    let min = p.min_plain() as f32;
    let span = (p.max_plain() as f32 - min).max(f32::EPSILON);
    // Over-unity begins at plain 1.0 (100 %).
    let warm_from = ((1.0 - min) / span).clamp(0.0, 1.0);
    knob_param(ui, params, index, MACRO_KNOB_SIZE, Some(warm_from), None);
}

/// Shared param-bound knob body: linear plain↔unit mapping, drag to
/// edit (Shift = fine), double-click resets to the param default.
fn knob_param(
    ui: &mut Ui,
    params: &GranularDelayParams,
    index: usize,
    size: f32,
    warm_from: Option<f32>,
    label: Option<&str>,
) {
    let p = params.param_at(index);
    let min = p.min_plain() as f32;
    let max = p.max_plain() as f32;
    let span = (max - min).max(f32::EPSILON);
    let val = p.get_plain() as f32;
    let display = p.display(val as f64);
    let bipolar = min < 0.0 && max > 0.0;

    if let Some(unit) = draw_knob(
        ui,
        label.unwrap_or(p.name()),
        (val - min) / span,
        &display,
        (p.default_plain() as f32 - min) / span,
        size,
        bipolar,
        warm_from,
    ) {
        p.set_plain(f64::from((min + unit * span).clamp(min, max)));
    }
}

/// Painter-drawn lavender knob (adapted from
/// `wayland_plugin_gui::widgets`' themed knob, parameterized by size
/// and the warm over-unity zone). Returns the new unit value on edit.
#[allow(clippy::too_many_arguments)]
fn draw_knob(
    ui: &mut Ui,
    label: &str,
    value_unit: f32,
    formatted_value: &str,
    default_unit: f32,
    size: f32,
    bipolar: bool,
    warm_from: Option<f32>,
) -> Option<f32> {
    let cell = knob_cell(size);
    let (rect, response) = ui.allocate_exact_size(cell, egui::Sense::click_and_drag());
    if !ui.is_rect_visible(rect) {
        return knob_input(ui, &response, value_unit, default_unit);
    }

    let center = egui::pos2(rect.center().x, rect.top() + size * 0.5 + 1.0);
    let painter = ui.painter_at(rect);
    let radius = size * 0.5 - 2.0;
    let macro_tier = size >= MACRO_KNOB_SIZE - f32::EPSILON;

    painter.circle_filled(center, radius, theme::BG_1);
    painter.circle_stroke(center, radius, egui::Stroke::new(1.0, theme::LINE_2));

    // Track arc (dim), then the warm over-unity zone marking on top.
    let arc_r = radius - 3.0;
    arc(&painter, center, arc_r, -135.0, 135.0, theme::LINE, 2.0);
    if let Some(f) = warm_from {
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
    let unit = value_unit.clamp(0.0, 1.0);
    if bipolar {
        // Fill from the 12-o'clock centre outward: accent positive,
        // warm negative (matches the shared lavender helper).
        let target_deg = (unit - 0.5) * 270.0;
        let (start, end, color) = if target_deg >= 0.0 {
            (0.0, target_deg, theme::ACCENT)
        } else {
            (target_deg, 0.0, theme::WARM)
        };
        arc(&painter, center, arc_r, start, end, color, 2.4);
        let (sx, sy) = polar(center, radius - 6.0, 0.0);
        let (ex, ey) = polar(center, radius - 1.0, 0.0);
        painter.line_segment(
            [egui::pos2(sx, sy), egui::pos2(ex, ey)],
            egui::Stroke::new(1.0, theme::TEXT_4),
        );
    } else {
        let target_deg = -135.0 + unit * 270.0;
        match warm_from {
            // Split the active arc at the over-unity boundary: accent
            // below 100 %, warm beyond.
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
    let (ix, iy) = (
        center.x + angle.sin() * radius * 0.32,
        center.y - angle.cos() * radius * 0.32,
    );
    let (ox, oy) = (
        center.x + angle.sin() * (radius - 5.0),
        center.y - angle.cos() * (radius - 5.0),
    );
    painter.line_segment(
        [egui::pos2(ix, iy), egui::pos2(ox, oy)],
        egui::Stroke::new(1.6, theme::TEXT_1),
    );

    if response.hovered() {
        painter.circle_stroke(
            center,
            radius + 1.0,
            egui::Stroke::new(1.0, theme::ACCENT_SOFT),
        );
    }

    // Value + label below (smaller type on the texture tier).
    let (val_font, lab_font) = if macro_tier { (10.5, 9.0) } else { (9.5, 8.0) };
    painter.text(
        egui::pos2(rect.center().x, rect.top() + size + 4.0),
        egui::Align2::CENTER_TOP,
        formatted_value,
        egui::FontId::monospace(val_font),
        theme::TEXT_1,
    );
    painter.text(
        egui::pos2(rect.center().x, rect.top() + size + 17.0),
        egui::Align2::CENTER_TOP,
        label.to_uppercase(),
        egui::FontId::proportional(lab_font),
        theme::TEXT_3,
    );

    knob_input(ui, &response, unit, default_unit)
}

fn knob_input(
    ui: &Ui,
    response: &egui::Response,
    unit: f32,
    default_unit: f32,
) -> Option<f32> {
    if response.double_clicked() {
        return Some(default_unit.clamp(0.0, 1.0));
    }
    if response.dragged() {
        let drag = response.drag_delta().y;
        if drag != 0.0 {
            let speed = if ui.input(|i| i.modifiers.shift) {
                0.002
            } else {
                0.008
            };
            return Some((unit - drag * speed).clamp(0.0, 1.0));
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
    let mut points: Vec<egui::Pos2> = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let deg = a + (b - a) * t;
        let rad = deg.to_radians();
        // 0° = 12 o'clock, sweeping clockwise.
        points.push(egui::pos2(
            center.x + rad.sin() * radius,
            center.y - rad.cos() * radius,
        ));
    }
    painter.add(egui::Shape::line(points, egui::Stroke::new(stroke, color)));
}

fn polar(center: egui::Pos2, radius: f32, deg: f32) -> (f32, f32) {
    let rad = deg.to_radians();
    (center.x + rad.sin() * radius, center.y - rad.cos() * radius)
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
// Division stepper
// ---------------------------------------------------------------------------

/// Division stepper `‹ 1/4T ›` bound to the division param, with a
/// live `= 333.3 ms` readout computed from the viz BPM. Occupies the
/// same cell as the Time macro knob so the sync disclosure swaps them
/// in-place with no layout jump (design doc #264 req-4).
pub fn division_stepper(ui: &mut Ui, params: &GranularDelayParams, index: usize, bpm: f32) {
    let p = params.param_at(index);
    let max = DIVISION_LABELS.len() as i64 - 1;
    let current = (p.get_plain().round() as i64).clamp(0, max);
    let cell = knob_cell(MACRO_KNOB_SIZE);
    let (rect, _) = ui.allocate_exact_size(cell, egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }

    // Frame the stepper where the knob circle would sit.
    let body = egui::Rect::from_min_size(
        egui::pos2(rect.left(), rect.top() + 6.0),
        egui::vec2(cell.x, MACRO_KNOB_SIZE - 12.0),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(body, theme::RADIUS_CHIP, theme::BG_1);
    painter.rect_stroke(
        body,
        theme::RADIUS_CHIP,
        egui::Stroke::new(1.0, theme::LINE_2),
        egui::StrokeKind::Inside,
    );

    // ‹ / › hit zones at the sides, division label centred.
    let btn_w = 16.0;
    let prev_rect = egui::Rect::from_min_max(
        body.min,
        egui::pos2(body.left() + btn_w, body.bottom()),
    );
    let next_rect = egui::Rect::from_min_max(
        egui::pos2(body.right() - btn_w, body.top()),
        body.max,
    );
    let prev = ui.interact(prev_rect, ui.id().with((p.id(), "prev")), egui::Sense::click());
    let next = ui.interact(next_rect, ui.id().with((p.id(), "next")), egui::Sense::click());
    let arrow_color = |r: &egui::Response, on: bool| {
        if !on {
            theme::TEXT_4
        } else if r.hovered() {
            theme::ACCENT_SOFT
        } else {
            theme::TEXT_2
        }
    };
    painter.text(
        prev_rect.center(),
        egui::Align2::CENTER_CENTER,
        "‹",
        egui::FontId::proportional(13.0),
        arrow_color(&prev, current > 0),
    );
    painter.text(
        next_rect.center(),
        egui::Align2::CENTER_CENTER,
        "›",
        egui::FontId::proportional(13.0),
        arrow_color(&next, current < max),
    );
    painter.text(
        body.center(),
        egui::Align2::CENTER_CENTER,
        DIVISION_LABELS[current as usize],
        egui::FontId::monospace(12.0),
        theme::ACCENT_SOFT,
    );

    // Live effective-time readout from the host tempo (— without one).
    let readout = if bpm > 0.0 {
        let tempo = resonance_plugin::TempoInfo {
            bpm,
            time_sig_num: 4,
            time_sig_den: 4,
            playing: false,
            song_pos_beats: 0.0,
        };
        let seconds = crate::sync::delay_seconds(
            true,
            current as usize,
            0.0,
            Some(tempo),
            crate::dsp::MAX_DELAY_SECONDS,
        );
        format!("= {:.1} ms", seconds * 1000.0)
    } else {
        "= — ms".to_string()
    };
    painter.text(
        egui::pos2(rect.center().x, rect.top() + MACRO_KNOB_SIZE + 4.0),
        egui::Align2::CENTER_TOP,
        readout,
        egui::FontId::monospace(10.5),
        theme::TEXT_1,
    );
    painter.text(
        egui::pos2(rect.center().x, rect.top() + MACRO_KNOB_SIZE + 17.0),
        egui::Align2::CENTER_TOP,
        p.name().to_uppercase(),
        egui::FontId::proportional(9.0),
        theme::TEXT_3,
    );

    if prev.clicked() && current > 0 {
        p.set_plain((current - 1) as f64);
    }
    if next.clicked() && current < max {
        p.set_plain((current + 1) as f64);
    }
}

// ---------------------------------------------------------------------------
// Chips + freeze latch
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

/// The large amber freeze latch (design doc #264 req-5): warm-token
/// latching button with a glow/filled state while engaged, sized for
/// the OUTPUT group.
pub fn freeze_latch(ui: &mut Ui, params: &GranularDelayParams, index: usize) {
    let p = params.param_at(index);
    let on = p.get_plain() >= 0.5;
    let size = egui::vec2(96.0, 44.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        // Glow halo outside the button while engaged.
        if on {
            let painter = ui.painter();
            for (expand, alpha) in [(5.0, 0.10), (3.0, 0.18), (1.5, 0.30)] {
                painter.rect_stroke(
                    rect.expand(expand),
                    theme::RADIUS_CHIP + expand,
                    egui::Stroke::new(2.0, theme::WARM.gamma_multiply(alpha)),
                    egui::StrokeKind::Outside,
                );
            }
        }
        let painter = ui.painter_at(rect.expand(1.0));
        let (fill, text_color, stroke) = if on {
            (theme::WARM, theme::BG_0, theme::WARM)
        } else if response.hovered() {
            (theme::BG_3, theme::WARM, theme::WARM.gamma_multiply(0.6))
        } else {
            (theme::BG_1, theme::WARM.gamma_multiply(0.8), theme::LINE)
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
            "FREEZE",
            egui::FontId::proportional(12.0),
            text_color,
        );
    }
    if response.clicked() {
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
