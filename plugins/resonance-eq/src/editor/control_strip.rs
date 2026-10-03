//! Per-band control strip drawn at the bottom of the EQ editor.
//!
//! Freq / Gain / Q and the dynamics sliders are the shared kit's slider
//! bound through `resonance_plugin::editor_widgets` (code review PUX-01,
//! PUX-06): travel is each param's own normalized value — the
//! `FloatRange::Skewed` curve `params.rs` declares, not a restated
//! true-log range — a double-click resets to the declared default, the
//! readout under each slider takes a typed value, and every gesture is
//! announced to the host as one undoable edit. The switches and the
//! kind / slope / M-S picks announce too.

use plugin_gui_core::egui;
use resonance_plugin::editor_widgets::{self, ParamSlider};
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
                    editor_widgets::bool_checkbox(ui, &band.enabled, "");
                });

                ui.add_space(2.0);

                // Kind dropdown.
                let kind = BandKind::from_index(band.kind.value());
                egui::ComboBox::from_id_salt(("eq_band_kind", band_index))
                    .width(92.0)
                    .selected_text(kind.short_name())
                    .show_ui(ui, |ui| {
                        for opt in BandKind::ALL {
                            if ui.selectable_label(kind == opt, opt.short_name()).clicked() {
                                let v = f64::from(opt.to_index());
                                editor_widgets::commit_plain(ui.ctx(), &band.kind, v);
                            }
                        }
                    });

                // Slope dropdown (only meaningful on cuts).
                if kind.is_cut() {
                    let slope = BandSlope::from_index(band.slope.value());
                    egui::ComboBox::from_id_salt(("eq_band_slope", band_index))
                        .width(92.0)
                        .selected_text(slope.label())
                        .show_ui(ui, |ui| {
                            for opt in [BandSlope::Db12, BandSlope::Db24, BandSlope::Db48] {
                                if ui.selectable_label(slope == opt, opt.label()).clicked() {
                                    let v = f64::from(opt.to_index());
                                    editor_widgets::commit_plain(ui.ctx(), &band.slope, v);
                                }
                            }
                        });
                } else {
                    // Keep columns the same height whether or not the slope
                    // row is rendered, so all bands line up.
                    ui.add_space(22.0);
                }

                // Stereo / Mid / Side: which component the band filters.
                let ms = BandMs::from_index(band.ms.value());
                egui::ComboBox::from_id_salt(("eq_band_ms", band_index))
                    .width(92.0)
                    .selected_text(ms.label())
                    .show_ui(ui, |ui| {
                        for opt in BandMs::ALL {
                            if ui.selectable_label(ms == opt, opt.label()).clicked() {
                                let v = f64::from(opt.to_index());
                                editor_widgets::commit_plain(ui.ctx(), &band.ms, v);
                            }
                        }
                    });

                ui.add_space(4.0);

                band_slider(ui, &band.freq, None);

                // Gain (only meaningful for bell/shelf). Bipolar: the
                // fill runs out from 0 dB rather than up from -24 (read
                // off the range, which spans zero).
                if kind.uses_gain() {
                    band_slider(ui, &band.gain, None);
                } else {
                    // Keep vertical alignment with bell/shelf bands.
                    ui.add_space(22.0);
                    ui.label(egui::RichText::new(" ").color(theme::TEXT_DIM));
                }

                // Q — the one-knob kinds (Tilt, LF Lift+Dip, Air) fix
                // their own shape, so they show no Q.
                if kind.uses_q() {
                    band_slider(ui, &band.q, Some("Q"));
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
                    let on = band.dyn_on.value();
                    ui.horizontal(|ui| {
                        editor_widgets::bool_checkbox(ui, &band.dyn_on, "Dyn");
                        // Detect on the sidechain key instead of the band.
                        ui.add_enabled_ui(on, |ui| {
                            editor_widgets::bool_checkbox(ui, &band.dyn_sc, "Key");
                        })
                        .response
                        .on_hover_text("Detect on the sidechain key when one is connected");
                    });
                    ui.add_enabled_ui(on, |ui| {
                        band_slider(ui, &band.dyn_threshold, Some("Thr"));
                        band_slider(ui, &band.dyn_ratio, Some("Ratio"));
                        band_slider(ui, &band.dyn_attack, Some("Att"));
                        band_slider(ui, &band.dyn_release, Some("Rel"));
                    });
                });
            });
        });
}

/// A band slider bound to `param` — its own travel (declared skew),
/// default, polarity and formatter — with its readout underneath,
/// captioned with `caption` when given. Click the readout to type a
/// value.
fn band_slider(ui: &mut egui::Ui, param: &FloatParam, caption: Option<&str>) {
    editor_widgets::param_slider(ui, ParamSlider::new(param, SLIDER_W));
    // The readout follows the value the drag just produced, not last
    // frame's.
    let text = param.display(param.get_plain());
    let font = egui::TextStyle::Body.resolve(ui.style());
    let caption = caption.unwrap_or("");
    editor_widgets::param_readout(ui, param, caption, &text, SLIDER_W, font, theme::TEXT_DIM);
}
