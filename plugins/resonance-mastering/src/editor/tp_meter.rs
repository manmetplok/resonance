//! Stereo true-peak bars in dBTP with a 0 dBTP ceiling line.

use wayland_plugin_gui::egui;

use super::theme;

const FLOOR_DBTP: f32 = -60.0;
const CEILING_DBTP: f32 = 3.0;

pub fn draw(painter: &egui::Painter, rect: egui::Rect, left: f32, right: f32) {
    painter.rect_filled(rect, 2.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    painter.text(
        egui::pos2(rect.center().x, rect.top() + 6.0),
        egui::Align2::CENTER_TOP,
        "dBTP",
        egui::FontId::monospace(10.0),
        theme::TEXT_DIM,
    );

    let body = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 4.0, rect.top() + 22.0),
        egui::pos2(rect.right() - 4.0, rect.bottom() - 22.0),
    );

    // 0 dBTP reference line.
    let zero = dbtp_to_y(0.0, body);
    painter.line_segment(
        [
            egui::pos2(body.left(), zero),
            egui::pos2(body.right(), zero),
        ],
        egui::Stroke::new(1.5, theme::DANGER),
    );

    let half = body.width() * 0.5 - 2.0;
    let l_rect = egui::Rect::from_min_max(
        egui::pos2(body.left(), body.top()),
        egui::pos2(body.left() + half, body.bottom()),
    );
    let r_rect = egui::Rect::from_min_max(
        egui::pos2(body.right() - half, body.top()),
        egui::pos2(body.right(), body.bottom()),
    );

    draw_bar(painter, l_rect, left, 'L');
    draw_bar(painter, r_rect, right, 'R');

    // Numeric readout for the max of the two.
    let peak = left.max(right);
    let txt = if peak.is_finite() && peak > FLOOR_DBTP {
        format!("{peak:>5.1}")
    } else {
        "  —".to_string()
    };
    painter.text(
        egui::pos2(rect.center().x, rect.bottom() - 12.0),
        egui::Align2::CENTER_CENTER,
        txt,
        egui::FontId::monospace(10.0),
        color_for_dbtp(peak),
    );
}

fn draw_bar(painter: &egui::Painter, rect: egui::Rect, dbtp: f32, label: char) {
    painter.rect_filled(rect, 1.0, theme::BG);
    if dbtp.is_finite() && dbtp > FLOOR_DBTP {
        let top_y = dbtp_to_y(dbtp, rect);
        let fill = egui::Rect::from_min_max(
            egui::pos2(rect.left(), top_y),
            egui::pos2(rect.right(), rect.bottom()),
        );
        painter.rect_filled(fill, 1.0, color_for_dbtp(dbtp));
    }
    painter.text(
        egui::pos2(rect.center().x, rect.top() - 10.0),
        egui::Align2::CENTER_BOTTOM,
        label,
        egui::FontId::monospace(9.0),
        theme::TEXT_DIM,
    );
}

fn dbtp_to_y(dbtp: f32, rect: egui::Rect) -> f32 {
    let norm = ((dbtp - FLOOR_DBTP) / (CEILING_DBTP - FLOOR_DBTP)).clamp(0.0, 1.0);
    rect.bottom() - norm * rect.height()
}

fn color_for_dbtp(dbtp: f32) -> egui::Color32 {
    if dbtp > -0.5 {
        theme::DANGER
    } else if dbtp > -3.0 {
        theme::WARN
    } else {
        theme::ACCENT
    }
}
