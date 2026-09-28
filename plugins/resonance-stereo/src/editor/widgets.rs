//! The one place the stereo editor turns a parameter into a control.
//!
//! Range, default, skew, unit, readout, caption and choice labels all
//! come off the parameter itself; nothing here restates a fact that
//! `params.rs` declares.

use plugin_gui_core::{egui, widgets};
use resonance_plugin::{editor_widgets, Param};

use crate::params::{ParamRef, StereoParams};

use super::theme;

/// Draw the control for parameter `index`, chosen by its declared type.
pub fn param_control(ui: &mut egui::Ui, params: &StereoParams, index: usize) {
    match params.param_ref(index) {
        ParamRef::Float(p) => editor_widgets::float_knob(ui, p, p.name(), ""),
        ParamRef::Bool(p) => {
            ui.vertical(|ui| {
                ui.add_space(12.0);
                editor_widgets::bool_checkbox(ui, p, p.name());
            });
        }
        ParamRef::Int(p) => {
            ui.label(egui::RichText::new(p.name()).color(theme::TEXT_2).size(10.5));
            let labels = p.choices().unwrap_or(&[]);
            let min = p.range().min();
            let selected = usize::try_from(p.value() - min).unwrap_or(0);
            if let Some(i) = widgets::segmented(ui, labels, selected) {
                p.set_plain(f64::from(min + i as i32));
            }
            ui.add_space(4.0);
        }
    }
}
