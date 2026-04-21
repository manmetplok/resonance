//! Static nonlinear transfer-curve plot. Visualises the currently-loaded
//! NAM model's response to a DC input sweep from -1 to +1, computed on
//! the loader thread and published via `AmpViz`.
//!
//! The curve makes model-switching visually meaningful: a clean amp
//! shows a near-linear diagonal, a crunch model bends, and a high-gain
//! profile flattens at the rails.

use wayland_plugin_gui::egui;

use crate::viz::{AmpViz, CURVE_POINTS};

use super::theme;

pub fn draw(painter: &egui::Painter, rect: egui::Rect, viz: &AmpViz) {
    painter.rect_filled(rect, 3.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        3.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    let inner = rect.shrink(12.0);
    if inner.width() <= 20.0 || inner.height() <= 20.0 {
        return;
    }

    painter.text(
        egui::pos2(inner.left(), rect.top() + 6.0),
        egui::Align2::LEFT_TOP,
        "TRANSFER CURVE",
        egui::FontId::proportional(11.0),
        theme::TEXT_DIM,
    );

    // Square plot centred in the panel.
    let side = inner.width().min(inner.height());
    let origin = egui::pos2(inner.center().x - side * 0.5, inner.center().y - side * 0.5);
    let plot = egui::Rect::from_min_size(origin, egui::vec2(side, side));

    draw_axes(painter, plot);

    let Some(curve) = viz.snapshot_transfer_curve() else {
        painter.text(
            plot.center(),
            egui::Align2::CENTER_CENTER,
            "(no model loaded)",
            egui::FontId::proportional(11.0),
            theme::TEXT_DIM,
        );
        return;
    };

    draw_curve(painter, plot, &curve);
}

fn draw_axes(painter: &egui::Painter, rect: egui::Rect) {
    // Centre cross.
    let cx = rect.center().x;
    let cy = rect.center().y;
    painter.line_segment(
        [egui::pos2(rect.left(), cy), egui::pos2(rect.right(), cy)],
        egui::Stroke::new(0.6, theme::BORDER),
    );
    painter.line_segment(
        [egui::pos2(cx, rect.top()), egui::pos2(cx, rect.bottom())],
        egui::Stroke::new(0.6, theme::BORDER),
    );

    // Diagonal reference (y = x) so the curve's deviation is obvious.
    painter.line_segment(
        [
            egui::pos2(rect.left(), rect.bottom()),
            egui::pos2(rect.right(), rect.top()),
        ],
        egui::Stroke::new(0.5, theme::TEXT_DIM),
    );

    // Unit-axis labels.
    painter.text(
        egui::pos2(rect.right() - 2.0, cy - 2.0),
        egui::Align2::RIGHT_BOTTOM,
        "+1",
        egui::FontId::monospace(9.0),
        theme::TEXT_DIM,
    );
    painter.text(
        egui::pos2(rect.left() + 2.0, cy - 2.0),
        egui::Align2::LEFT_BOTTOM,
        "−1",
        egui::FontId::monospace(9.0),
        theme::TEXT_DIM,
    );
    painter.text(
        egui::pos2(cx + 2.0, rect.top() + 2.0),
        egui::Align2::LEFT_TOP,
        "out",
        egui::FontId::monospace(9.0),
        theme::TEXT_DIM,
    );
    painter.text(
        egui::pos2(cx + 2.0, rect.bottom() - 2.0),
        egui::Align2::LEFT_BOTTOM,
        "in",
        egui::FontId::monospace(9.0),
        theme::TEXT_DIM,
    );
}

fn draw_curve(painter: &egui::Painter, rect: egui::Rect, curve: &[f32; CURVE_POINTS]) {
    // Auto-scale the output so curves for hot profiles (>1.0) and clean
    // profiles (≈ linear) both fill the panel nicely.
    let peak = curve.iter().fold(0.0f32, |m, &v| m.max(v.abs())).max(0.2);
    let scale = 0.95 / peak;

    let mut pts = Vec::with_capacity(CURVE_POINTS);
    for (i, &y) in curve.iter().enumerate() {
        let tx = i as f32 / (CURVE_POINTS - 1) as f32; // 0..1
        let nx = tx * 2.0 - 1.0; // -1..+1
        let ny = (y * scale).clamp(-1.0, 1.0);
        let px = rect.left() + (nx * 0.5 + 0.5) * rect.width();
        let py = rect.bottom() - (ny * 0.5 + 0.5) * rect.height();
        pts.push(egui::pos2(px, py));
    }

    // Filled area under the curve, against the horizontal axis.
    let cy = rect.center().y;
    let mut mesh = egui::epaint::Mesh::default();
    for pair in pts.windows(2) {
        let p0 = pair[0];
        let p1 = pair[1];
        let b = mesh.vertices.len() as u32;
        mesh.colored_vertex(egui::pos2(p0.x, cy), theme::CURVE_FILL);
        mesh.colored_vertex(p0, theme::CURVE_FILL);
        mesh.colored_vertex(p1, theme::CURVE_FILL);
        mesh.colored_vertex(egui::pos2(p1.x, cy), theme::CURVE_FILL);
        mesh.add_triangle(b, b + 1, b + 2);
        mesh.add_triangle(b, b + 2, b + 3);
    }
    painter.add(egui::Shape::mesh(mesh));

    painter.add(egui::Shape::line(
        pts,
        egui::Stroke::new(1.6, theme::CURVE_LINE),
    ));
}
