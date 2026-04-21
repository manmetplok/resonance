//! ADSR envelope curve with live playhead.
//!
//! Draws an A/D/sustain-hold/R outline matching the `exp_coeff` shape used
//! by `AdsrEnvelope::next()`. Fills below the curve with a translucent
//! accent. If a `live_value` + `live_stage` are provided (from the audio
//! thread), a dot is drawn at the current position.

use wayland_plugin_gui::egui;

use crate::editor::theme;

/// Envelope parameters in seconds / 0..1.
pub struct Adsr {
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    /// Curve shaping -1..+1 (currently informational; the closed-form viz
    /// approximation doesn't use it yet).
    #[allow(dead_code)]
    pub curve: f32,
}

pub fn draw(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    adsr: &Adsr,
    live_value: Option<f32>,
    live_stage: u32,
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

    // Grid line at y=0.
    let bottom = rect.bottom() - 6.0;
    let top = rect.top() + 6.0;
    let left = rect.left() + 8.0;
    let right = rect.right() - 8.0;
    painter.line_segment(
        [egui::pos2(left, bottom), egui::pos2(right, bottom)],
        egui::Stroke::new(0.5, theme::BORDER),
    );

    // Total duration used for x-scaling: A + D + "sustain region" + R.
    // The sustain region is a fixed visual fraction so sustain is visible
    // even when A/D/R are very short.
    let sustain_display_s = 0.4f32;
    let total = (adsr.attack + adsr.decay + sustain_display_s + adsr.release).max(0.01);
    let width = right - left;
    let height = bottom - top;

    // Helpers to convert (t_seconds, y_0to1) -> screen pos.
    let p = |t: f32, y: f32| -> egui::Pos2 {
        let x = left + (t / total) * width;
        let yy = bottom - y.clamp(0.0, 1.0) * height;
        egui::pos2(x, yy)
    };

    const N: usize = 32;
    let mut points: Vec<egui::Pos2> = Vec::with_capacity(N * 3 + 4);

    points.push(p(0.0, 0.0));

    // --- Attack segment (0..attack, rises 0..1).
    // Approximate with a closed-form exponential — close enough visually
    // to the audio-thread's iterative envelope.
    for i in 1..=N {
        let t = i as f32 / N as f32;
        let level = (1.3 * (1.0 - (-t * (1.3_f32.ln() + 1.0)).exp())).min(1.0);
        points.push(p(adsr.attack * t, level));
        if level >= 1.0 {
            break;
        }
    }
    // --- Decay segment (attack..attack+decay, falls 1 -> sustain)
    let t_decay_start = adsr.attack;
    for i in 1..=N {
        let t = i as f32 / N as f32;
        let y = 1.0 + (adsr.sustain - 1.0) * (1.0 - (-t * 3.0).exp());
        points.push(p(t_decay_start + adsr.decay * t, y));
    }
    // --- Sustain hold
    let t_sustain_start = adsr.attack + adsr.decay;
    points.push(p(t_sustain_start + sustain_display_s, adsr.sustain));
    // --- Release segment
    let t_release_start = t_sustain_start + sustain_display_s;
    for i in 1..=N {
        let t = i as f32 / N as f32;
        let y = adsr.sustain * (1.0 - (-t * 3.0).exp()).mul_add(-1.0, 1.0);
        points.push(p(t_release_start + adsr.release * t, y));
    }
    points.push(p(total, 0.0));

    // Fill under curve.
    let mut fill_points = points.clone();
    fill_points.push(p(total, 0.0));
    fill_points.push(p(0.0, 0.0));
    painter.add(egui::Shape::convex_polygon(
        fill_points,
        theme::ACCENT_GLOW,
        egui::Stroke::NONE,
    ));

    // Stroke the outline (drop the closing bottom point).
    points.pop();
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(1.5, theme::ACCENT),
    ));

    // Phase labels.
    let phase_labels = [
        ("A", adsr.attack / total * 0.5),
        ("D", (adsr.attack + adsr.decay / 2.0) / total),
        ("S", (t_sustain_start + sustain_display_s / 2.0) / total),
        ("R", (t_release_start + adsr.release / 2.0) / total),
    ];
    for (label, rel_x) in phase_labels.iter() {
        let x = left + rel_x * width;
        painter.text(
            egui::pos2(x, rect.bottom() - 2.0),
            egui::Align2::CENTER_BOTTOM,
            *label,
            egui::FontId::proportional(9.0),
            theme::TEXT_DIM,
        );
    }

    // Live playhead.
    if let Some(level) = live_value {
        // Pick an x based on the stage reported by the audio thread.
        let px = match live_stage {
            1 => {
                // Attack — x is proportional to level since attack ramps 0..1.
                left + (adsr.attack / total) * width * level.min(1.0)
            }
            2 => {
                // Decay — x is proportional to how far below 1 we are.
                let progress = ((1.0 - level) / (1.0 - adsr.sustain).max(0.01)).clamp(0.0, 1.0);
                left + ((adsr.attack + adsr.decay * progress) / total) * width
            }
            3 => {
                // Sustain — hold at the middle of the sustain region.
                left + ((t_sustain_start + sustain_display_s * 0.5) / total) * width
            }
            4 => {
                // Release — progress from sustain level down to 0.
                let progress = 1.0 - (level / adsr.sustain.max(0.01)).clamp(0.0, 1.0);
                left + ((t_release_start + adsr.release * progress) / total) * width
            }
            _ => return,
        };
        let py = bottom - level.clamp(0.0, 1.0) * height;
        painter.circle_filled(egui::pos2(px, py), 4.0, theme::WARN);
        painter.circle_stroke(
            egui::pos2(px, py),
            5.0,
            egui::Stroke::new(1.0, theme::WARN.linear_multiply(0.4)),
        );
    }
}
