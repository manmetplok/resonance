//! Transfer curve renderer: input dB → output dB with threshold, ratio
//! and knee visualized as a smooth line plus a subtle reference grid.
//!
//! The plot's dB window is not a fact of this file: it is derived from
//! the threshold parameter's declared range (ba todo #1346). The plot
//! exists to show where the threshold sits, so the one thing it must
//! guarantee is that every threshold the user can dial in is on the
//! plot — which a second, hand-maintained copy of those bounds cannot
//! promise. It used to carry `DB_MIN`/`DB_MAX` constants that happened
//! to equal the declared range; moving the range would have slid the
//! threshold indicator silently off the plot instead of failing.

use resonance_plugin::FloatParam;
use wayland_plugin_gui::egui;

use crate::dsp::transfer_curve_db;
use crate::editor::theme;

const NUM_POINTS: usize = 128;

/// Reference-grid divisions along each axis. Six evenly spaced lines
/// (five gaps) across whatever window the threshold declares — for the
/// range declared today that is the familiar 12 dB grid, and a different
/// declared range simply redistributes them.
const GRID_DIVISIONS: usize = 5;

/// How far the plotted window reaches past the threshold parameter's
/// declared range, in dB at each end.
///
/// Zero — the window is exactly the span the threshold can take. Should
/// the plot ever want air around the extremes, this is the knob to turn:
/// the window stays expressed relative to the declaration either way,
/// which is the point.
const AXIS_MARGIN_DB: f32 = 0.0;

/// The plot's dB window, derived from the threshold parameter's declared
/// range rather than restating it. Both axes share it: the x axis is
/// input level, the y axis output level, and the threshold indicator is
/// drawn against the same mapping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DbAxis {
    min: f32,
    max: f32,
}

impl DbAxis {
    /// The window for a compressor whose threshold is `threshold`.
    /// `params.rs` owns the numbers; this only widens them by the
    /// margin above.
    pub fn from_threshold(threshold: &FloatParam) -> Self {
        let range = threshold.range();
        Self {
            min: range.min() - AXIS_MARGIN_DB,
            max: range.max() + AXIS_MARGIN_DB,
        }
    }

    pub fn min(&self) -> f32 {
        self.min
    }

    pub fn max(&self) -> f32 {
        self.max
    }

    /// Where `db` sits in the window: 0 at the bottom, 1 at the top.
    /// Levels outside the window clamp to the edge — output level can
    /// exceed it once makeup gain is applied, and a bar pinned to the
    /// frame is the honest reading there.
    pub fn fraction(&self, db: f32) -> f32 {
        ((db - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
    }

    /// The `GRID_DIVISIONS + 1` reference levels spanning the window,
    /// lowest first.
    pub fn grid_levels(&self) -> impl Iterator<Item = f32> + '_ {
        let step = (self.max - self.min) / GRID_DIVISIONS as f32;
        (0..=GRID_DIVISIONS).map(move |i| self.min + i as f32 * step)
    }

    fn x(&self, db: f32, width: f32) -> f32 {
        self.fraction(db) * width
    }

    fn y(&self, db: f32, height: f32) -> f32 {
        (1.0 - self.fraction(db)) * height
    }
}

/// Parameters the transfer curve needs to render itself and the live
/// operating-point marker.
#[derive(Clone, Copy)]
pub struct CurveParams {
    /// The plot window, built from the threshold parameter by the caller
    /// that owns it — see [`DbAxis::from_threshold`].
    pub axis: DbAxis,
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
        axis,
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

    draw_grid(painter, plot, axis);
    draw_unity_line(painter, plot);

    // Threshold indicator — a vertical line at the threshold input
    // level. The axis spans the threshold's declared range, so this line
    // is on the plot for every value the user can reach.
    let thr_x = plot.left() + axis.x(threshold, plot.width());
    painter.line_segment(
        [
            egui::pos2(thr_x, plot.top()),
            egui::pos2(thr_x, plot.bottom()),
        ],
        egui::Stroke::new(1.0, theme::TEXT_DIM),
    );

    // Transfer curve itself.
    let mut points: Vec<egui::Pos2> = Vec::with_capacity(NUM_POINTS);
    for i in 0..NUM_POINTS {
        let t = i as f32 / (NUM_POINTS - 1) as f32;
        let in_db = axis.min() + t * (axis.max() - axis.min());
        let out_db = transfer_curve_db(in_db, threshold, ratio, knee, makeup);
        let x = plot.left() + axis.x(in_db, plot.width());
        let y = plot.top() + axis.y(out_db, plot.height());
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
    if current_input_db.is_finite() && current_input_db > axis.min() {
        let op_in_db = current_input_db.clamp(axis.min(), axis.max());
        let op_out_db = op_in_db - current_gr_db + makeup;
        let op_x = plot.left() + axis.x(op_in_db, plot.width());
        let op_y = plot.top() + axis.y(op_out_db, plot.height());
        painter.circle_filled(egui::pos2(op_x, op_y), 4.0, theme::GR);
        painter.circle_stroke(
            egui::pos2(op_x, op_y),
            6.0,
            egui::Stroke::new(1.0, theme::GR_GLOW),
        );
    }
}

fn draw_grid(painter: &egui::Painter, plot: egui::Rect, axis: DbAxis) {
    for (i, db) in axis.grid_levels().enumerate() {
        let x = plot.left() + axis.x(db, plot.width());
        painter.line_segment(
            [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
            egui::Stroke::new(0.4, theme::BORDER),
        );
        let y = plot.top() + axis.y(db, plot.height());
        painter.line_segment(
            [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
            egui::Stroke::new(0.4, theme::BORDER),
        );
        // The topmost line sits on the frame; its label would hang off
        // the plot, so it goes unlabelled as it always has.
        if i < GRID_DIVISIONS {
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
