//! Transfer curve renderer: input dB → output dB with threshold, ratio
//! and knee visualized as a smooth line plus a subtle reference grid.

use wayland_plugin_gui::egui;

use crate::dsp::transfer_curve_db;
use crate::editor::theme;

const NUM_POINTS: usize = 128;
const DB_MIN: f32 = -60.0;
const DB_MAX: f32 = 0.0;

/// Parameters the transfer curve needs to render itself and the live
/// operating-point marker.
#[derive(Clone, Copy)]
pub struct CurveParams {
    pub threshold: f32,
    pub ratio: f32,
    pub knee: f32,
    pub makeup: f32,
    pub current_gr_db: f32,
    pub current_input_db: f32,
}

/// Draw the static input/output transfer curve for the current threshold,
/// ratio, knee, and makeup. `rect` is the plot area; the caller is
/// responsible for margins.
pub fn draw(painter: &egui::Painter, rect: egui::Rect, p: CurveParams) {
    let CurveParams {
        threshold,
        ratio,
        knee,
        makeup,
        current_gr_db,
        current_input_db,
    } = p;
    painter.rect_filled(rect, 4.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    let pad = 10.0;
    let plot = egui::Rect::from_min_max(
        egui::pos2(rect.left() + pad, rect.top() + pad),
        egui::pos2(rect.right() - pad, rect.bottom() - pad),
    );

    draw_grid(painter, plot);
    draw_unity_line(painter, plot);

    // Threshold indicator — a vertical line at the threshold input level.
    let thr_x = plot.left() + db_to_x(threshold, plot.width());
    painter.line_segment(
        [egui::pos2(thr_x, plot.top()), egui::pos2(thr_x, plot.bottom())],
        egui::Stroke::new(1.0, theme::TEXT_DIM),
    );

    // Transfer curve itself.
    let mut points: Vec<egui::Pos2> = Vec::with_capacity(NUM_POINTS);
    for i in 0..NUM_POINTS {
        let t = i as f32 / (NUM_POINTS - 1) as f32;
        let in_db = DB_MIN + t * (DB_MAX - DB_MIN);
        let out_db = transfer_curve_db(in_db, threshold, ratio, knee, makeup);
        let x = plot.left() + db_to_x(in_db, plot.width());
        let y = plot.top() + db_to_y(out_db, plot.height());
        points.push(egui::pos2(x, y));
    }

    painter.add(egui::Shape::line(
        points.clone(),
        egui::Stroke::new(4.0, theme::ACCENT_GLOW),
    ));
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(1.8, theme::ACCENT),
    ));

    // Live operating point: where the current input level sits on the
    // curve. Drawn as a small circle so the user can see the compressor
    // working in real time.
    if current_input_db.is_finite() && current_input_db > DB_MIN {
        let op_in_db = current_input_db.clamp(DB_MIN, DB_MAX);
        let op_out_db = op_in_db - current_gr_db + makeup;
        let op_x = plot.left() + db_to_x(op_in_db, plot.width());
        let op_y = plot.top() + db_to_y(op_out_db, plot.height());
        painter.circle_filled(egui::pos2(op_x, op_y), 4.0, theme::GR);
        painter.circle_stroke(
            egui::pos2(op_x, op_y),
            6.0,
            egui::Stroke::new(1.0, theme::GR_GLOW),
        );
    }
}

fn draw_grid(painter: &egui::Painter, plot: egui::Rect) {
    for db in [-60.0, -48.0, -36.0, -24.0, -12.0, 0.0] {
        let x = plot.left() + db_to_x(db, plot.width());
        painter.line_segment(
            [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
            egui::Stroke::new(0.4, theme::BORDER),
        );
        let y = plot.top() + db_to_y(db, plot.height());
        painter.line_segment(
            [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
            egui::Stroke::new(0.4, theme::BORDER),
        );
        if db != 0.0 {
            painter.text(
                egui::pos2(x + 2.0, plot.bottom() - 2.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{:.0}", db),
                egui::FontId::proportional(9.0),
                theme::TEXT_DIM,
            );
        }
    }
}

fn draw_unity_line(painter: &egui::Painter, plot: egui::Rect) {
    // Input == output → slope 1 from bottom-left to top-right of the plot.
    painter.line_segment(
        [
            egui::pos2(plot.left(), plot.bottom()),
            egui::pos2(plot.right(), plot.top()),
        ],
        egui::Stroke::new(0.8, theme::TEXT_DIM),
    );
}

fn db_to_x(db: f32, width: f32) -> f32 {
    let t = ((db - DB_MIN) / (DB_MAX - DB_MIN)).clamp(0.0, 1.0);
    t * width
}

fn db_to_y(db: f32, height: f32) -> f32 {
    let t = 1.0 - ((db - DB_MIN) / (DB_MAX - DB_MIN)).clamp(0.0, 1.0);
    t * height
}
