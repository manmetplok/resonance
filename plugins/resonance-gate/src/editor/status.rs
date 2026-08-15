//! The header's detector readout (ba todo #1314, finding C3).
//!
//! A gate keyed from its own input and a gate keyed from a sidechain
//! that happens to be silent look identical from the outside, so the
//! one thing this editor has to say out loud is *which detector is
//! running* — and, since the detector is the whole plugin, what that
//! detector is currently doing.
//!
//! [`DetectorSummary`] is the editor-facing struct: a pure snapshot of
//! [`GateViz`] with every string already resolved, so what the header
//! shows can be asserted in `tests/viz.rs` without a window.

use wayland_plugin_gui::egui;

use crate::dsp::GateState;
use crate::viz::{DetectorSource, GateViz};

use super::theme;

/// Everything the header says about the detector, as plain data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectorSummary {
    /// Which signal the detector is reading.
    pub source: DetectorSource,
    /// `"EXTERNAL KEY"` / `"INPUT"`.
    pub source_label: &'static str,
    /// Whether the gate is open, holding, or closed.
    pub state: GateState,
    /// `"OPEN"` / `"HOLD"` / `"CLOSED"`.
    pub state_label: &'static str,
    /// Peak detector level of the last block, e.g. `"-18.4 dB"`, or an
    /// em dash while the detector sees nothing at all.
    pub detector_text: String,
    /// Peak gain reduction of the last block as a negative gain, e.g.
    /// `"-12.0 dB"`.
    pub gr_text: String,
}

impl DetectorSummary {
    pub fn from_viz(viz: &GateViz) -> Self {
        let source = viz.detector_source();
        let state = viz.state();
        let detector_db = viz.detector_db();
        let gr_db = viz.gr_db();
        Self {
            source,
            source_label: source.label(),
            state,
            state_label: state.label(),
            detector_text: if detector_db.is_finite() {
                format!("{detector_db:.1} dB")
            } else {
                "—".to_string()
            },
            // Round the last sliver of the opening ramp down to a flat
            // zero: a readout of "-0.0 dB" is not a reduction, it is a
            // rendering artefact.
            gr_text: {
                let gr = gr_db.max(0.0);
                format!("{:.1} dB", if gr < 0.05 { 0.0 } else { -gr })
            },
        }
    }
}

/// Draw the right-hand side of the header: detector source, gate state,
/// and the two live numbers behind them.
pub fn draw(ui: &mut egui::Ui, summary: &DetectorSummary) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.add_space(12.0);
        chip(
            ui,
            summary.state_label,
            match summary.state {
                GateState::Open => Tone::Good,
                GateState::Holding => Tone::Warm,
                GateState::Closed => Tone::Dim,
            },
        );
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(format!("GR {}", summary.gr_text))
                .monospace()
                .size(10.0)
                .color(theme::TEXT_2),
        );
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(format!("DET {}", summary.detector_text))
                .monospace()
                .size(10.0)
                .color(theme::TEXT_2),
        );
        ui.add_space(8.0);
        chip(
            ui,
            &format!("KEY · {}", summary.source_label),
            match summary.source {
                DetectorSource::ExternalKey => Tone::Accent,
                DetectorSource::SelfInput => Tone::Dim,
            },
        );
    });
}

/// Tone of a non-interactive status pill.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tone {
    /// Idle / not engaged.
    Dim,
    /// Lit lavender: an external key is driving the detector.
    Accent,
    /// Lit mint: signal is passing.
    Good,
    /// Lit amber: the hold timer is the only reason signal is passing.
    Warm,
}

/// Small status pill, display only — the same shape and type scale as
/// the granular delay's header chips.
fn chip(ui: &mut egui::Ui, label: &str, tone: Tone) {
    let font = egui::FontId::monospace(9.0);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), font.clone(), theme::TEXT_1);
    let size = egui::vec2(galley.size().x + 14.0, 18.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter_at(rect);
    let (fill, stroke, text) = match tone {
        Tone::Dim => (theme::BG_1, theme::LINE_2, theme::TEXT_3),
        Tone::Accent => (
            theme::ACCENT_DIM,
            theme::ACCENT.gamma_multiply(0.6),
            theme::ACCENT_SOFT,
        ),
        Tone::Good => (
            theme::GOOD.gamma_multiply(0.12),
            theme::GOOD.gamma_multiply(0.5),
            theme::GOOD,
        ),
        Tone::Warm => (
            theme::WARM.gamma_multiply(0.12),
            theme::WARM.gamma_multiply(0.55),
            theme::WARM,
        ),
    };
    painter.rect_filled(rect, theme::RADIUS_CHIP, fill);
    painter.rect_stroke(
        rect,
        theme::RADIUS_CHIP,
        egui::Stroke::new(1.0, stroke),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        font,
        text,
    );
}
