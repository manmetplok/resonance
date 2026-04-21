//! IR waveform envelope plot. Snapshots the precomputed waveform from
//! `IrViz` once per frame and draws it as a mirrored envelope around
//! the centre line — the left channel on top, the right channel
//! below. For mono IRs the two halves mirror each other exactly.

use wayland_plugin_gui::egui;

use crate::viz::IrViz;

use super::theme;

pub fn draw(painter: &egui::Painter, rect: egui::Rect, viz: &IrViz, ir_name: &str) {
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
        "IMPULSE RESPONSE",
        egui::FontId::proportional(11.0),
        theme::TEXT_DIM,
    );

    draw_grid(painter, inner);

    let Some(snap) = viz.snapshot() else {
        painter.text(
            inner.center(),
            egui::Align2::CENTER_CENTER,
            "(no IR loaded)",
            egui::FontId::proportional(12.0),
            theme::TEXT_DIM,
        );
        return;
    };

    if snap.wave_len == 0 {
        return;
    }

    // Auto-gain so the tallest point in either channel sits at ~90%
    // of the half-panel height. Guarantees visible results for IRs
    // with small absolute magnitude.
    let peak = snap.wave_left[..snap.wave_len]
        .iter()
        .chain(snap.wave_right[..snap.wave_len].iter())
        .fold(0.0f32, |m, &v| m.max(v.abs()))
        .max(0.05);
    let scale = 0.9 / peak;

    draw_channel(
        painter,
        inner,
        &snap.wave_left[..snap.wave_len],
        scale,
        true,
        theme::WAVE_L,
    );
    draw_channel(
        painter,
        inner,
        &snap.wave_right[..snap.wave_len],
        scale,
        false,
        theme::WAVE_R,
    );

    if !ir_name.is_empty() {
        painter.text(
            egui::pos2(inner.right() - 4.0, rect.top() + 6.0),
            egui::Align2::RIGHT_TOP,
            ir_name,
            egui::FontId::proportional(11.0),
            theme::TEXT_DIM,
        );
    }
}

fn draw_grid(painter: &egui::Painter, rect: egui::Rect) {
    let cy = rect.center().y;
    painter.line_segment(
        [egui::pos2(rect.left(), cy), egui::pos2(rect.right(), cy)],
        egui::Stroke::new(0.8, theme::BORDER),
    );
    for frac in &[0.25f32, 0.75] {
        let y = rect.top() + rect.height() * frac;
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(0.3, theme::BORDER),
        );
    }
    for i in 1..4 {
        let x = rect.left() + rect.width() * (i as f32 / 4.0);
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(0.3, theme::BORDER),
        );
    }
}

/// Draw one channel's envelope as a filled mirrored polygon. `upper`
/// selects whether the envelope lives above or below the centre line.
fn draw_channel(
    painter: &egui::Painter,
    rect: egui::Rect,
    samples: &[f32],
    scale: f32,
    upper: bool,
    color: egui::Color32,
) {
    let n = samples.len();
    if n == 0 {
        return;
    }
    let cy = rect.center().y;
    let half_h = rect.height() * 0.5;
    let sign: f32 = if upper { -1.0 } else { 1.0 };

    let width = rect.width();
    let mut pts = Vec::with_capacity(n + 2);
    pts.push(egui::pos2(rect.left(), cy));
    for (i, &s) in samples.iter().enumerate() {
        let t = if n == 1 {
            0.0
        } else {
            i as f32 / (n - 1) as f32
        };
        let x = rect.left() + t * width;
        let mag = (s.abs() * scale).clamp(0.0, 1.0);
        let y = cy + sign * mag * half_h;
        pts.push(egui::pos2(x, y));
    }
    pts.push(egui::pos2(rect.right(), cy));

    // Fill.
    let mut mesh = egui::epaint::Mesh::default();
    for w in pts.windows(2) {
        let a = w[0];
        let b = w[1];
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(egui::pos2(a.x, cy), theme::WAVE_FILL);
        mesh.colored_vertex(a, theme::WAVE_FILL);
        mesh.colored_vertex(b, theme::WAVE_FILL);
        mesh.colored_vertex(egui::pos2(b.x, cy), theme::WAVE_FILL);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    painter.add(egui::Shape::mesh(mesh));

    // Envelope stroke (skip the bookend baseline points so the line
    // doesn't zig-zag from the centre at either edge).
    let stroke_pts = pts[1..pts.len() - 1].to_vec();
    painter.add(egui::Shape::line(stroke_pts, egui::Stroke::new(1.2, color)));
}
