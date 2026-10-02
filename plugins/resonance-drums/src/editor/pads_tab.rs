//! The Pads tab (§6.2): the 6×5 pad grid on the left, the selected pad's
//! inspector on the right. Each scrolls on its own, so neither is ever
//! cut off at the window's minimum size.

use plugin_gui_core::egui;

use super::app::{column, DrumsEditorApp};
use super::{pad_grid, pad_inspector};

/// Gap between the grid and the inspector.
const GAP: f32 = 12.0;

pub(super) fn draw(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    let avail = ui.available_size();
    let grid_w = pad_grid::preferred_width(avail.x - GAP);
    let inspector_w = (avail.x - grid_w - GAP).max(0.0);
    let now = ui.ctx().input(|i| i.time);

    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(GAP, 0.0);
        column(ui, egui::vec2(grid_w, avail.y), |ui| {
            if let Some(click) = pad_grid::draw(ui, app, now) {
                app.selected_pad = click.pad;
                if let Some(velocity) = click.audition {
                    let note = crate::drum_map::PAD_MAPPINGS[click.pad].note;
                    app.bridge.audition_at(note, velocity);
                }
            }
        });
        column(ui, egui::vec2(inspector_w, avail.y), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("pad_inspector_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let catalog = app.bridge.catalog.lock().clone();
                    let hit = app.pad_hit(app.selected_pad);
                    pad_inspector::draw(
                        ui,
                        &app.bridge,
                        &catalog,
                        app.selected_pad,
                        &mut pad_inspector::InspectorState {
                            audition_velocity: &mut app.audition_velocity,
                            labels: &mut app.labels,
                            hit,
                        },
                    );
                });
        });
    });
}
