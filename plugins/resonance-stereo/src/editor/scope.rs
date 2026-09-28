//! Goniometer + correlation strip, painted straight onto the frame (the
//! egui equivalent of a canvas): no widgets, no per-frame allocation —
//! the point buffer is the editor's reusable scratch `Vec`.
//!
//! The goniometer is the usual M/S view: mid up, side across, so a mono
//! source is a vertical line, a hard-left source the `L` diagonal, and
//! antiphase material a horizontal line. The strip under it is the
//! correlation history, −1 at the bottom, +1 at the top.

use plugin_gui_core::egui;

use crate::viz::StereoViz;

use super::theme;

/// Height of the correlation strip, in points.
pub const STRIP_H: f32 = 46.0;

/// Goniometer position of an `(L, R)` frame: `x` = side (right is
/// positive, i.e. `R` louder), `y` = mid, both scaled by `1/√2` so a
/// full-scale hard-panned sample lands on the unit circle.
pub fn gonio_xy(l: f32, r: f32) -> (f32, f32) {
    let k = std::f32::consts::FRAC_1_SQRT_2;
    ((r - l) * k, (l + r) * k)
}

/// Where a correlation value sits in the strip: 0 at the bottom (−1),
/// 1 at the top (+1). Non-finite reads as 0 correlation.
pub fn strip_fraction(correlation: f32) -> f32 {
    let c = if correlation.is_finite() { correlation } else { 0.0 };
    ((c + 1.0) * 0.5).clamp(0.0, 1.0)
}

/// Colour for a correlation reading: good when positive, a warning
/// between 0 and +0.3, bad below 0 (the §2.2 thresholds).
pub fn correlation_color(correlation: f32) -> egui::Color32 {
    if correlation < 0.0 {
        theme::BAD
    } else if correlation < 0.3 {
        theme::WARM
    } else {
        theme::GOOD
    }
}

pub fn draw(ui: &mut egui::Ui, rect: egui::Rect, viz: &StereoViz, scratch: &mut Vec<(f32, f32)>) {
    let painter = ui.painter_at(rect);
    let gonio = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.max.y - STRIP_H - 6.0));
    let strip = egui::Rect::from_min_max(egui::pos2(rect.min.x, rect.max.y - STRIP_H), rect.max);
    draw_goniometer(&painter, gonio, viz, scratch);
    draw_strip(&painter, strip, viz);
}

fn draw_goniometer(painter: &egui::Painter, rect: egui::Rect, viz: &StereoViz, scratch: &mut Vec<(f32, f32)>) {
    painter.rect_filled(rect, theme::RADIUS_PANEL, theme::BG_2);
    painter.rect_stroke(
        rect,
        theme::RADIUS_PANEL,
        egui::Stroke::new(1.0, theme::LINE),
        egui::StrokeKind::Inside,
    );
    let c = rect.center();
    let radius = 0.5 * rect.width().min(rect.height()) - 12.0;
    if radius <= 4.0 {
        return;
    }
    let grid = egui::Stroke::new(0.6, theme::LINE);
    painter.line_segment([egui::pos2(c.x, c.y - radius), egui::pos2(c.x, c.y + radius)], grid);
    painter.line_segment([egui::pos2(c.x - radius, c.y), egui::pos2(c.x + radius, c.y)], grid);
    let d = radius * std::f32::consts::FRAC_1_SQRT_2;
    painter.line_segment([egui::pos2(c.x - d, c.y - d), egui::pos2(c.x + d, c.y + d)], grid);
    painter.line_segment([egui::pos2(c.x + d, c.y - d), egui::pos2(c.x - d, c.y + d)], grid);
    let label = |pos: egui::Pos2, text: &str| {
        painter.text(pos, egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(9.0), theme::TEXT_3);
    };
    label(egui::pos2(c.x, c.y - radius - 6.0), "M");
    label(egui::pos2(c.x + radius + 6.0, c.y), "S");
    label(egui::pos2(c.x - d - 6.0, c.y - d - 6.0), "L");
    label(egui::pos2(c.x + d + 6.0, c.y - d - 6.0), "R");

    scratch.clear();
    scratch.extend(viz.points());
    // Auto-gain: the loudest point reaches 90 % of the radius, with a
    // floor so near-silence does not blow the noise up to full size.
    let peak = scratch
        .iter()
        .map(|&(l, r)| {
            let (x, y) = gonio_xy(l, r);
            (x * x + y * y).sqrt()
        })
        .filter(|v| v.is_finite())
        .fold(0.0f32, f32::max);
    if peak < 1e-5 {
        return;
    }
    let gain = 0.9 * radius / peak.max(0.05);
    let mut mesh = egui::epaint::Mesh::default();
    let dot = 1.1;
    for &(l, r) in scratch.iter() {
        let (x, y) = gonio_xy(l, r);
        if !(x.is_finite() && y.is_finite()) {
            continue;
        }
        let p = egui::pos2(c.x + x * gain, c.y - y * gain);
        mesh.add_colored_rect(
            egui::Rect::from_center_size(p, egui::vec2(dot * 2.0, dot * 2.0)),
            theme::ACCENT_SOFT,
        );
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn draw_strip(painter: &egui::Painter, rect: egui::Rect, viz: &StereoViz) {
    painter.rect_filled(rect, theme::RADIUS_CHIP, theme::BG_2);
    painter.rect_stroke(
        rect,
        theme::RADIUS_CHIP,
        egui::Stroke::new(1.0, theme::LINE),
        egui::StrokeKind::Inside,
    );
    let plot = rect.shrink2(egui::vec2(6.0, 5.0));
    let y_of = |c: f32| plot.bottom() - strip_fraction(c) * plot.height();
    painter.line_segment(
        [egui::pos2(plot.left(), y_of(0.0)), egui::pos2(plot.right(), y_of(0.0))],
        egui::Stroke::new(0.6, theme::LINE),
    );

    let n = crate::viz::CORRELATION_LEN;
    let step = plot.width() / (n - 1) as f32;
    let mut prev: Option<egui::Pos2> = None;
    for (i, v) in viz.history.iter_chrono().enumerate() {
        let p = egui::pos2(plot.left() + i as f32 * step, y_of(v));
        if let Some(q) = prev {
            painter.line_segment([q, p], egui::Stroke::new(1.2, correlation_color(v)));
        }
        prev = Some(p);
    }

    let now = viz.correlation();
    painter.text(
        egui::pos2(plot.right(), plot.top()),
        egui::Align2::RIGHT_TOP,
        format!("r {now:+.2}"),
        egui::FontId::proportional(10.0),
        correlation_color(now),
    );
    painter.text(
        egui::pos2(plot.left(), plot.top()),
        egui::Align2::LEFT_TOP,
        "CORRELATION",
        egui::FontId::proportional(9.0),
        theme::TEXT_3,
    );
}
