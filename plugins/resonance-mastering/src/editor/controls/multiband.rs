//! Multiband compressor control panel.
//!
//! Top row: master on + three crossover frequency knobs.
//! Below it: four per-band groups. Each band is a full glue compressor,
//! so it gets the glue stage's vocabulary in the glue stage's order —
//! Threshold / Ratio / Gain on the first line, the ballistics (Attack,
//! Release, Knee, Mix) on the second. Four knobs per line keeps each
//! band one fixed-width column, so the four columns stay aligned and
//! the panel reads as a grid rather than a wall of dials.
//!
//! Every knob binds straight to its `FloatParam`, so range, skew and
//! printed value all come off the parameter — there is no second copy
//! of those facts here to drift out of sync with the DSP.

use wayland_plugin_gui::egui;

use crate::params::{MultibandBandParams, MultibandParams};
use crate::stages::multiband::NUM_BANDS;

use super::gr_meter;
use super::theme;
use super::widgets;

/// Footprint of one knob cell in `wayland_plugin_gui::widgets::knob`
/// (`total_width` and `knob_radius * 2 + 36`).
const KNOB_W: f32 = 64.0;
const KNOB_H: f32 = 76.0;
/// Knobs per line inside a band column — also what sets the column width.
const KNOBS_PER_LINE: usize = 4;
/// One band column, wide enough for a full line of knobs.
pub const BAND_COLUMN_W: f32 = KNOB_W * KNOBS_PER_LINE as f32;
/// Gap between band columns.
const BAND_GAP: f32 = 8.0;
/// Left margin the band row starts at.
const LEFT_MARGIN: f32 = 8.0;

/// Total width the four band columns occupy, left margin included. The
/// stage panel spans the full window width, so this must stay inside it.
pub const BANDS_W: f32 =
    LEFT_MARGIN + BAND_COLUMN_W * NUM_BANDS as f32 + BAND_GAP * (NUM_BANDS as f32 - 1.0);

/// Height this panel needs: the crossover row, then each band's title
/// row, its gain-reduction meter and two knob lines.
/// `stage_panel_height` must give it at least this much or the bottom
/// line of knobs is clipped.
pub const REQUIRED_PANEL_H: f32 = 6.0
    + KNOB_H
    + 4.0
    + 18.0
    + 3.0
    + gr_meter::HEIGHT
    + 3.0
    + KNOB_H
    + 3.0
    + KNOB_H;

/// Band names, low to high.
pub const BAND_NAMES: [&str; NUM_BANDS] = ["Low", "Low-Mid", "High-Mid", "High"];

/// `band_gr_db` is the live gain reduction of each band's compressor in
/// dB (positive = attenuation), low band first, as published by the
/// audio thread.
pub fn draw(ui: &mut egui::Ui, params: &MultibandParams, band_gr_db: [f32; NUM_BANDS]) {
    ui.vertical(|ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new("Multiband")
                    .strong()
                    .size(14.0)
                    .color(theme::ACCENT),
            );
            ui.add_space(16.0);
            widgets::bool_checkbox(ui, &params.on, "Enabled");
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("Crossovers:")
                    .size(11.0)
                    .color(theme::TEXT_DIM),
            );
            ui.add_space(4.0);

            widgets::float_knob(ui, &params.xo1, "LO/LM", "");
            widgets::float_knob(ui, &params.xo2, "LM/HM", "");
            widgets::float_knob(ui, &params.xo3, "HM/HI", "");
        });
        ui.add_space(4.0);

        ui.horizontal_top(|ui| {
            ui.add_space(LEFT_MARGIN);
            for (i, name) in BAND_NAMES.iter().enumerate() {
                draw_band(ui, &params.bands[i], name, band_gr_db[i]);
                if i + 1 < NUM_BANDS {
                    ui.add_space(BAND_GAP);
                }
            }
        });
    });
}

fn draw_band(ui: &mut egui::Ui, band: &MultibandBandParams, title: &str, gr_db: f32) {
    ui.vertical(|ui| {
        ui.set_min_width(BAND_COLUMN_W);
        ui.set_max_width(BAND_COLUMN_W);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(title)
                    .strong()
                    .size(11.0)
                    .color(theme::TEXT),
            );
            widgets::bool_checkbox(ui, &band.on, "On");
        });
        // Live gain reduction, so Threshold is set with feedback rather
        // than blind. Sits directly under the band's name, above the
        // controls that cause it.
        let (meter_rect, _) = ui.allocate_exact_size(
            egui::vec2(BAND_COLUMN_W, gr_meter::HEIGHT),
            egui::Sense::hover(),
        );
        gr_meter::draw(&ui.painter_at(meter_rect), meter_rect, gr_db);
        // Line 1 — what the band does. Gain is a band output trim, so it
        // works with this band's compressor off, which is what makes the
        // stage usable as a static four-band tone balancer.
        ui.horizontal(|ui| {
            widgets::float_knob(ui, &band.threshold, "Threshold", "");
            widgets::float_knob(ui, &band.ratio, "Ratio", "");
            widgets::float_knob(ui, &band.gain, "Gain", "band out");
        });
        // Line 2 — how it does it. Same names, ranges and order as the
        // Glue Comp tab, so the two stages read as one vocabulary.
        ui.horizontal(|ui| {
            widgets::float_knob(ui, &band.attack, "Attack", "");
            widgets::float_knob(ui, &band.release, "Release", "per band");
            widgets::float_knob(ui, &band.knee, "Knee", "");
            widgets::float_knob(ui, &band.mix, "Mix", "parallel");
        });
    });
}
