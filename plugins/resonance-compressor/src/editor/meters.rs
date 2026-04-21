//! Three vertical bar meters: input peak, gain reduction, output peak.
//!
//! The meters read atomic scalar values from `CompressorViz`, which are
//! updated once per audio block. Values are in dB:
//! - Input / output use a −60…0 dBFS scale, rising from the bottom.
//! - Gain reduction uses a 0…18 dB scale, hanging from the top.

use wayland_plugin_gui::egui;

use crate::editor::theme;

pub fn draw_input_meter(painter: &egui::Painter, rect: egui::Rect, db: f32) {
    draw_level_meter(
        painter,
        rect,
        db,
        LevelMode::FromBottom,
        theme::ACCENT,
        "IN",
    );
}

pub fn draw_output_meter(painter: &egui::Painter, rect: egui::Rect, db: f32) {
    draw_level_meter(
        painter,
        rect,
        db,
        LevelMode::FromBottom,
        theme::ACCENT,
        "OUT",
    );
}

pub fn draw_gr_meter(painter: &egui::Painter, rect: egui::Rect, db: f32) {
    draw_level_meter(painter, rect, db, LevelMode::HangingGr, theme::GR, "GR");
}

#[derive(Clone, Copy)]
enum LevelMode {
    /// Input/output meters: bar rises from the bottom of the rect. Value
    /// is dBFS, clamped to [-60, +6].
    FromBottom,
    /// Gain reduction meter: bar hangs from the top of the rect. Value is
    /// dB of gain reduction, clamped to [0, 24].
    HangingGr,
}

fn draw_level_meter(
    painter: &egui::Painter,
    rect: egui::Rect,
    db: f32,
    mode: LevelMode,
    fill: egui::Color32,
    label: &str,
) {
    painter.rect_filled(rect, 2.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    // Leave a bit of padding so the bar doesn't touch the rounded frame.
    let pad = 2.0f32;
    let inner = egui::Rect::from_min_max(
        egui::pos2(rect.left() + pad, rect.top() + pad + 12.0),
        egui::pos2(rect.right() - pad, rect.bottom() - pad - 12.0),
    );

    // Horizontal tick marks on the side of the meter.
    let ticks: &[f32] = match mode {
        LevelMode::FromBottom => &[-60.0, -40.0, -20.0, -12.0, -6.0, 0.0],
        LevelMode::HangingGr => &[0.0, 3.0, 6.0, 12.0, 18.0],
    };
    for &t in ticks {
        let y = value_to_y(t, mode, inner);
        painter.line_segment(
            [egui::pos2(inner.left(), y), egui::pos2(inner.right(), y)],
            egui::Stroke::new(0.3, theme::BORDER),
        );
    }

    // Filled bar.
    let value_y = value_to_y(db, mode, inner);
    let bar = match mode {
        LevelMode::FromBottom => egui::Rect::from_min_max(
            egui::pos2(inner.left(), value_y),
            egui::pos2(inner.right(), inner.bottom()),
        ),
        LevelMode::HangingGr => egui::Rect::from_min_max(
            egui::pos2(inner.left(), inner.top()),
            egui::pos2(inner.right(), value_y),
        ),
    };
    if bar.height() > 0.0 {
        painter.rect_filled(bar, 1.0, fill);
    }

    // Label at top.
    painter.text(
        egui::pos2(rect.center().x, rect.top() + 2.0),
        egui::Align2::CENTER_TOP,
        label,
        egui::FontId::proportional(9.0),
        theme::TEXT_DIM,
    );

    // Numeric readout at bottom.
    let readout = match mode {
        LevelMode::FromBottom => {
            if db.is_finite() && db > -100.0 {
                format!("{:+.1}", db)
            } else {
                "-inf".to_string()
            }
        }
        LevelMode::HangingGr => format!("-{:.1}", db.max(0.0)),
    };
    painter.text(
        egui::pos2(rect.center().x, rect.bottom() - 2.0),
        egui::Align2::CENTER_BOTTOM,
        readout,
        egui::FontId::proportional(9.0),
        theme::TEXT,
    );
}

fn value_to_y(db: f32, mode: LevelMode, rect: egui::Rect) -> f32 {
    match mode {
        LevelMode::FromBottom => {
            const BOT: f32 = -60.0;
            const TOP: f32 = 6.0;
            let t = ((db - BOT) / (TOP - BOT)).clamp(0.0, 1.0);
            rect.bottom() - t * rect.height()
        }
        LevelMode::HangingGr => {
            const TOP: f32 = 0.0;
            const BOT: f32 = 24.0;
            let t = ((db - TOP) / (BOT - TOP)).clamp(0.0, 1.0);
            rect.top() + t * rect.height()
        }
    }
}
