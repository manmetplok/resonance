//! Per-band control strip drawn at the bottom of the EQ editor.
//!
//! Freq / Gain / Q are the shared kit's slider (ba todo #1335). They
//! were raw `egui::Slider`s — the only sliders left in the fleet that
//! were not the widget every other editor draws — so the band strip now
//! matches the rest of the plugins: a thin track with a filled span and
//! a circular thumb, and Gain fills centre-out from 0 dB with a centre
//! tick, which is what a ±24 dB control should look like.
//!
//! Two things moved out of the widget and into this file as a result:
//! the logarithmic mapping of Freq and Q (the shared slider works in
//! unit travel and leaves the curve to the caller, the same contract
//! `resonance_plugin::editor_widgets` uses) and the palette, which was
//! `SliderStyle::CLASSIC` while this editor was still on the old blue
//! palette. Ba todo #1338 moved it: [`SliderStyle::LAVENDER`] is the
//! only palette now, and since the two differed in colour alone, nothing
//! else here changed.

use plugin_gui_core::egui;
use plugin_gui_core::widgets::{slider, HSlider, SliderStyle};
use resonance_plugin::{FloatParam, Param};

use crate::band::{BandKind, BandMs, BandSlope};
use crate::params::NUM_BANDS;

use super::app::EqEditorApp;
use super::theme;

/// Width of a band column's sliders, px — the `slider_width` the raw
/// `egui::Slider`s were laid out with, so the 104 px column is unchanged.
const SLIDER_W: f32 = 92.0;

pub(crate) fn draw_band_strip(ui: &mut egui::Ui, app: &mut EqEditorApp) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        for i in 0..NUM_BANDS {
            draw_band_column(ui, app, i);
            ui.add_space(4.0);
        }
    });
}

fn draw_band_column(ui: &mut egui::Ui, app: &mut EqEditorApp, band_index: usize) {
    let band = &app.params.bands[band_index];
    let selected = app.selected_band == Some(band_index);
    let header_color = if selected {
        theme::ACCENT
    } else {
        theme::TEXT_DIM
    };

    egui::Frame::group(ui.style())
        .fill(if selected {
            theme::PANEL_LIGHT
        } else {
            theme::PANEL
        })
        .stroke(egui::Stroke::new(
            1.0,
            if selected {
                theme::ACCENT
            } else {
                theme::BORDER
            },
        ))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            // The band strip lays out columns horizontally, so `Frame::show`
            // inherits a horizontal parent layout. Everything inside a band
            // cell needs to stack vertically, hence the explicit wrap.
            ui.vertical(|ui| {
                ui.set_min_width(104.0);
                ui.set_max_width(104.0);

                // Header row — index + enable toggle.
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("B{}", band_index + 1))
                            .strong()
                            .color(header_color),
                    );
                    let mut enabled = band.enabled.value();
                    if ui.checkbox(&mut enabled, "").changed() {
                        band.enabled.set_value(enabled);
                    }
                });

                ui.add_space(2.0);

                // Kind dropdown.
                let mut kind = BandKind::from_index(band.kind.value());
                egui::ComboBox::from_id_salt(("eq_band_kind", band_index))
                    .width(92.0)
                    .selected_text(kind.short_name())
                    .show_ui(ui, |ui| {
                        for opt in BandKind::ALL {
                            if ui.selectable_label(kind == opt, opt.short_name()).clicked() {
                                kind = opt;
                                band.kind.set_value(kind.to_index());
                            }
                        }
                    });

                // Slope dropdown (only meaningful on cuts).
                if kind.is_cut() {
                    let mut slope = BandSlope::from_index(band.slope.value());
                    egui::ComboBox::from_id_salt(("eq_band_slope", band_index))
                        .width(92.0)
                        .selected_text(slope.label())
                        .show_ui(ui, |ui| {
                            for opt in [BandSlope::Db12, BandSlope::Db24, BandSlope::Db48] {
                                if ui.selectable_label(slope == opt, opt.label()).clicked() {
                                    slope = opt;
                                    band.slope.set_value(slope.to_index());
                                }
                            }
                        });
                } else {
                    // Keep columns the same height whether or not the slope
                    // row is rendered, so all bands line up.
                    ui.add_space(22.0);
                }

                // Stereo / Mid / Side: which component the band filters.
                let mut ms = BandMs::from_index(band.ms.value());
                egui::ComboBox::from_id_salt(("eq_band_ms", band_index))
                    .width(92.0)
                    .selected_text(ms.label())
                    .show_ui(ui, |ui| {
                        for opt in BandMs::ALL {
                            if ui.selectable_label(ms == opt, opt.label()).clicked() {
                                ms = opt;
                                band.ms.set_value(ms.to_index());
                            }
                        }
                    });

                ui.add_space(4.0);

                // Freq.
                // The readout follows the value the drag just produced,
                // not last frame's — `egui::Slider` wrote through a
                // `&mut`, so it read fresh in the same frame.
                let mut freq = band.freq.value();
                if let Some(travel) = band_slider(ui, log_travel(FREQ_HZ, freq), false) {
                    freq = log_value(FREQ_HZ, travel);
                    band.freq.set_value(freq);
                }
                ui.label(egui::RichText::new(format_hz_short(freq)).color(theme::TEXT_DIM));

                // Gain (only meaningful for bell/shelf). Bipolar: the
                // fill runs out from 0 dB rather than up from -24.
                if kind.uses_gain() {
                    let mut gain = band.gain.value();
                    if let Some(travel) = band_slider(ui, (gain + GAIN_DB) / (GAIN_DB * 2.0), true)
                    {
                        gain = travel * GAIN_DB * 2.0 - GAIN_DB;
                        band.gain.set_value(gain);
                    }
                    ui.label(
                        egui::RichText::new(format!("{:+.1} dB", gain)).color(theme::TEXT_DIM),
                    );
                } else {
                    // Keep vertical alignment with bell/shelf bands.
                    ui.add_space(22.0);
                    ui.label(egui::RichText::new(" ").color(theme::TEXT_DIM));
                }

                // Q — the one-knob kinds (Tilt, LF Lift+Dip, Air) fix
                // their own shape, so they show no Q.
                if kind.uses_q() {
                    let mut q = band.q.value();
                    if let Some(travel) = band_slider(ui, log_travel(Q, q), false) {
                        q = log_value(Q, travel);
                        band.q.set_value(q);
                    }
                    ui.label(egui::RichText::new(format!("Q {:.2}", q)).color(theme::TEXT_DIM));
                } else {
                    ui.add_space(22.0);
                    ui.label(egui::RichText::new(" ").color(theme::TEXT_DIM));
                }

                // Dynamics: a switch, then threshold / ratio / attack /
                // release. Always laid out so the columns line up; greyed
                // while off, and unavailable on the kinds that take no
                // dynamics (the cuts, Tilt, LF Lift+Dip).
                ui.add_space(4.0);
                ui.add_enabled_ui(kind.supports_dyn(), |ui| {
                    let mut on = band.dyn_on.value();
                    if ui.checkbox(&mut on, "Dynamic").changed() {
                        band.dyn_on.set_value(on);
                    }
                    ui.add_enabled_ui(on, |ui| {
                        param_slider(ui, &band.dyn_threshold, "Thr");
                        param_slider(ui, &band.dyn_ratio, "Ratio");
                        param_slider(ui, &band.dyn_attack, "Att");
                        param_slider(ui, &band.dyn_release, "Rel");
                    });
                });
            });
        });
}

/// A band slider bound straight to a `FloatParam`: travel is the param's
/// own normalized value (its declared skew), and the readout is its own
/// formatter, captioned with `caption`.
fn param_slider(ui: &mut egui::Ui, param: &FloatParam, caption: &str) {
    if let Some(travel) = band_slider(ui, param.normalized_value(), false) {
        param.set_normalized(travel);
    }
    let text = format!("{caption} {}", param.display(param.value() as f64));
    ui.label(egui::RichText::new(text).color(theme::TEXT_DIM));
}

/// One band slider: the shared geometry and palette. Returns the new
/// unit travel while it is being positioned.
fn band_slider(ui: &mut egui::Ui, travel: f32, bipolar: bool) -> Option<f32> {
    let s = HSlider::new(SLIDER_W, travel)
        .bipolar(bipolar)
        .style(SliderStyle::LAVENDER);
    slider(ui, &s)
}

/// Declared range of the Freq slider, Hz.
const FREQ_HZ: (f32, f32) = (20.0, 20_000.0);
/// Declared range of the Q slider.
const Q: (f32, f32) = (0.1, 10.0);
/// Half-range of the Gain slider, dB (it runs `-GAIN_DB..=GAIN_DB`).
const GAIN_DB: f32 = 24.0;

/// Slider travel of `value` on a logarithmic `min..max` range.
///
/// This is `egui::Slider::logarithmic(true)` for an all-positive range,
/// which is what Freq and Q were before they moved onto the shared
/// slider: even travel per decade, so the first third of the Freq
/// groove is still 20–200 Hz.
fn log_travel((min, max): (f32, f32), value: f32) -> f32 {
    let span = max.ln() - min.ln();
    ((value.clamp(min, max).ln() - min.ln()) / span).clamp(0.0, 1.0)
}

/// The value a travel maps back to on a logarithmic range.
fn log_value((min, max): (f32, f32), travel: f32) -> f32 {
    let span = max.ln() - min.ln();
    (min.ln() + travel.clamp(0.0, 1.0) * span).exp().clamp(min, max)
}

fn format_hz_short(freq: f32) -> String {
    if freq >= 1000.0 {
        format!("{:.2} kHz", freq / 1000.0)
    } else {
        format!("{:.0} Hz", freq)
    }
}
