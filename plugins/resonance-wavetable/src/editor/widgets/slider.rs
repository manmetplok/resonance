//! Horizontal slider with accent fill + circular thumb. Optional bipolar
//! mode fills outward from the centre line.

use wayland_plugin_gui::egui;

use crate::editor::theme;

const HEIGHT: f32 = 18.0;
const TRACK_HEIGHT: f32 = 3.0;
const THUMB_RADIUS: f32 = 5.5;

/// Bipolar slider: `value_unit` -1..1. Returns the new bipolar value if dragged.
pub fn slider_bipolar(ui: &mut egui::Ui, width: f32, value_signed: f32) -> Option<f32> {
    let unit = (value_signed + 1.0) * 0.5;
    draw(ui, width, unit, true).map(|u| u * 2.0 - 1.0)
}

fn draw(ui: &mut egui::Ui, width: f32, value_unit: f32, bipolar: bool) -> Option<f32> {
    let size = egui::vec2(width, HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    let track_y = rect.center().y;
    let track_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left(), track_y - TRACK_HEIGHT * 0.5),
        egui::vec2(rect.width(), TRACK_HEIGHT),
    );
    painter.rect_filled(track_rect, 1.5, theme::BG_1);
    painter.rect_stroke(
        track_rect,
        1.5,
        egui::Stroke::new(1.0, theme::LINE_2),
        egui::StrokeKind::Inside,
    );

    let v = value_unit.clamp(0.0, 1.0);
    let thumb_x = rect.left() + v * rect.width();

    if bipolar {
        let center_x = rect.center().x;
        if thumb_x >= center_x {
            let fill = egui::Rect::from_min_max(
                egui::pos2(center_x, track_rect.top()),
                egui::pos2(thumb_x, track_rect.bottom()),
            );
            painter.rect_filled(fill, 1.5, theme::ACCENT);
        } else {
            let fill = egui::Rect::from_min_max(
                egui::pos2(thumb_x, track_rect.top()),
                egui::pos2(center_x, track_rect.bottom()),
            );
            painter.rect_filled(fill, 1.5, theme::WARM);
        }
        // Centre tick.
        painter.line_segment(
            [
                egui::pos2(center_x, track_y - 4.0),
                egui::pos2(center_x, track_y + 4.0),
            ],
            egui::Stroke::new(1.0, theme::TEXT_4),
        );
    } else {
        let fill = egui::Rect::from_min_max(
            egui::pos2(rect.left(), track_rect.top()),
            egui::pos2(thumb_x, track_rect.bottom()),
        );
        painter.rect_filled(fill, 1.5, theme::ACCENT);
    }

    // Thumb.
    let thumb_color = if response.hovered() {
        theme::ACCENT_SOFT
    } else {
        theme::BG_3
    };
    painter.circle_filled(
        egui::pos2(thumb_x, track_y),
        THUMB_RADIUS,
        thumb_color,
    );
    painter.circle_stroke(
        egui::pos2(thumb_x, track_y),
        THUMB_RADIUS,
        egui::Stroke::new(1.5, theme::ACCENT),
    );

    // Interaction: any click/drag positions the value.
    let mut new_value: Option<f32> = None;
    if response.dragged() || response.clicked() {
        if let Some(p) = response.interact_pointer_pos() {
            let frac = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            new_value = Some(frac);
        }
    }
    new_value
}
