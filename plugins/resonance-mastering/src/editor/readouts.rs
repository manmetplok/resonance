//! Numeric readout panels for PLR / PSR / Crest / LRA.

use wayland_plugin_gui::egui;

use super::theme;

pub struct Readouts {
    pub plr_db: f32,
    pub psr_db: f32,
    pub crest_db: f32,
    pub lra_lu: f32,
}

pub fn draw(painter: &egui::Painter, rect: egui::Rect, r: &Readouts) {
    painter.rect_filled(rect, 2.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    let rows = [
        ("PLR", r.plr_db, "dB"),
        ("PSR", r.psr_db, "dB"),
        ("Crest", r.crest_db, "dB"),
        ("LRA", r.lra_lu, "LU"),
    ];
    let row_h = rect.height() / rows.len() as f32;
    for (i, (label, value, unit)) in rows.iter().enumerate() {
        let y = rect.top() + (i as f32 + 0.5) * row_h;
        painter.text(
            egui::pos2(rect.left() + 10.0, y),
            egui::Align2::LEFT_CENTER,
            *label,
            egui::FontId::monospace(11.0),
            theme::TEXT_DIM,
        );
        let text = if value.is_finite() {
            format!("{value:>5.1} {unit}")
        } else {
            format!("  — {unit}")
        };
        painter.text(
            egui::pos2(rect.right() - 10.0, y),
            egui::Align2::RIGHT_CENTER,
            text,
            egui::FontId::monospace(11.0),
            theme::TEXT,
        );
    }
}
