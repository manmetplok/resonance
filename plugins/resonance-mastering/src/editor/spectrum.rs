//! 1/6-octave spectrum bar widget.
//!
//! Takes a `SpectrumHandle` from the metering crate, pulls the latest
//! held bins, and draws them as filled bars across a log-frequency axis.

use resonance_metering::{SpectrumHandle, NUM_OCTAVE_BINS};
use wayland_plugin_gui::egui;

use super::theme;

/// Minimum dB shown on the vertical axis.
const FLOOR_DB: f32 = -96.0;
/// Top of the vertical axis (a few dB above 0 so hot signals don't clip).
const CEILING_DB: f32 = 6.0;

pub fn draw(painter: &egui::Painter, rect: egui::Rect, handle: Option<&SpectrumHandle>) {
    draw_background(painter, rect);
    draw_gridlines(painter, rect);

    let Some(handle) = handle else {
        draw_placeholder(painter, rect, "waiting for audio…");
        return;
    };
    let snapshot = handle.latest();
    if snapshot.magnitudes_db.len() != NUM_OCTAVE_BINS {
        draw_placeholder(painter, rect, "spectrum not ready");
        return;
    }

    let bars = rect.width() / NUM_OCTAVE_BINS as f32;
    let bar_w = (bars - 1.0).max(1.0);
    for (i, &db) in snapshot.magnitudes_db.iter().enumerate() {
        let x = rect.left() + i as f32 * bars;
        let norm = ((db - FLOOR_DB) / (CEILING_DB - FLOOR_DB)).clamp(0.0, 1.0);
        let h = norm * rect.height();
        let bar_rect = egui::Rect::from_min_max(
            egui::pos2(x, rect.bottom() - h),
            egui::pos2(x + bar_w, rect.bottom()),
        );
        let color = bar_color(db);
        painter.rect_filled(bar_rect, 1.0, color);
    }
}

fn bar_color(db: f32) -> egui::Color32 {
    if db > -1.0 {
        theme::DANGER
    } else if db > -6.0 {
        theme::WARN
    } else {
        theme::ACCENT
    }
}

fn draw_background(painter: &egui::Painter, rect: egui::Rect) {
    painter.rect_filled(rect, 2.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );
}

fn draw_gridlines(painter: &egui::Painter, rect: egui::Rect) {
    // Horizontal dB gridlines every 12 dB.
    let mut db = CEILING_DB - ((CEILING_DB as i32).rem_euclid(12)) as f32;
    while db > FLOOR_DB {
        let norm = (db - FLOOR_DB) / (CEILING_DB - FLOOR_DB);
        let y = rect.bottom() - norm * rect.height();
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(1.0, theme::BORDER),
        );
        painter.text(
            egui::pos2(rect.left() + 4.0, y - 2.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{db:.0}"),
            egui::FontId::monospace(9.0),
            theme::TEXT_DIM,
        );
        db -= 12.0;
    }
}

fn draw_placeholder(painter: &egui::Painter, rect: egui::Rect, msg: &str) {
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        msg,
        egui::FontId::proportional(12.0),
        theme::TEXT_DIM,
    );
}
