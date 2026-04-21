//! One-cycle LFO shape preview with live phase marker.

use wayland_plugin_gui::egui;

use crate::editor::theme;

pub fn draw(ui: &mut egui::Ui, rect: egui::Rect, shape: i32, depth: f32, live_phase: f32) {
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, 4.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    let mid_y = rect.center().y;
    let pad = 8.0f32;
    let left = rect.left() + pad;
    let right = rect.right() - pad;
    let width = right - left;
    let amp = (rect.height() - pad * 2.0) * 0.45 * depth.clamp(0.0, 1.0);

    // Zero line.
    painter.line_segment(
        [egui::pos2(left, mid_y), egui::pos2(right, mid_y)],
        egui::Stroke::new(0.5, theme::BORDER),
    );

    const N: usize = 192;
    let mut points: Vec<egui::Pos2> = Vec::with_capacity(N);
    for i in 0..N {
        let phase = i as f32 / (N - 1) as f32;
        let v = lfo_sample(shape, phase);
        let x = left + phase * width;
        let y = mid_y - v * amp;
        points.push(egui::pos2(x, y));
    }

    // Glow + sharp.
    painter.add(egui::Shape::line(
        points.clone(),
        egui::Stroke::new(4.0, theme::ACCENT_GLOW),
    ));
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(1.5, theme::ACCENT),
    ));

    // Live phase marker.
    let phase = live_phase.clamp(0.0, 1.0);
    let marker_x = left + phase * width;
    painter.line_segment(
        [
            egui::pos2(marker_x, rect.top() + 4.0),
            egui::pos2(marker_x, rect.bottom() - 4.0),
        ],
        egui::Stroke::new(1.0, theme::WARN),
    );
    let v = lfo_sample(shape, phase);
    let marker_y = mid_y - v * amp;
    painter.circle_filled(egui::pos2(marker_x, marker_y), 3.5, theme::WARN);
}

/// Reference implementation of the LFO shape for display. Does NOT mirror
/// the audio-thread randomness for S&H — we seed with a deterministic PRNG
/// so the visualisation is stable.
fn lfo_sample(shape: i32, phase: f32) -> f32 {
    let phase = phase.clamp(0.0, 1.0);
    match shape {
        0 => (phase * std::f32::consts::TAU).sin(),
        1 => {
            if phase < 0.25 {
                phase * 4.0
            } else if phase < 0.75 {
                2.0 - phase * 4.0
            } else {
                phase * 4.0 - 4.0
            }
        }
        2 => 2.0 * phase - 1.0,
        3 => {
            if phase < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        4 => {
            // Sample & hold: 8 deterministic steps.
            let step = (phase * 8.0).floor() as i32;
            deterministic_sh(step)
        }
        _ => 0.0,
    }
}

fn deterministic_sh(i: i32) -> f32 {
    // Fixed lookup so the preview is stable across redraws.
    const VALS: [f32; 8] = [0.6, -0.3, 0.9, -0.8, 0.2, -0.5, 0.7, -0.1];
    VALS[i.rem_euclid(8) as usize]
}
