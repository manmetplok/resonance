//! Segmented control — a pill of buttons where exactly one is "on".

use wayland_plugin_gui::egui;

use crate::editor::theme;

/// Draw a segmented selector. Returns the index that was clicked (if any).
pub fn segmented(
    ui: &mut egui::Ui,
    labels: &[&str],
    selected: usize,
    led_for_active: bool,
) -> Option<usize> {
    let mut clicked = None;
    let frame = egui::Frame::default()
        .fill(theme::BG_2)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(7.0)
        .inner_margin(egui::Margin::same(3));
    frame.show(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
        ui.horizontal(|ui| {
            for (i, label) in labels.iter().enumerate() {
                let is_on = i == selected;
                let bg = if is_on { theme::BG_3 } else { theme::BG_2 };
                let fg = if is_on { theme::TEXT_1 } else { theme::TEXT_2 };
                let mut rt = egui::RichText::new(*label).size(11.5).color(fg);
                if is_on {
                    rt = rt.strong();
                }
                let btn = egui::Button::new(rt)
                    .fill(bg)
                    .stroke(egui::Stroke::NONE)
                    .corner_radius(5.0)
                    .min_size(egui::vec2(0.0, 22.0));
                let resp = ui.add(btn);
                if led_for_active && is_on {
                    let pos = resp.rect.left_center();
                    // We can't easily inject a glyph inside the button label;
                    // skip the LED inline rendering — callers that need it
                    // can place a small circle to the left themselves.
                    let _ = pos;
                }
                if resp.clicked() {
                    clicked = Some(i);
                }
            }
        });
    });
    clicked
}
