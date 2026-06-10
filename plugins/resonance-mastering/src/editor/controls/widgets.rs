//! Stage control widget helpers.
//!
//! Re-exports the shared param-bound knob / checkbox widgets from
//! `resonance_plugin::editor_widgets` and keeps `int_combo` locally
//! since the shared crate doesn't provide it.

pub use resonance_plugin::editor_widgets::{bool_checkbox, float_knob};

use resonance_plugin::{IntParam, Param};
use wayland_plugin_gui::egui;

/// Standard per-control column width for combo-box layouts.
pub const COL_WIDTH: f32 = 108.0;

/// Labeled combo box bound to an `IntParam`. `labels` is indexed by the
/// integer value (offset from range min).
pub fn int_combo(ui: &mut egui::Ui, param: &IntParam, id: &str, labels: &[&str]) {
    let current = param.value();
    let min = param.min_plain() as i32;
    let idx = (current - min).clamp(0, labels.len() as i32 - 1) as usize;
    let current_label = labels.get(idx).copied().unwrap_or("?");
    egui::ComboBox::from_id_salt(id)
        .width(COL_WIDTH - 16.0)
        .selected_text(current_label)
        .show_ui(ui, |ui| {
            for (i, label) in labels.iter().enumerate() {
                if ui.selectable_label(i == idx, *label).clicked() {
                    param.set_value(min + i as i32);
                }
            }
        });
}
