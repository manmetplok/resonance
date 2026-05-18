//! Per-band control strip drawn at the bottom of the EQ editor.

use wayland_plugin_gui::egui;

use crate::band::{BandKind, BandSlope};
use crate::params::NUM_BANDS;

use super::app::EqEditorApp;
use super::theme;

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
                ui.spacing_mut().slider_width = 92.0;

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
                        for opt in [
                            BandKind::Bell,
                            BandKind::LowShelf,
                            BandKind::HighShelf,
                            BandKind::LowCut,
                            BandKind::HighCut,
                        ] {
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

                ui.add_space(4.0);

                // Freq.
                let mut freq = band.freq.value();
                if ui
                    .add(
                        egui::Slider::new(&mut freq, 20.0..=20_000.0)
                            .logarithmic(true)
                            .show_value(false),
                    )
                    .changed()
                {
                    band.freq.set_value(freq);
                }
                ui.label(egui::RichText::new(format_hz_short(freq)).color(theme::TEXT_DIM));

                // Gain (only meaningful for bell/shelf).
                if kind.uses_gain() {
                    let mut gain = band.gain.value();
                    if ui
                        .add(
                            egui::Slider::new(&mut gain, -24.0..=24.0)
                                .fixed_decimals(1)
                                .show_value(false),
                        )
                        .changed()
                    {
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

                // Q.
                let mut q = band.q.value();
                if ui
                    .add(
                        egui::Slider::new(&mut q, 0.1..=10.0)
                            .logarithmic(true)
                            .show_value(false),
                    )
                    .changed()
                {
                    band.q.set_value(q);
                }
                ui.label(egui::RichText::new(format!("Q {:.2}", q)).color(theme::TEXT_DIM));
            });
        });
}

fn format_hz_short(freq: f32) -> String {
    if freq >= 1000.0 {
        format!("{:.2} kHz", freq / 1000.0)
    } else {
        format!("{:.0} Hz", freq)
    }
}
