//! Pad list / grid view down the left side of the editor.
//!
//! Renders one selectable row per pad, with a right-aligned round-robin
//! indicator showing the most recently played RR index for that pad.

use std::sync::atomic::Ordering;

use wayland_plugin_gui::egui;

use crate::drum_map::{NUM_PADS, PAD_MAPPINGS};
use crate::KitBridge;

use super::theme;

/// Decode a packed `rr_index | (n_rrs << 16)` atomic value into
/// `(rr_index, n_rrs)`. Returns `None` for the sentinel zero (pad
/// never triggered).
fn unpack_rr_display(packed: u32) -> Option<(usize, usize)> {
    let n_rrs = (packed >> 16) as usize;
    if n_rrs == 0 {
        return None;
    }
    let rr_index = (packed & 0xFFFF) as usize;
    Some((rr_index, n_rrs))
}

/// Render the pad list as a left side panel. Mutates `selected_pad` when
/// the user clicks a row.
pub fn draw(ui: &mut egui::Ui, bridge: &KitBridge, selected_pad: &mut usize) {
    #[allow(deprecated)] // SidePanel -> Panel::left rename; current API on this egui version
    egui::SidePanel::left("drum_pad_list")
        .default_size(150.0)
        .resizable(false)
        .show_inside(ui, |ui| {
            ui.label(
                egui::RichText::new("PADS")
                    .size(10.0)
                    .strong()
                    .color(theme::TEXT_DIM),
            );
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (i, mapping) in PAD_MAPPINGS.iter().enumerate().take(NUM_PADS) {
                    let name = mapping.name;
                    let rr_label =
                        unpack_rr_display(bridge.last_rr[i].load(Ordering::Relaxed));
                    let selected = *selected_pad == i;
                    ui.horizontal(|ui| {
                        if ui.selectable_label(selected, name).clicked() {
                            *selected_pad = i;
                        }
                        if let Some((idx, total)) = rr_label {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(format!("{}/{}", idx + 1, total))
                                            .size(9.0)
                                            .color(theme::TEXT_DIM),
                                    );
                                },
                            );
                        }
                    });
                }
            });
        });
}
