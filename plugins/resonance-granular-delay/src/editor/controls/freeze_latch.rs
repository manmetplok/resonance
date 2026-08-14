//! The large amber FREEZE latch of the OUTPUT group (design doc #264
//! req-5) — a granular-specific composite rather than a generic widget.

use egui::Ui;
use wayland_plugin_gui::egui;

use crate::params::GranularDelayParams;

use super::super::theme;

/// The large amber freeze latch (design doc #264 req-5): warm-token
/// latching button with a glow/filled state while engaged, sized for
/// the OUTPUT group.
pub fn freeze_latch(ui: &mut Ui, params: &GranularDelayParams, index: usize) {
    let p = params.param_at(index);
    let on = p.get_plain() >= 0.5;
    let size = egui::vec2(96.0, 44.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        // Glow halo outside the button while engaged.
        if on {
            let painter = ui.painter();
            for (expand, alpha) in [(5.0, 0.10), (3.0, 0.18), (1.5, 0.30)] {
                painter.rect_stroke(
                    rect.expand(expand),
                    theme::RADIUS_CHIP + expand,
                    egui::Stroke::new(2.0, theme::WARM.gamma_multiply(alpha)),
                    egui::StrokeKind::Outside,
                );
            }
        }
        let painter = ui.painter_at(rect.expand(1.0));
        let (fill, text_color, stroke) = if on {
            (theme::WARM, theme::BG_0, theme::WARM)
        } else if response.hovered() {
            (theme::BG_3, theme::WARM, theme::WARM.gamma_multiply(0.6))
        } else {
            (theme::BG_1, theme::WARM.gamma_multiply(0.8), theme::LINE)
        };
        painter.rect_filled(rect, theme::RADIUS_CHIP, fill);
        painter.rect_stroke(
            rect,
            theme::RADIUS_CHIP,
            egui::Stroke::new(1.0, stroke),
            egui::StrokeKind::Inside,
        );
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "FREEZE",
            egui::FontId::proportional(12.0),
            text_color,
        );
    }
    if response.clicked() {
        p.set_plain(if on { 0.0 } else { 1.0 });
    }
}
