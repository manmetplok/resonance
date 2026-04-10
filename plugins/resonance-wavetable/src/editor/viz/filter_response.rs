//! SVF magnitude response curve with live modulated cutoff marker.

use wayland_plugin_gui::egui;

use crate::editor::theme;

pub fn draw(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    filter_type: i32,
    cutoff: f32,
    resonance: f32,
    drive: f32,
    live_cutoff: f32,
) {
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, 4.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    let pad = 8.0f32;
    let left = rect.left() + pad;
    let right = rect.right() - pad;
    let top = rect.top() + pad;
    let bottom = rect.bottom() - pad;
    let width = right - left;
    let height = bottom - top;

    // Grid lines at 100 Hz, 1 kHz, 10 kHz.
    for freq in [100.0, 1_000.0, 10_000.0] {
        let x = left + freq_to_x(freq, width);
        painter.line_segment(
            [egui::pos2(x, top), egui::pos2(x, bottom)],
            egui::Stroke::new(0.4, theme::BORDER),
        );
        painter.text(
            egui::pos2(x, bottom - 2.0),
            egui::Align2::CENTER_BOTTOM,
            label_freq(freq),
            egui::FontId::proportional(9.0),
            theme::TEXT_DIM,
        );
    }
    // 0 dB line.
    let y0 = top + db_to_y(0.0, height);
    painter.line_segment(
        [egui::pos2(left, y0), egui::pos2(right, y0)],
        egui::Stroke::new(0.5, theme::BORDER),
    );

    // Response curve.
    const N: usize = 128;
    let q = 0.5 + resonance.clamp(0.0, 1.0) * 19.5; // ~0.5..20
    let mut points: Vec<egui::Pos2> = Vec::with_capacity(N);
    for i in 0..N {
        let t = i as f32 / (N - 1) as f32;
        let freq = 20.0 * (1000.0_f32).powf(t * 1.0); // 20..20k, log
        let mag_db = svf_magnitude_db(filter_type, freq, cutoff, q) + drive * 6.0;
        let x = left + freq_to_x(freq, width);
        let y = top + db_to_y(mag_db, height);
        points.push(egui::pos2(x, y));
    }
    painter.add(egui::Shape::line(
        points.clone(),
        egui::Stroke::new(4.0, theme::ACCENT_GLOW),
    ));
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(1.5, theme::ACCENT),
    ));

    // Live modulated cutoff marker.
    let live_x = left + freq_to_x(live_cutoff.clamp(20.0, 20_000.0), width);
    painter.line_segment(
        [egui::pos2(live_x, top), egui::pos2(live_x, bottom)],
        egui::Stroke::new(1.0, theme::WARN),
    );
    painter.circle_filled(
        egui::pos2(live_x, top + height * 0.5),
        3.5,
        theme::WARN,
    );
}

fn freq_to_x(freq: f32, width: f32) -> f32 {
    // Log scale over 20..20k.
    let t = (freq / 20.0).log10() / 3.0;
    t.clamp(0.0, 1.0) * width
}

fn db_to_y(db: f32, height: f32) -> f32 {
    // Map -24..+12 dB to [1..0] (inverted for screen y).
    let t = 1.0 - (db + 24.0) / 36.0;
    t.clamp(0.0, 1.0) * height
}

fn label_freq(freq: f32) -> String {
    if freq >= 1000.0 {
        format!("{}k", (freq / 1000.0) as i32)
    } else {
        format!("{}", freq as i32)
    }
}

/// Approximate SVF magnitude response in dB.
///
/// Uses a standard 2nd-order filter transfer function for LP/HP/BP/Notch.
/// Formula derived from RBJ cookbook / bilinear-transformed biquad.
fn svf_magnitude_db(filter_type: i32, freq: f32, cutoff: f32, q: f32) -> f32 {
    let w_ratio = (freq / cutoff).max(1e-6);
    let w2 = w_ratio * w_ratio;
    // SVF: H_lp = 1 / (1 - w^2 + j*w/Q), at normalized frequency w = f/fc.
    // |H_lp|^2 = 1 / ((1 - w^2)^2 + (w/Q)^2)
    let denom_sq = (1.0 - w2).powi(2) + (w_ratio / q).powi(2);
    let mag_lp_sq = 1.0 / denom_sq;
    let mag_hp_sq = w2 * w2 / denom_sq;
    let mag_bp_sq = (w_ratio / q).powi(2) / denom_sq;
    // Notch = 1 - BP (in magnitude, approximately).
    let mag_notch_sq = ((1.0 - w2).powi(2)) / denom_sq;
    let mag_sq = match filter_type {
        0 => mag_lp_sq,
        1 => mag_hp_sq,
        2 => mag_bp_sq,
        3 => mag_notch_sq,
        _ => mag_lp_sq,
    };
    10.0 * mag_sq.max(1e-10).log10()
}
