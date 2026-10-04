//! A load failure or a missing file, drawn as a banner over the centre
//! viz area instead of plain text in the filename slot (code review
//! PUX-09).
//!
//! `loader.rs::load_into`'s only error channel is `ir_name` itself: on
//! failure it writes `"Error: {e}"` there (its own doc comment says
//! so), which `header::draw` used to show verbatim as if it were a
//! loaded file's name — "Error: No such file or directory (os error
//! 2)" sitting where "Marshall 4x12" usually does, with no colour or
//! action to tell them apart. This mirrors the amp editor's
//! missing-model banner (`resonance-amp/src/editor/missing_banner.rs`,
//! nam-model-library.md §6.4): a WARM-bordered panel with the failure
//! and a `Locate…` button, in the one place this plugin has a
//! dedicated signal for "something is wrong" to draw from.

use plugin_gui_core::egui;

use super::{header, theme, IrEditorApp};

/// `ir_name`'s load-error message, if it currently holds one — the
/// `"Error: "` prefix `loader.rs` writes on a failed load. `None` for
/// every other state (nothing loaded yet, or a file loaded fine, even
/// one whose name happens to start the same way).
pub(crate) fn load_error(ir_name: &str) -> Option<&str> {
    ir_name.strip_prefix("Error: ")
}

/// Draw the banner over `rect` (the centre viz area, in place of the
/// waveform/response views) with `message` — `load_error`'s result.
pub(crate) fn draw(ui: &mut egui::Ui, rect: egui::Rect, app: &mut IrEditorApp, message: &str) {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(12.0)));
    egui::Frame::new()
        .fill(theme::PANEL)
        .stroke(egui::Stroke::new(1.0, theme::WARN))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::same(14))
        .show(&mut child, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new("⚠ Could not load the impulse response")
                    .color(theme::WARN)
                    .size(14.0)
                    .strong(),
            );
            ui.label(
                egui::RichText::new(message)
                    .color(theme::TEXT_DIM)
                    .size(11.0),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Locate…").clicked() {
                    header::start_load_ir(ui.ctx(), app);
                }
            });
        });
}
