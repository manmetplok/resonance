//! Stereo IN / OUT peak meters. Structurally identical to the reverb
//! editor's meters — keeps both plugins visually consistent. Input is
//! drawn dim and output bright so the eye naturally follows the amp's
//! processed signal.

use wayland_plugin_gui::egui;

use crate::viz::AmpViz;

use super::theme;

const FLOOR_DB: f32 = -48.0;
const CEILING_DB: f32 = 0.0;

pub fn draw(painter: &egui::Painter, rect: egui::Rect, viz: &AmpViz) {
    painter.rect_filled(rect, 3.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        3.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    let (in_l, in_r) = viz.read_in_peaks_db();
    let (out_l, out_r) = viz.read_out_peaks_db();

    let half_gap = 12.0;
    let half_w = (rect.width() - half_gap) * 0.5;
    let inp_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 4.0, rect.top() + 2.0),
        egui::pos2(rect.left() + 4.0 + half_w - 4.0, rect.bottom() - 2.0),
    );
    let out_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left() + half_w + half_gap, rect.top() + 2.0),
        egui::pos2(rect.right() - 4.0, rect.bottom() - 2.0),
    );

    draw_label(painter, inp_rect, "IN", theme::TEXT_DIM);
    draw_label(painter, out_rect, "OUT", theme::TEXT);
    let label_w = 28.0;

    let inp_bars = egui::Rect::from_min_max(
        egui::pos2(inp_rect.left() + label_w, inp_rect.top()),
        inp_rect.max,
    );
    let out_bars = egui::Rect::from_min_max(
        egui::pos2(out_rect.left() + label_w, out_rect.top()),
        out_rect.max,
    );

    draw_channel_pair(painter, inp_bars, in_l, in_r, theme::ACCENT_DIM);
    draw_channel_pair(painter, out_bars, out_l, out_r, theme::ACCENT);
}

fn draw_channel_pair(
    painter: &egui::Painter,
    rect: egui::Rect,
    l_db: f32,
    r_db: f32,
    fill: egui::Color32,
) {
    let bar_h = (rect.height() - 2.0) * 0.5;
    let top = egui::Rect::from_min_max(
        rect.min,
        egui::pos2(rect.right(), rect.top() + bar_h),
    );
    let bot = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.bottom() - bar_h),
        rect.max,
    );
    draw_bar(painter, top, l_db, fill);
    draw_bar(painter, bot, r_db, fill);
}

fn draw_bar(painter: &egui::Painter, rect: egui::Rect, db: f32, fill: egui::Color32) {
    painter.rect_filled(rect, 1.0, theme::BG);
    let norm = ((db - FLOOR_DB) / (CEILING_DB - FLOOR_DB)).clamp(0.0, 1.0);
    if norm <= 0.0 {
        return;
    }
    let w = norm * rect.width();
    let fill_rect = egui::Rect::from_min_max(
        rect.min,
        egui::pos2(rect.left() + w, rect.bottom()),
    );
    let color = if db > -1.0 {
        theme::DANGER
    } else if db > -6.0 {
        theme::WARN
    } else {
        fill
    };
    painter.rect_filled(fill_rect, 1.0, color);
}

fn draw_label(painter: &egui::Painter, rect: egui::Rect, label: &str, color: egui::Color32) {
    painter.text(
        egui::pos2(rect.left() + 4.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(11.0),
        color,
    );
}
