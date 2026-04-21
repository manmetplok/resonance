//! 8-channel FDN "tank" view.
//!
//! Draws each of the 8 FDN feedback channels as a vertical energy
//! column. Height tracks the smoothed |feedback| published by the
//! audio thread, so the columns breathe with the tail. Below each
//! column is a small monospace label showing the current delay-line
//! length in ms — these move as the user scrubs `Size`, making the
//! invisible DSP state tangible.
//!
//! At the base of the columns a thin horizontal "mix bus" bar shows
//! the summed energy going through the Householder reflection — a
//! visual echo of the feedback matrix at the heart of the reverb.

use wayland_plugin_gui::egui;

use crate::viz::FDN_CHANNELS;

use super::theme;
use super::ReverbEditorApp;

pub fn draw(painter: &egui::Painter, rect: egui::Rect, app: &ReverbEditorApp) {
    // Panel background.
    painter.rect_filled(rect, 3.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        3.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    // Header label in the top-left corner.
    painter.text(
        egui::pos2(rect.left() + 10.0, rect.top() + 8.0),
        egui::Align2::LEFT_TOP,
        "FDN TANK",
        egui::FontId::proportional(11.0),
        theme::TEXT_DIM,
    );
    painter.text(
        egui::pos2(rect.right() - 10.0, rect.top() + 8.0),
        egui::Align2::RIGHT_TOP,
        "8 ch · Householder",
        egui::FontId::proportional(9.0),
        theme::TEXT_DIM,
    );

    // Reserved area for the columns.
    let pad_top = 28.0f32;
    let label_h = 14.0f32;
    let bus_h = 4.0f32;
    let gap_between = 6.0f32;
    let columns_top = rect.top() + pad_top;
    let columns_bottom = rect.bottom() - label_h - gap_between - bus_h - 10.0;
    let columns_area = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 16.0, columns_top),
        egui::pos2(rect.right() - 16.0, columns_bottom),
    );

    // Column geometry.
    let col_gap = 6.0f32;
    let col_w =
        (columns_area.width() - col_gap * (FDN_CHANNELS as f32 - 1.0)) / FDN_CHANNELS as f32;
    let max_col_h = columns_area.height();

    let energies = app.viz.read_channel_energies();
    let delays_ms = app.viz.read_fdn_delay_ms();

    // Normalise energies against the loudest channel so the viz is
    // always occupying the vertical range nicely, regardless of input
    // level. Fall back to a fixed scale when everything is silent so
    // idle channels don't blow up to full height.
    let peak = energies.iter().copied().fold(0.0f32, f32::max).max(0.05);

    for c in 0..FDN_CHANNELS {
        let x_left = columns_area.left() + c as f32 * (col_w + col_gap);
        let col_rect = egui::Rect::from_min_max(
            egui::pos2(x_left, columns_area.top()),
            egui::pos2(x_left + col_w, columns_area.bottom()),
        );
        // Trough.
        painter.rect_filled(col_rect, 2.0, theme::BG);
        painter.rect_stroke(
            col_rect,
            2.0,
            egui::Stroke::new(0.5, theme::BORDER),
            egui::StrokeKind::Inside,
        );

        let norm = (energies[c] / peak).clamp(0.0, 1.0);
        // Mild non-linearity so a channel at 20% peak is still visible.
        let display = norm.powf(0.55);
        let fill_h = display * max_col_h;
        let fill_rect = egui::Rect::from_min_max(
            egui::pos2(x_left + 1.0, col_rect.bottom() - fill_h),
            egui::pos2(x_left + col_w - 1.0, col_rect.bottom()),
        );

        // Gradient-ish fill via two stacked rects: dim base + bright cap.
        painter.rect_filled(fill_rect, 1.0, theme::ACCENT_DIM);
        let cap_h = (fill_h * 0.35).min(18.0);
        if cap_h > 0.5 {
            let cap_rect = egui::Rect::from_min_max(
                fill_rect.min,
                egui::pos2(fill_rect.right(), fill_rect.top() + cap_h),
            );
            painter.rect_filled(cap_rect, 1.0, theme::ACCENT);
        }

        // Delay-length label under the column.
        painter.text(
            egui::pos2(col_rect.center().x, col_rect.bottom() + 4.0),
            egui::Align2::CENTER_TOP,
            format!("{:.0}", delays_ms[c]),
            egui::FontId::monospace(9.0),
            theme::TEXT_DIM,
        );
    }

    // "ms" suffix above the delay labels.
    painter.text(
        egui::pos2(columns_area.right(), columns_area.bottom() + 4.0),
        egui::Align2::LEFT_TOP,
        "ms",
        egui::FontId::proportional(9.0),
        theme::TEXT_DIM,
    );

    // Mix bus: thin horizontal bar that pulses with the summed tank energy.
    let bus_top = rect.bottom() - bus_h - 10.0;
    let bus_rect = egui::Rect::from_min_max(
        egui::pos2(columns_area.left(), bus_top),
        egui::pos2(columns_area.right(), bus_top + bus_h),
    );
    painter.rect_filled(bus_rect, 1.0, theme::BORDER);
    let sum: f32 = energies.iter().sum();
    let bus_norm = (sum / (peak * FDN_CHANNELS as f32 * 0.5))
        .clamp(0.0, 1.0)
        .powf(0.6);
    let bus_fill = egui::Rect::from_min_max(
        bus_rect.min,
        egui::pos2(
            bus_rect.left() + bus_norm * bus_rect.width(),
            bus_rect.bottom(),
        ),
    );
    painter.rect_filled(bus_fill, 1.0, theme::ACCENT);
}
