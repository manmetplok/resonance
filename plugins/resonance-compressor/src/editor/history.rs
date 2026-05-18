//! Rolling gain-reduction history trace.
//!
//! Reads the shared `CompressorViz::history` ring each frame and renders
//! it as a filled area chart extending down from the 0 dB line. The
//! history is plotted with the newest sample on the right, oldest on
//! the left.

use wayland_plugin_gui::egui;

use crate::editor::theme;
use crate::viz::{CompressorViz, HISTORY_LEN};

/// Maximum gain reduction (in dB) the y-axis will display. Anything more
/// than this pins to the floor of the plot. Typical compression sits in
/// the 0..12 dB range, which this window covers with some headroom.
const GR_MAX_DB: f32 = 18.0;

pub fn draw(painter: &egui::Painter, rect: egui::Rect, viz: &CompressorViz) {
    painter.rect_filled(rect, 4.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    let pad = 8.0;
    let plot = egui::Rect::from_min_max(
        egui::pos2(rect.left() + pad, rect.top() + pad),
        egui::pos2(rect.right() - pad, rect.bottom() - pad),
    );

    // Horizontal reference lines at 0, 3, 6, 12, 18 dB of GR.
    for db in [0.0, 3.0, 6.0, 12.0, 18.0] {
        let y = plot.top() + gr_db_to_y(db, plot.height());
        painter.line_segment(
            [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
            egui::Stroke::new(0.4, theme::BORDER),
        );
        if db != 0.0 {
            painter.text(
                egui::pos2(plot.left() + 2.0, y),
                egui::Align2::LEFT_CENTER,
                format!("-{:.0}", db),
                egui::FontId::proportional(9.0),
                theme::TEXT_DIM,
            );
        }
    }

    // Snapshot the lock-free ring buffer into a local array so the
    // rest of the draw walks plain stack memory.
    let samples: [f32; HISTORY_LEN] = {
        let mut out = [0.0; HISTORY_LEN];
        for (i, v) in viz.history.iter_chrono().enumerate() {
            out[i] = v;
        }
        out
    };

    // Build the top edge of the history area, left to right.
    let mut top: Vec<egui::Pos2> = Vec::with_capacity(HISTORY_LEN);
    for (i, &gr) in samples.iter().enumerate() {
        let t = i as f32 / (HISTORY_LEN - 1) as f32;
        let x = plot.left() + t * plot.width();
        let y = plot.top() + gr_db_to_y(gr.max(0.0), plot.height());
        top.push(egui::pos2(x, y));
    }

    // Filled area via an explicit quad strip (same pattern as the EQ
    // analyzer) so the tessellator can't fan-triangulate a non-convex
    // shape. Each quad spans (x_i, y_i) → (x_{i+1}, y_{i+1}) down to
    // the 0 dB baseline.
    let base_y = plot.top() + gr_db_to_y(0.0, plot.height());
    let fill = gr_fill();
    let mut mesh = egui::epaint::Mesh::default();
    for pair in top.windows(2) {
        let p0 = pair[0];
        let p1 = pair[1];
        let b = mesh.vertices.len() as u32;
        mesh.colored_vertex(egui::pos2(p0.x, base_y), fill);
        mesh.colored_vertex(p0, fill);
        mesh.colored_vertex(p1, fill);
        mesh.colored_vertex(egui::pos2(p1.x, base_y), fill);
        mesh.add_triangle(b, b + 1, b + 2);
        mesh.add_triangle(b, b + 2, b + 3);
    }
    painter.add(egui::Shape::mesh(mesh));

    // Crisp top edge.
    painter.add(egui::Shape::line(top, egui::Stroke::new(1.0, theme::GR)));
}

fn gr_db_to_y(gr_db: f32, height: f32) -> f32 {
    let t = (gr_db.clamp(0.0, GR_MAX_DB) / GR_MAX_DB).clamp(0.0, 1.0);
    t * height
}

fn gr_fill() -> egui::Color32 {
    egui::Color32::from_rgba_premultiplied(0xff, 0xb6, 0x4a, 0x3c)
}
