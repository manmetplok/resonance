//! ENV / FILTER tab: two ADSR envelope curves and the filter response.

use wayland_plugin_gui::egui;

use crate::editor::viz::{envelope, filter_response};
use crate::editor::WavetableEditorApp;

use super::{bool_checkbox, float_slider, int_slider, section_header};

pub fn draw(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    ui.spacing_mut().item_spacing.y = 6.0;

    ui.columns(2, |cols| {
        // -- Amp envelope viewer + sliders --
        let col = &mut cols[0];
        section_header(col, "AMP ENV");
        let (_id, rect) = col.allocate_space(egui::vec2(col.available_width(), 110.0));
        let adsr = envelope::Adsr {
            attack: app.params.amp_env.attack.value(),
            decay: app.params.amp_env.decay.value(),
            sustain: app.params.amp_env.sustain.value(),
            release: app.params.amp_env.release.value(),
            curve: app.params.amp_env.curve.value(),
        };
        envelope::draw(
            col,
            rect,
            &adsr,
            Some(app.snapshot.env_amp_value),
            app.snapshot.env_amp_stage,
        );
        float_slider(col, "Attack", &app.params.amp_env.attack, Some(" s"));
        float_slider(col, "Decay", &app.params.amp_env.decay, Some(" s"));
        float_slider(col, "Sustain", &app.params.amp_env.sustain, None);
        float_slider(col, "Release", &app.params.amp_env.release, Some(" s"));
        float_slider(col, "Curve", &app.params.amp_env.curve, None);

        // -- Mod envelope viewer + sliders --
        let col = &mut cols[1];
        section_header(col, "MOD ENV");
        let (_id, rect) = col.allocate_space(egui::vec2(col.available_width(), 110.0));
        let adsr = envelope::Adsr {
            attack: app.params.mod_env.attack.value(),
            decay: app.params.mod_env.decay.value(),
            sustain: app.params.mod_env.sustain.value(),
            release: app.params.mod_env.release.value(),
            curve: app.params.mod_env.curve.value(),
        };
        envelope::draw(col, rect, &adsr, Some(app.snapshot.env_mod_value), 0);
        float_slider(col, "Attack", &app.params.mod_env.attack, Some(" s"));
        float_slider(col, "Decay", &app.params.mod_env.decay, Some(" s"));
        float_slider(col, "Sustain", &app.params.mod_env.sustain, None);
        float_slider(col, "Release", &app.params.mod_env.release, Some(" s"));
        float_slider(col, "Curve", &app.params.mod_env.curve, None);
    });

    ui.add_space(8.0);
    section_header(ui, "FILTER");
    ui.columns(2, |cols| {
        let col = &mut cols[0];
        let (_id, rect) = col.allocate_space(egui::vec2(col.available_width(), 150.0));
        filter_response::draw(
            col,
            rect,
            app.params.filter.filter_type.value(),
            app.params.filter.cutoff.value(),
            app.params.filter.resonance.value(),
            app.params.filter.drive.value(),
            app.snapshot.filter_cutoff_live,
        );

        let col = &mut cols[1];
        bool_checkbox(col, "Enabled", &app.params.filter.enabled);
        int_slider(col, "Type", &app.params.filter.filter_type);
        float_slider(col, "Cutoff", &app.params.filter.cutoff, Some(" Hz"));
        float_slider(col, "Resonance", &app.params.filter.resonance, None);
        float_slider(col, "Env Depth", &app.params.filter.env_depth, None);
        float_slider(col, "Key Track", &app.params.filter.keytrack, None);
        float_slider(col, "Drive", &app.params.filter.drive, None);
    });
}
