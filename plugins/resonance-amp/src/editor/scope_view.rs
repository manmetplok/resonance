//! Dual-trace oscilloscope. Snapshots the rolling scope history from
//! `AmpViz` once per frame and draws two overlaid waveforms:
//!
//! - A dim translucent "input" trace (pre-gain dry signal).
//! - A brighter "output" trace (post-model, post-gain).
//!
//! Auto-gain normalises to the larger of the two peaks so a quiet
//! signal still fills the viewport. A soft centre-line grid helps
//! the eye align the waveform.

use wayland_plugin_gui::egui;

use crate::viz::{AmpViz, SCOPE_LEN};

use super::theme;

pub fn draw(painter: &egui::Painter, rect: egui::Rect, viz: &AmpViz) {
    painter.rect_filled(rect, 3.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        3.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    let inner = rect.shrink(10.0);
    if inner.width() <= 20.0 || inner.height() <= 20.0 {
        return;
    }

    painter.text(
        egui::pos2(inner.left(), rect.top() + 6.0),
        egui::Align2::LEFT_TOP,
        "SCOPE",
        egui::FontId::proportional(11.0),
        theme::TEXT_DIM,
    );

    draw_grid(painter, inner);

    // Snapshot the scope ring.
    let mut input_buf = [0.0f32; SCOPE_LEN];
    let mut output_buf = [0.0f32; SCOPE_LEN];
    {
        let scope = viz.scope.lock();
        for (i, (in_s, out_s)) in scope.iter_chrono().enumerate() {
            input_buf[i] = in_s;
            output_buf[i] = out_s;
        }
    }

    // Auto-gain: fit both traces to ~85% of the panel height, but
    // never zoom closer than a floor so silence doesn't explode noise.
    let peak = input_buf
        .iter()
        .chain(output_buf.iter())
        .fold(0.0f32, |m, &v| m.max(v.abs()))
        .max(0.05);
    let scale = 0.85 / peak;

    draw_trace(painter, inner, &input_buf, scale, theme::SCOPE_IN, 1.0);
    draw_trace(painter, inner, &output_buf, scale, theme::SCOPE_OUT, 1.4);
}

fn draw_grid(painter: &egui::Painter, rect: egui::Rect) {
    // Centre horizontal axis.
    let cy = rect.center().y;
    painter.line_segment(
        [egui::pos2(rect.left(), cy), egui::pos2(rect.right(), cy)],
        egui::Stroke::new(0.8, theme::BORDER),
    );

    // Faint half-way guides above and below.
    for frac in &[0.25f32, 0.75] {
        let y = rect.top() + rect.height() * frac;
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(0.3, theme::BORDER),
        );
    }

    // Vertical ruler every quarter of the width.
    for i in 1..4 {
        let x = rect.left() + rect.width() * (i as f32 / 4.0);
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(0.3, theme::BORDER),
        );
    }
}

fn draw_trace(
    painter: &egui::Painter,
    rect: egui::Rect,
    samples: &[f32; SCOPE_LEN],
    scale: f32,
    color: egui::Color32,
    stroke_width: f32,
) {
    let cy = rect.center().y;
    let half_h = rect.height() * 0.5;

    // Down-sample to ~2 points per pixel so the line render stays cheap
    // on wide windows. 512 vertices is plenty for this window size.
    let target_points = (rect.width() * 0.5).clamp(64.0, 512.0) as usize;
    let step = (SCOPE_LEN / target_points).max(1);

    let mut pts = Vec::with_capacity(target_points + 1);
    let mut i = 0;
    while i < SCOPE_LEN {
        let t = i as f32 / (SCOPE_LEN - 1) as f32;
        let x = rect.left() + t * rect.width();
        let s = samples[i] * scale;
        let y = cy - s.clamp(-1.0, 1.0) * half_h;
        pts.push(egui::pos2(x, y));
        i += step;
    }

    painter.add(egui::Shape::line(pts, egui::Stroke::new(stroke_width, color)));
}
