//! OSC tab: wavetable viewer + osc selector + per-osc controls + unison.

use wayland_plugin_gui::egui;

use crate::editor::display_waves;
use crate::editor::theme;
use crate::editor::viz::{frame_strip, waveform};
use crate::editor::WavetableEditorApp;
use resonance_plugin::param::Param;

use super::{bool_checkbox, float_slider, int_slider, section_header};

pub fn draw(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    ui.spacing_mut().item_spacing.y = 6.0;

    // Osc selector row.
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("OSCILLATOR").color(theme::TEXT_DIM));
        ui.selectable_value(&mut app.selected_osc, 0, "Osc 1");
        ui.selectable_value(&mut app.selected_osc, 1, "Osc 2");
        ui.add_space(12.0);
        // Osc balance slider — always visible on this tab.
        let bal = &app.params.osc_balance;
        let mut v = bal.value();
        let resp = ui.add(
            egui::Slider::new(&mut v, -1.0..=1.0)
                .text("Balance")
                .custom_formatter(|x, _| format!("{:+.2}", x)),
        );
        if resp.changed() {
            bal.set_value(v);
        }
    });

    ui.add_space(4.0);

    let (osc_params, live_pos) = if app.selected_osc == 0 {
        (&app.params.osc1, app.snapshot.osc1_position_live)
    } else {
        (&app.params.osc2, app.snapshot.osc2_position_live)
    };

    let wt_idx = osc_params.wavetable.value() as usize;
    let position = osc_params.position.value();

    // Wavetable viewer + right-side controls row.
    ui.horizontal(|ui| {
        // Left side: viewer + frame strip + wavetable picker.
        ui.vertical(|ui| {
            let (viewer_id, viewer_rect) = ui.allocate_space(egui::vec2(340.0, 170.0));
            let _ = viewer_id;
            waveform::draw(ui, viewer_rect, wt_idx, position, live_pos);

            let (strip_id, strip_rect) = ui.allocate_space(egui::vec2(340.0, 22.0));
            let _ = strip_id;
            frame_strip::draw(ui, strip_rect, wt_idx, position);

            // Wavetable picker.
            ui.horizontal(|ui| {
                let prev = ui.button("<");
                ui.label(
                    egui::RichText::new(display_waves::wavetable_name(wt_idx)).color(theme::ACCENT),
                );
                let next = ui.button(">");
                if prev.clicked() && wt_idx > 0 {
                    osc_params.wavetable.set_plain((wt_idx - 1) as f64);
                }
                if next.clicked() && wt_idx + 1 < display_waves::WAVETABLE_NAMES.len() {
                    osc_params.wavetable.set_plain((wt_idx + 1) as f64);
                }
                ui.label(
                    egui::RichText::new(format!(
                        "{}/{}",
                        wt_idx + 1,
                        display_waves::WAVETABLE_NAMES.len()
                    ))
                    .color(theme::TEXT_DIM)
                    .size(10.0),
                );
            });
        });

        ui.add_space(12.0);

        // Right side: osc controls.
        ui.vertical(|ui| {
            section_header(
                ui,
                if app.selected_osc == 0 {
                    "OSC 1"
                } else {
                    "OSC 2"
                },
            );
            bool_checkbox(ui, "Enabled", &osc_params.enabled);
            float_slider(ui, "Position", &osc_params.position, Some("%"));
            int_slider(ui, "Coarse", &osc_params.coarse);
            float_slider(ui, "Fine", &osc_params.fine, Some(" ct"));
            float_slider(ui, "Level", &osc_params.level, Some("%"));
            float_slider(ui, "Pan", &osc_params.pan, None);
        });
    });

    ui.add_space(6.0);
    section_header(ui, "UNISON");
    ui.horizontal(|ui| {
        int_slider(ui, "Voices", &app.params.unison.voices);
        float_slider(ui, "Detune", &app.params.unison.detune, Some(" ct"));
        float_slider(ui, "Spread", &app.params.unison.spread, Some("%"));
    });

    ui.add_space(4.0);
    section_header(ui, "GLOBAL");
    ui.horizontal(|ui| {
        float_slider(ui, "Master", &app.params.master_volume, None);
        float_slider(ui, "Glide ms", &app.params.glide_time, None);
        bool_checkbox(ui, "Glide on", &app.params.glide_enabled);
        int_slider(ui, "Max voices", &app.params.max_voices);
    });
}
