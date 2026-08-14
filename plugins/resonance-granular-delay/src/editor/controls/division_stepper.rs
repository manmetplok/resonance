//! The division stepper of the TIME group (design doc #264 req-4).
//!
//! A granular-specific composite, not a generic widget: it borrows the
//! macro-knob cell so the SYNC disclosure can swap it in for the Time
//! knob with no layout jump, and it reads the host tempo to show what
//! the selected division actually costs in milliseconds.

use egui::Ui;
use wayland_plugin_gui::egui;

use crate::params::GranularDelayParams;
use crate::sync::{self, DIVISION_LABELS};

use super::super::theme;
use super::super::widgets::MACRO_KNOB_STYLE;

/// Division stepper `‹ 1/4T ›` bound to the division param, with a
/// live `= 333.3 ms` readout computed from the viz BPM. Occupies the
/// same cell as the Time macro knob so the sync disclosure swaps them
/// in-place with no layout jump (design doc #264 req-4).
pub fn division_stepper(ui: &mut Ui, params: &GranularDelayParams, index: usize, bpm: f32) {
    let p = params.param_at(index);
    let max = DIVISION_LABELS.len() as i64 - 1;
    let current = (p.get_plain().round() as i64).clamp(0, max);
    let cell = MACRO_KNOB_STYLE.cell();
    let (rect, _) = ui.allocate_exact_size(cell, egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }

    // Frame the stepper where the knob circle would sit.
    let body = egui::Rect::from_min_size(
        egui::pos2(rect.left(), rect.top() + 6.0),
        egui::vec2(cell.x, MACRO_KNOB_STYLE.diameter - 12.0),
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

    // Live effective-time readout from the host tempo (— without one),
    // clamped the way the delay line clamps it so the number is what
    // the DSP will actually play.
    let readout = if bpm > 0.0 {
        let seconds = sync::clamp_delay_seconds(
            sync::division_seconds(bpm, current as usize),
            crate::dsp::MAX_DELAY_SECONDS,
        );
        format!("= {:.1} ms", seconds * 1000.0)
    } else {
        "= — ms".to_string()
    };
    painter.text(
        egui::pos2(rect.center().x, rect.top() + MACRO_KNOB_STYLE.diameter + 4.0),
        egui::Align2::CENTER_TOP,
        readout,
        egui::FontId::monospace(10.5),
        theme::TEXT_1,
    );
    painter.text(
        egui::pos2(rect.center().x, rect.top() + MACRO_KNOB_STYLE.diameter + 17.0),
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
