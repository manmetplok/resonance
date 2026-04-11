//! Log-frequency magnitude-response plot. Snapshots the precomputed
//! response curve from `IrViz` and draws it like a spectrum-analyser
//! trace: log-frequency X axis, dB Y axis, reference lines at 0 and
//! every 12 dB.

use wayland_plugin_gui::egui;

use crate::viz::{IrViz, RESPONSE_POINTS};

use super::theme;

/// Top of the plot's dB range.
const DB_MAX: f32 = 6.0;
/// Bottom of the plot's dB range (matches the loader's clamp floor).
const DB_MIN: f32 = -48.0;

pub fn draw(painter: &egui::Painter, rect: egui::Rect, viz: &IrViz) {
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
        "FREQUENCY RESPONSE",
        egui::FontId::proportional(11.0),
        theme::TEXT_DIM,
    );

    let Some(snap) = viz.snapshot() else {
        draw_grid(painter, inner, 20.0, 20_000.0);
        painter.text(
            inner.center(),
            egui::Align2::CENTER_CENTER,
            "(no IR loaded)",
            egui::FontId::proportional(12.0),
            theme::TEXT_DIM,
        );
        return;
    };

    draw_grid(painter, inner, snap.response_min_hz, snap.response_max_hz);
    draw_curve(painter, inner, &snap.response_db);
}

fn draw_grid(painter: &egui::Painter, rect: egui::Rect, f_min: f32, f_max: f32) {
    // Horizontal dB lines every 12 dB.
    let mut db = DB_MIN.ceil();
    while db <= DB_MAX {
        let y = db_to_y(rect, db);
        let is_zero = (db - 0.0).abs() < 0.5;
        let stroke = if is_zero {
            egui::Stroke::new(0.8, theme::BORDER)
        } else if (db as i32) % 12 == 0 {
            egui::Stroke::new(0.4, theme::BORDER)
        } else {
            egui::Stroke::new(0.0, theme::BORDER)
        };
        if stroke.width > 0.0 {
            painter.line_segment(
                [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                stroke,
            );
            if (db as i32) % 12 == 0 {
                painter.text(
                    egui::pos2(rect.left() + 2.0, y - 1.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("{db:+.0}"),
                    egui::FontId::monospace(9.0),
                    theme::TEXT_DIM,
                );
            }
        }
        db += 6.0;
    }

    // Vertical decade lines: 100, 1k, 10k.
    let log_min = f_min.ln();
    let log_max = f_max.ln();
    for &f in &[100.0f32, 1000.0, 10_000.0] {
        if f < f_min || f > f_max {
            continue;
        }
        let t = (f.ln() - log_min) / (log_max - log_min);
        let x = rect.left() + t * rect.width();
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(0.4, theme::BORDER),
        );
        let label = if f >= 1000.0 {
            format!("{:.0}k", f / 1000.0)
        } else {
            format!("{f:.0}")
        };
        painter.text(
            egui::pos2(x + 2.0, rect.bottom() - 2.0),
            egui::Align2::LEFT_BOTTOM,
            label,
            egui::FontId::monospace(9.0),
            theme::TEXT_DIM,
        );
    }
}

fn draw_curve(painter: &egui::Painter, rect: egui::Rect, response_db: &[f32; RESPONSE_POINTS]) {
    let mut pts = Vec::with_capacity(RESPONSE_POINTS);
    for (i, &db) in response_db.iter().enumerate() {
        let t = i as f32 / (RESPONSE_POINTS - 1) as f32;
        let x = rect.left() + t * rect.width();
        let y = db_to_y(rect, db.clamp(DB_MIN, DB_MAX));
        pts.push(egui::pos2(x, y));
    }

    // Fill under the curve, bounded at the bottom of the plot area.
    let mut mesh = egui::epaint::Mesh::default();
    for w in pts.windows(2) {
        let a = w[0];
        let b = w[1];
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(egui::pos2(a.x, rect.bottom()), theme::RESPONSE_FILL);
        mesh.colored_vertex(a, theme::RESPONSE_FILL);
        mesh.colored_vertex(b, theme::RESPONSE_FILL);
        mesh.colored_vertex(egui::pos2(b.x, rect.bottom()), theme::RESPONSE_FILL);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    painter.add(egui::Shape::mesh(mesh));

    painter.add(egui::Shape::line(
        pts,
        egui::Stroke::new(1.6, theme::RESPONSE_LINE),
    ));
}

fn db_to_y(rect: egui::Rect, db: f32) -> f32 {
    let t = (db - DB_MIN) / (DB_MAX - DB_MIN);
    rect.bottom() - t * rect.height()
}
