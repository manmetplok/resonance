//! Master Assistant control panel.
//!
//! Renders:
//! - A target-source toggle (genre vs loaded reference track)
//! - Genre dropdown or reference-track path input depending on mode
//! - A capture-fill progress bar
//! - Analyze / Clear buttons
//! - Results block (analysis stats) and rationale list
//! - An Apply button that commits the suggested params
//!
//! The target choice (mode, genre, reference path) lives in the
//! [`Assistant`], not in the editor, so it is saved with the plugin state
//! and survives closing the window and reopening the project.

use plugin_gui_core::egui;

use crate::assistant::{Assistant, Genre, TargetMode};
use crate::params::MasteringParams;

use super::theme;

pub fn draw(ui: &mut egui::Ui, params: &MasteringParams, assistant: &Assistant) {
    let settings = assistant.settings();
    ui.vertical(|ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new("Master Assistant")
                    .strong()
                    .size(14.0)
                    .color(theme::ACCENT),
            );
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("Pick a target, let ~10 s of audio play, then Analyze.")
                    .size(10.0)
                    .color(theme::TEXT_DIM),
            );
        });
        ui.add_space(6.0);

        // Global input trim — applied before the chain. Lives on the
        // Assistant tab because that's where users normally start a
        // session.
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(egui::RichText::new("Input trim:").color(theme::TEXT_DIM));
            let mut trim = params.input_trim_db.value();
            if ui
                .add(
                    egui::Slider::new(&mut trim, -24.0..=24.0)
                        .fixed_decimals(1)
                        .suffix(" dB"),
                )
                .changed()
            {
                params.input_trim_db.set_value(trim);
            }
        });
        ui.add_space(6.0);

        // Target-source toggle
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(egui::RichText::new("Target:").color(theme::TEXT_DIM));
            if ui
                .selectable_label(settings.mode == TargetMode::Genre, "Genre preset")
                .clicked()
            {
                assistant.set_mode(TargetMode::Genre);
            }
            ui.add_space(4.0);
            if ui
                .selectable_label(settings.mode == TargetMode::Reference, "Reference track")
                .clicked()
            {
                assistant.set_mode(TargetMode::Reference);
            }
        });
        ui.add_space(6.0);

        // Target input: genre dropdown OR reference file row
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            match settings.mode {
                TargetMode::Genre => {
                    ui.label(egui::RichText::new("Genre:").color(theme::TEXT_DIM));
                    egui::ComboBox::from_id_salt("assistant_genre")
                        .width(120.0)
                        .selected_text(settings.genre.label())
                        .show_ui(ui, |ui| {
                            for &g in Genre::ALL {
                                if ui
                                    .selectable_label(settings.genre == g, g.label())
                                    .clicked()
                                {
                                    assistant.set_genre(g);
                                }
                            }
                        });
                }
                TargetMode::Reference => {
                    ui.label(egui::RichText::new("File:").color(theme::TEXT_DIM));
                    let mut reference_path = settings.reference_path.clone();
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut reference_path)
                                .desired_width(300.0)
                                .hint_text("/path/to/reference.wav"),
                        )
                        .changed()
                    {
                        assistant.set_reference_path(&reference_path);
                    }
                    if ui.button("Load").clicked() {
                        let _ = assistant.load_reference(&reference_path);
                    }
                    if ui.button("Clear").clicked() {
                        assistant.clear_reference();
                    }

                    // Current reference status
                    if let Some(r) = assistant.reference() {
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "[{}  {:.1} LUFS]",
                                r.display_name, r.analysis.integrated_lufs
                            ))
                            .size(11.0)
                            .color(theme::GOOD),
                        );
                    } else if let Some(err) = assistant.reference_error() {
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new(format!("[error: {}]", err))
                                .size(11.0)
                                .color(theme::DANGER),
                        );
                    }
                }
            }
        });
        ui.add_space(6.0);

        // Capture + action row
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            let fraction = assistant.capture_fraction();
            ui.label(egui::RichText::new("Captured:").color(theme::TEXT_DIM));
            ui.add(
                egui::ProgressBar::new(fraction)
                    .desired_width(160.0)
                    .text(format!("{:.0}%", fraction * 100.0)),
            );

            ui.add_space(24.0);
            let can_analyze = fraction > 0.2
                && (settings.mode == TargetMode::Genre || assistant.reference().is_some());
            if ui
                .add_enabled(can_analyze, egui::Button::new("Analyze"))
                .clicked()
            {
                let _ = assistant.analyze(assistant.current_target());
            }

            ui.add_space(8.0);
            if ui.button("Clear capture").clicked() {
                assistant.clear();
            }
        });
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(4.0);

        // Results block
        if let Some(analysis) = assistant.last_analysis() {
            let suggestions = assistant.last_suggestions();
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.set_min_width(280.0);
                    ui.set_max_width(280.0);
                    ui.label(egui::RichText::new("Analysis").strong().color(theme::TEXT));
                    ui.add_space(2.0);
                    stat_row(ui, "Duration", format!("{:.1} s", analysis.duration_s));
                    stat_row(
                        ui,
                        "Integrated",
                        format!("{:.1} LUFS", analysis.integrated_lufs),
                    );
                    stat_row(
                        ui,
                        "True peak",
                        format!("{:.1} dBTP", analysis.true_peak_dbtp),
                    );
                    stat_row(ui, "Crest", format!("{:.1} dB", analysis.crest_db));
                    stat_row(ui, "Correlation", format!("{:+.2}", analysis.correlation));
                });

                ui.add_space(16.0);
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new("Suggestions")
                            .strong()
                            .color(theme::TEXT),
                    );
                    ui.add_space(2.0);
                    if let Some(s) = &suggestions {
                        ui.label(
                            egui::RichText::new(format!("Target: {}", s.target_label))
                                .size(11.0)
                                .color(theme::TEXT_DIM),
                        );
                        ui.add_space(2.0);
                        for line in &s.rationale {
                            ui.label(
                                egui::RichText::new(format!("• {line}"))
                                    .size(11.0)
                                    .color(theme::TEXT),
                            );
                        }
                        ui.add_space(6.0);
                        if ui
                            .add(egui::Button::new(
                                egui::RichText::new("Apply suggestions").color(theme::ACCENT),
                            ))
                            .clicked()
                        {
                            s.apply_to(params);
                        }
                    } else {
                        ui.label(
                            egui::RichText::new("No suggestions yet.")
                                .size(11.0)
                                .color(theme::TEXT_DIM),
                        );
                    }
                });
            });
        } else {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new(
                        "Click Analyze once a representative section of the mix \
                         has played through.",
                    )
                    .size(11.0)
                    .color(theme::TEXT_DIM),
                );
            });
        }
    });
}

fn stat_row(ui: &mut egui::Ui, label: &str, value: String) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("{label}:"))
                .size(11.0)
                .color(theme::TEXT_DIM),
        );
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(value)
                .size(11.0)
                .monospace()
                .color(theme::TEXT),
        );
    });
}
