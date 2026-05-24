//! Pill-shaped chip button — used for articulation pickers and similar
//! discrete toggles.

use wayland_plugin_gui::egui;

use crate::editor::theme;

/// Draws a chip and returns true on click.
pub fn chip_button(ui: &mut egui::Ui, label: &str, active: bool) -> bool {
    let (bg, fg, stroke) = if active {
        (
            egui::Color32::from_rgba_unmultiplied(0x8b, 0x6d, 0xff, 0x18),
            theme::TEXT_1,
            theme::ACCENT,
        )
    } else {
        (theme::BG_1, theme::TEXT_2, theme::LINE_2)
    };
    let mut rt = egui::RichText::new(label).size(11.0).color(fg);
    if active {
        rt = rt.strong();
    }
    let btn = egui::Button::new(rt)
        .fill(bg)
        .stroke(egui::Stroke::new(1.0, stroke))
        .corner_radius(theme::RADIUS_CHIP)
        .min_size(egui::vec2(0.0, 22.0));
    let resp = ui.add(btn);
    resp.clicked()
}
