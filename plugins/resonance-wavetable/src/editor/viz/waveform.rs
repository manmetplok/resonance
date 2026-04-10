//! Wavetable waveform viewer.
//!
//! Draws the selected frame as a polyline at full alpha, with the two
//! neighbour frames drawn at low alpha when `position` sits between frames
//! so the morph is visible. A vertical marker shows the post-modulation
//! live osc position coming from the audio thread.

use wayland_plugin_gui::egui;

use crate::editor::display_waves;
use crate::editor::theme;

/// Draw the waveform viewer for the given oscillator into a reserved rect.
///
/// `wavetable_idx` is the current wavetable selection for this osc.
/// `position` is the raw param value (0..1) for frame morph position.
/// `live_position` is the post-mod osc position from the viz snapshot, used
/// to draw the live marker; pass the same value as `position` if you don't
/// want a marker.
pub fn draw(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    wavetable_idx: usize,
    position: f32,
    live_position: f32,
) {
    let painter = ui.painter_at(rect);

    // Background + border.
    painter.rect_filled(rect, 4.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    // Horizontal zero-axis line.
    let mid_y = rect.center().y;
    painter.line_segment(
        [
            egui::pos2(rect.left() + 6.0, mid_y),
            egui::pos2(rect.right() - 6.0, mid_y),
        ],
        egui::Stroke::new(0.5, theme::BORDER),
    );

    let frames = display_waves::frame_count(wavetable_idx);
    if frames == 0 {
        return;
    }

    // Pick blend neighbours.
    let f_max = (frames - 1) as f32;
    let f_float = position.clamp(0.0, 1.0) * f_max;
    let f_lo = f_float.floor() as usize;
    let f_hi = (f_lo + 1).min(frames - 1);
    let t = f_float - f_lo as f32;

    const N_POINTS: usize = 256;
    let samples_lo = display_waves::display_samples(wavetable_idx, f_lo, N_POINTS);
    let samples_hi = display_waves::display_samples(wavetable_idx, f_hi, N_POINTS);

    // Draw low-alpha neighbour (lo) and next neighbour (hi) when blending.
    if t > 0.0001 {
        draw_wave(
            &painter,
            rect,
            &samples_lo,
            egui::Stroke::new(1.0, theme::ACCENT.linear_multiply(0.3)),
        );
        draw_wave(
            &painter,
            rect,
            &samples_hi,
            egui::Stroke::new(1.0, theme::ACCENT.linear_multiply(0.3)),
        );
    }

    // Blended main waveform.
    let mut blended = vec![0.0f32; N_POINTS];
    for i in 0..N_POINTS {
        blended[i] = samples_lo[i] * (1.0 - t) + samples_hi[i] * t;
    }

    // Glow: wide low-alpha stroke first, then sharp full-alpha on top.
    draw_wave(
        &painter,
        rect,
        &blended,
        egui::Stroke::new(4.0, theme::ACCENT_GLOW),
    );
    draw_wave(
        &painter,
        rect,
        &blended,
        egui::Stroke::new(1.5, theme::ACCENT),
    );

    // Live position marker — post-mod.
    let live = live_position.clamp(0.0, 1.0);
    let marker_x = rect.left() + live * rect.width();
    painter.line_segment(
        [
            egui::pos2(marker_x, rect.top() + 4.0),
            egui::pos2(marker_x, rect.bottom() - 4.0),
        ],
        egui::Stroke::new(1.0, theme::WARN),
    );
}

fn draw_wave(painter: &egui::Painter, rect: egui::Rect, samples: &[f32], stroke: egui::Stroke) {
    let n = samples.len();
    if n < 2 {
        return;
    }
    let mut points: Vec<egui::Pos2> = Vec::with_capacity(n);
    let inset = 6.0f32;
    let w = rect.width() - inset * 2.0;
    let h = rect.height() - inset * 2.0;
    for (i, s) in samples.iter().enumerate() {
        let x = rect.left() + inset + (i as f32 / (n - 1) as f32) * w;
        let y = rect.center().y - s.clamp(-1.0, 1.0) * (h * 0.5);
        points.push(egui::pos2(x, y));
    }
    painter.add(egui::Shape::line(points, stroke));
}
