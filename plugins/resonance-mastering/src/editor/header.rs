//! The window header: whole-plugin bypass, the loudness reference line,
//! and the at-a-glance readouts (integrated LUFS, glue GR, limiter GR).
//!
//! Two of these are controls, not readouts:
//!
//! * **Bypass** engages the chain's latency-matched dry path
//!   ([`crate::chain::Chain::process`]), so A/B-ing the master
//!   chain stays sample-aligned. The DSP has always honoured the
//!   `bypass` param — until now nothing in the window could set it.
//! * **Ref line** is the `target_lufs` param, the loudness target the
//!   LUFS meter, the LUFS history trace and this header all draw. The
//!   assistant writes it when the user clicks "Apply suggestions"; it
//!   is otherwise the user's to move, and nothing overwrites it behind
//!   their back.
//!
//! [`reference_line_lufs`] is the single read path for that line, so the
//! three places that draw it cannot drift apart.

use std::ops::RangeInclusive;

use plugin_gui_core::egui;

use crate::params::MasteringParams;
use crate::viz::MasteringViz;

use super::theme;

/// The loudness reference line, in LUFS. Every surface that draws the
/// line reads it from here.
pub fn reference_line_lufs(params: &MasteringParams) -> f32 {
    params.target_lufs.value()
}

/// Move the reference line, clamped to the param's declared range so a
/// drag past either end cannot push the DSP outside what it expects.
pub fn set_reference_line_lufs(params: &MasteringParams, lufs: f32) {
    let (min, max) = reference_line_range(params);
    params.target_lufs.set_value(lufs.clamp(min, max));
}

/// Inclusive `(min, max)` of the reference line, taken from the param.
pub fn reference_line_range(params: &MasteringParams) -> (f32, f32) {
    (
        params.target_lufs.range().min(),
        params.target_lufs.range().max(),
    )
}

/// Flip whole-plugin bypass; returns the new state.
pub fn toggle_bypass(params: &MasteringParams) -> bool {
    let next = !params.bypass.value();
    params.bypass.set_value(next);
    next
}

/// How the bypass button reads. Engaged is a filled warning-coloured
/// pill so a bypassed master chain is impossible to miss (and to
/// mistake for a dead control).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BypassLook {
    pub label: &'static str,
    pub fill: egui::Color32,
    pub text: egui::Color32,
}

pub fn bypass_look(bypassed: bool) -> BypassLook {
    if bypassed {
        BypassLook {
            label: "BYPASSED",
            fill: theme::WARN,
            text: theme::BG,
        }
    } else {
        BypassLook {
            label: "BYPASS",
            fill: theme::PANEL_LIGHT,
            text: theme::TEXT_DIM,
        }
    }
}

/// Everything the header shows, resolved from the params and the latest
/// viz snapshot in one place.
#[derive(Debug, Clone, PartialEq)]
pub struct HeaderModel {
    pub bypassed: bool,
    pub target_lufs: f32,
    pub target_range: RangeInclusive<f32>,
    /// `None` while the integrated measurement is still gating up.
    pub integrated_lufs: Option<f32>,
    pub glue_gr_db: f32,
    pub limiter_gr_db: f32,
}

impl HeaderModel {
    pub fn new(params: &MasteringParams, viz: &MasteringViz) -> Self {
        let snap = viz.load_snapshot();
        let (min, max) = reference_line_range(params);
        Self {
            bypassed: params.bypass.value(),
            target_lufs: reference_line_lufs(params),
            target_range: min..=max,
            integrated_lufs: snap
                .integrated_lufs
                .is_finite()
                .then_some(snap.integrated_lufs),
            glue_gr_db: viz.glue_gr_db(),
            limiter_gr_db: viz.limiter_gr_db(),
        }
    }

    pub fn integrated_text(&self) -> String {
        match self.integrated_lufs {
            Some(lufs) => format!("Integrated: {lufs:>5.1} LUFS"),
            None => "Integrated: —".to_string(),
        }
    }
}

// --- drawing ------------------------------------------------------------

pub(crate) fn draw(
    ui: &mut egui::Ui,
    params: &MasteringParams,
    viz: &MasteringViz,
    preset_editor: &mut resonance_plugin::presets::PresetEditor,
    bank: &resonance_plugin::presets::PresetBank,
    session: &resonance_plugin::presets::PresetSession,
) {
    let model = HeaderModel::new(params, viz);

    ui.horizontal_centered(|ui| {
        ui.add_space(12.0);
        ui.label(
            egui::RichText::new("RESONANCE MASTERING")
                .strong()
                .color(theme::ACCENT),
        );
        ui.add_space(12.0);

        // Whole-plugin bypass. The dry path is delay-matched, so this
        // is a level- and time-aligned A/B of the master chain.
        let look = bypass_look(model.bypassed);
        let button = egui::Button::new(
            egui::RichText::new(look.label)
                .strong()
                .size(11.0)
                .color(look.text),
        )
        .fill(look.fill)
        .stroke(egui::Stroke::new(1.0, theme::BORDER));
        if ui
            .add(button)
            .on_hover_text(
                "Bypass the whole mastering chain. The dry path is \
                 latency-matched, so the A/B stays sample-aligned.",
            )
            .clicked()
        {
            toggle_bypass(params);
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        // Presets. This plugin ships no factory bank, so until ba todo
        // #1332 a mastering chain a user had dialled in could not be kept
        // at all outside the one project it lived in.
        let refs: Vec<&dyn resonance_plugin::Param> = (0..crate::params::PARAM_COUNT)
            .map(|i| params.param_at(i))
            .collect();
        resonance_plugin::preset_ui::preset_bar(
            ui,
            "mastering_preset",
            preset_editor,
            bank,
            session,
            &refs,
            "— preset —",
        );

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        // The loudness reference line — drag or click to type.
        ui.label(egui::RichText::new("Ref line").color(theme::TEXT_DIM));
        let mut target = model.target_lufs;
        let drag = egui::DragValue::new(&mut target)
            .speed(0.1)
            .range(model.target_range.clone())
            .fixed_decimals(1)
            .suffix(" LUFS");
        if ui
            .add(drag)
            .on_hover_text(
                "Loudness target drawn on the LUFS meter and the history \
                 trace. The assistant sets it when you apply its \
                 suggestions; otherwise it stays where you put it.",
            )
            .changed()
        {
            set_reference_line_lufs(params, target);
        }

        ui.separator();
        ui.label(egui::RichText::new(model.integrated_text()).color(theme::TEXT));

        ui.separator();
        let glue_gr = model.glue_gr_db;
        ui.label(
            egui::RichText::new(format!("Glue GR: {glue_gr:>4.1} dB")).color(if glue_gr > 0.5 {
                theme::ACCENT
            } else {
                theme::TEXT_DIM
            }),
        );
        ui.separator();
        let lim_gr = model.limiter_gr_db;
        ui.label(
            egui::RichText::new(format!("Lim GR: {lim_gr:>4.1} dB")).color(if lim_gr > 0.5 {
                theme::WARN
            } else {
                theme::TEXT_DIM
            }),
        );
    });
}
