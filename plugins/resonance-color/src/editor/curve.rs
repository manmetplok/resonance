//! Transfer-curve plot: the mode's static curve (drive law,
//! normalisation, bias and mix — [`crate::dsp::voicing::transfer`]) over
//! input −1…+1, with the live input peak marked on it.

use plugin_gui_core::egui;

use crate::dsp::voicing::transfer;
use crate::dsp::Settings;
use crate::editor::theme;
use crate::params::Mode;

/// Points across the input range.
pub const NUM_POINTS: usize = 129;

/// The plotted input (and output) span, ±this.
pub const SPAN: f32 = 1.0;

/// The settings the curve depends on; anything else (filters, output
/// trim, oversampling) does not move it.
#[derive(Clone, Copy, PartialEq)]
struct Key {
    mode: Mode,
    drive: f32,
    bias: f32,
    mix: f32,
}

/// The curve's points, recomputed only when a setting it depends on
/// changes.
#[derive(Default)]
pub struct CurveCache {
    key: Option<Key>,
    points: Vec<(f32, f32)>,
}

impl CurveCache {
    /// `(input, output)` pairs for `s`, input ascending across ±[`SPAN`].
    pub fn points(&mut self, s: &Settings) -> &[(f32, f32)] {
        let key = Key {
            mode: s.mode,
            drive: s.drive,
            bias: s.bias,
            mix: s.mix,
        };
        if self.key != Some(key) {
            self.points = curve_points(s);
            self.key = Some(key);
        }
        &self.points
    }
}

/// The transfer curve for `s` as `(input, output)` pairs.
pub fn curve_points(s: &Settings) -> Vec<(f32, f32)> {
    (0..NUM_POINTS)
        .map(|i| {
            let x = -SPAN + 2.0 * SPAN * i as f32 / (NUM_POINTS - 1) as f32;
            (x, transfer(s.mode, s.drive, s.bias, s.mix, x))
        })
        .collect()
}

pub fn draw(painter: &egui::Painter, rect: egui::Rect, points: &[(f32, f32)], input_db: f32) {
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
    let to_screen = |x: f32, y: f32| {
        let fx = (x + SPAN) / (2.0 * SPAN);
        let fy = ((y + SPAN) / (2.0 * SPAN)).clamp(0.0, 1.0);
        egui::pos2(
            plot.left() + fx * plot.width(),
            plot.bottom() - fy * plot.height(),
        )
    };

    // Axes and the unity line.
    let axis = egui::Stroke::new(0.5, theme::BORDER);
    painter.line_segment([to_screen(-SPAN, 0.0), to_screen(SPAN, 0.0)], axis);
    painter.line_segment([to_screen(0.0, -SPAN), to_screen(0.0, SPAN)], axis);
    painter.line_segment(
        [to_screen(-SPAN, -SPAN), to_screen(SPAN, SPAN)],
        egui::Stroke::new(0.8, theme::TEXT_DIM),
    );

    let line: Vec<egui::Pos2> = points.iter().map(|&(x, y)| to_screen(x, y)).collect();
    painter.add(egui::Shape::line(
        line.clone(),
        egui::Stroke::new(4.0, theme::ACCENT_GLOW),
    ));
    painter.add(egui::Shape::line(line, egui::Stroke::new(1.8, theme::ACCENT)));

    // Live operating point: the input peak, on the positive half.
    if input_db.is_finite() && input_db > -60.0 && !points.is_empty() {
        let x = 10f32.powf(input_db / 20.0).min(SPAN);
        let t = (x + SPAN) / (2.0 * SPAN) * (points.len() - 1) as f32;
        let i = (t as usize).min(points.len() - 2);
        let f = t - i as f32;
        let y = points[i].1 + f * (points[i + 1].1 - points[i].1);
        let p = to_screen(x, y);
        painter.circle_filled(p, 4.0, theme::WARM);
    }

    painter.text(
        egui::pos2(plot.left() + 2.0, plot.top() + 2.0),
        egui::Align2::LEFT_TOP,
        "TRANSFER",
        egui::FontId::proportional(9.0),
        theme::TEXT_DIM,
    );
}
