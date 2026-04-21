//! FX tab: chorus, delay, distortion + master output oscilloscope.

use wayland_plugin_gui::egui;

use crate::editor::viz::scope;
use crate::editor::WavetableEditorApp;

use super::{bool_checkbox, float_slider, section_header};

pub fn draw(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    ui.spacing_mut().item_spacing.y = 6.0;

    // Output oscilloscope.
    section_header(ui, "OUTPUT");
    let (_id, rect) = ui.allocate_space(egui::vec2(ui.available_width() - 4.0, 72.0));
    scope::draw(ui, rect, &app.snapshot.scope_samples);

    ui.add_space(8.0);

    ui.columns(3, |cols| {
        // Chorus.
        section_header(&mut cols[0], "CHORUS");
        bool_checkbox(&mut cols[0], "Enabled", &app.params.chorus.enabled);
        float_slider(&mut cols[0], "Rate", &app.params.chorus.rate, Some(" Hz"));
        float_slider(&mut cols[0], "Depth", &app.params.chorus.depth, None);
        float_slider(&mut cols[0], "Mix", &app.params.chorus.mix, None);

        // Delay.
        section_header(&mut cols[1], "DELAY");
        bool_checkbox(&mut cols[1], "Enabled", &app.params.delay.enabled);
        float_slider(
            &mut cols[1],
            "Time L",
            &app.params.delay.time_l,
            Some(" ms"),
        );
        float_slider(
            &mut cols[1],
            "Time R",
            &app.params.delay.time_r,
            Some(" ms"),
        );
        float_slider(&mut cols[1], "Feedback", &app.params.delay.feedback, None);
        float_slider(&mut cols[1], "Mix", &app.params.delay.mix, None);

        // Distortion.
        section_header(&mut cols[2], "DISTORTION");
        bool_checkbox(&mut cols[2], "Enabled", &app.params.distortion.enabled);
        float_slider(&mut cols[2], "Drive", &app.params.distortion.drive, None);
        float_slider(&mut cols[2], "Mix", &app.params.distortion.mix, None);
    });
}
