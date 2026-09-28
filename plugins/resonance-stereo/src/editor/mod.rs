//! Stereo plugin editor: an egui UI hosted by the platform runtime.
//!
//! Layout: a header band (name, preset bar, the mono-risk pill), then the
//! body — the goniometer and correlation strip on the left, and the
//! parameter groups on the right, in signal order. The groups are the
//! data-driven table [`GROUPS`], which `tests/editor_layout.rs` holds to
//! the declared parameter surface: every parameter is drawn exactly once
//! (the dual-surface rule — each CLAP param is also in the GUI).

mod factory;
pub mod scope;
pub mod theme;
mod widgets;

pub use factory::StereoEditorFactory;

use std::sync::Arc;

use plugin_gui_core::{egui, EditorApp};
use resonance_plugin::preset_ui::preset_bar;
use resonance_plugin::presets::{PresetBank, PresetEditor, PresetSession};
use resonance_plugin::Param;

use crate::params::{index, ParamRef, StereoParams, PARAM_COUNT};
use crate::viz::StereoViz;

/// Height of the title band, in points (the fleet's 46 px band).
pub const HEADER_H: f32 = 46.0;

/// One captioned group of controls.
pub struct ControlGroup {
    pub caption: &'static str,
    /// Indices into [`StereoParams::param_at`].
    pub params: &'static [usize],
}

/// The groups, in the order the signal meets them.
pub const GROUPS: &[ControlGroup] = &[
    ControlGroup {
        caption: "WIDEN",
        params: &[
            index::WIDEN_MODE,
            index::WIDEN_AMOUNT,
            index::FOCUS_LOW,
            index::FOCUS_HIGH,
        ],
    },
    ControlGroup {
        caption: "IMAGE",
        params: &[index::WIDTH, index::BALANCE, index::ROTATION],
    },
    ControlGroup {
        caption: "MONO BASS",
        params: &[index::MONO_BELOW, index::MONO_SLOPE],
    },
    ControlGroup {
        caption: "AUDITION",
        params: &[index::SOLO_SIDE, index::MONO_CHECK],
    },
];

/// Which widget a parameter gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlKind {
    /// A rotary knob bound to a `FloatParam`.
    Knob,
    /// A segmented selector over a choice `IntParam`'s labels.
    Choice(&'static [&'static str]),
    /// A checkbox bound to a `BoolParam`.
    Toggle,
}

/// The widget for parameter `index`, decided by its declared type.
pub fn control_kind(params: &StereoParams, index: usize) -> ControlKind {
    match params.param_ref(index) {
        ParamRef::Float(_) => ControlKind::Knob,
        ParamRef::Int(p) => ControlKind::Choice(p.choices().unwrap_or(&[])),
        ParamRef::Bool(_) => ControlKind::Toggle,
    }
}

pub(crate) struct StereoEditorApp {
    pub(crate) params: Arc<StereoParams>,
    pub(crate) viz: Arc<StereoViz>,
    pub(crate) bank: PresetBank,
    pub(crate) presets: Arc<PresetSession>,
    pub(crate) preset_editor: PresetEditor,
    /// Goniometer points, reused across frames so the scope draws
    /// without allocating.
    pub(crate) scratch: Vec<(f32, f32)>,
}

impl StereoEditorApp {
    pub fn new(params: Arc<StereoParams>, viz: Arc<StereoViz>, presets: Arc<PresetSession>) -> Self {
        Self {
            params,
            viz,
            bank: PresetBank::new(
                <crate::ResonanceStereo as resonance_plugin::ResonancePlugin>::CLAP_ID,
                <crate::ResonanceStereo as resonance_plugin::ResonancePlugin>::FACTORY_PRESETS,
            ),
            presets,
            preset_editor: PresetEditor::default(),
            scratch: Vec::with_capacity(crate::viz::GONIO_POINTS),
        }
    }
}

impl EditorApp for StereoEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));

        egui::Panel::top("stereo_header")
            .exact_size(HEADER_H)
            .show_inside(ui, |ui| draw_header(ui, self));

        egui::CentralPanel::default().show_inside(ui, |ui| draw_body(ui, self));
    }
}

fn draw_header(ui: &mut egui::Ui, app: &mut StereoEditorApp) {
    ui.horizontal_centered(|ui| {
        ui.add_space(12.0);
        ui.label(
            egui::RichText::new("RESONANCE STEREO")
                .strong()
                .color(theme::ACCENT),
        );
        ui.add_space(14.0);
        ui.separator();
        ui.add_space(8.0);

        ui.label(egui::RichText::new("Preset").color(theme::TEXT_3));
        let params: Vec<&dyn Param> = (0..PARAM_COUNT).map(|i| app.params.param_at(i)).collect();
        preset_bar(
            ui,
            "stereo_preset",
            &mut app.preset_editor,
            &app.bank,
            &app.presets,
            &params,
            "— preset —",
        );

        ui.add_space(14.0);
        ui.separator();
        ui.add_space(8.0);
        // What the fold does with the current mode, in words. Haas (and
        // Micro-shift past amount 0.5) put deep combs into mono; say so
        // where the user looks, not only in the mode's label.
        let mode = app.params.widen_mode();
        let amount = app.params.widen_amount.value();
        let (dot, color, text) = if mode.is_mono_risk(amount) {
            ("\u{25cf}", theme::BAD, "MONO RISK")
        } else if mode.preserves_mono_sum() {
            ("\u{25cf}", theme::GOOD, "MONO SUM KEPT")
        } else {
            ("\u{25cf}", theme::WARM, "MONO RIPPLE")
        };
        ui.label(egui::RichText::new(dot).color(color).size(9.0));
        ui.label(egui::RichText::new(text).color(color).strong())
            .on_hover_text(mode.amount_hint(amount));
    });
}

fn draw_body(ui: &mut egui::Ui, app: &mut StereoEditorApp) {
    let avail = ui.available_rect_before_wrap();
    let scope_w = (avail.height() - 8.0).min(avail.width() * 0.4).max(120.0);
    let scope_rect = egui::Rect::from_min_size(avail.min, egui::vec2(scope_w, avail.height()));
    scope::draw(ui, scope_rect, &app.viz, &mut app.scratch);

    let controls_rect = egui::Rect::from_min_max(
        egui::pos2(scope_rect.max.x + 10.0, avail.min.y),
        avail.max,
    );
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(controls_rect));
    draw_controls(&mut child, app);
}

fn draw_controls(ui: &mut egui::Ui, app: &StereoEditorApp) {
    let params = &app.params;
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        for group in GROUPS {
            draw_group(ui, params, group);
            ui.add_space(6.0);
        }
    });
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(params.widen_mode().amount_hint(params.widen_amount.value()))
            .color(theme::TEXT_3)
            .size(11.0),
    );
}

fn draw_group(ui: &mut egui::Ui, params: &StereoParams, group: &ControlGroup) {
    let frame = egui::Frame::default()
        .fill(theme::BG_2)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(theme::RADIUS_PANEL)
        .inner_margin(egui::Margin::symmetric(10, 8));
    frame.show(ui, |ui| {
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(group.caption)
                    .color(theme::TEXT_3)
                    .size(10.5)
                    .strong(),
            );
            ui.add_space(4.0);
            // Choices get their own row (a segmented selector is wide);
            // knobs and toggles share one.
            for &i in group.params {
                if matches!(control_kind(params, i), ControlKind::Choice(_)) {
                    widgets::param_control(ui, params, i);
                }
            }
            ui.horizontal(|ui| {
                for &i in group.params {
                    if !matches!(control_kind(params, i), ControlKind::Choice(_)) {
                        widgets::param_control(ui, params, i);
                    }
                }
            });
        });
    });
}
