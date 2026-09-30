//! The egui app: the three-band shell of the redesigned editor
//! (ba todo #1136, design doc #264) — a 46 px header, the central hero
//! buffer-view band (~400 px, `hero.rs`, ba todo #1139) and the 246 px
//! control strip.
//!
//! `GranularEditorApp` is the `EditorApp` the runtime drives each
//! frame. The header carries the title and live readouts (effective
//! delay, tempo + division, voice-lock, grain count, freeze), all from
//! the shared [`GranularViz`] atomics — the editor never touches the
//! DSP state.

use std::sync::Arc;

use resonance_plugin::preset_ui::preset_bar;
use resonance_plugin::presets::{PresetBank, PresetEditor, PresetSession};
use resonance_plugin::Param;
use plugin_gui_core::{egui, EditorApp};

use crate::params::GranularDelayParams;
use crate::sync::DIVISION_LABELS;
use crate::viz::GranularViz;

use super::{controls, hero, theme};

/// Band heights of the three-band layout (design doc #264).
const HEADER_H: f32 = 46.0;
const STRIP_H: f32 = 246.0;

pub(crate) struct GranularEditorApp {
    pub(crate) params: Arc<GranularDelayParams>,
    pub(crate) viz: Arc<GranularViz>,
    /// Factory bank + this plugin's user preset directory.
    pub(crate) bank: PresetBank,
    /// Shared with the plugin struct, so what the bar shows is what
    /// `save_state` persists. Replaces the local `selected_preset`
    /// index, which was display-only and did not survive the window
    /// closing.
    pub(crate) presets: Arc<PresetSession>,
    /// Transient bar state (open combo, in-progress rename), editor-only.
    pub(crate) preset_editor: PresetEditor,
    /// Hero canvas gesture in flight (ba todo #1144).
    hero_drag: Option<hero::HeroDrag>,
}

impl GranularEditorApp {
    pub fn new(
        params: Arc<GranularDelayParams>,
        viz: Arc<GranularViz>,
        presets: Arc<PresetSession>,
    ) -> Self {
        Self {
            params,
            viz,
            bank: PresetBank::for_plugin::<crate::ResonanceGranularDelay>(),
            presets,
            preset_editor: PresetEditor::default(),
            hero_drag: None,
        }
    }
}

impl EditorApp for GranularEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));

        egui::Panel::top("granular_header")
            .exact_size(HEADER_H)
            .show_inside(ui, |ui| draw_header(ui, self));

        egui::Panel::bottom("granular_strip")
            .exact_size(STRIP_H)
            .show_inside(ui, |ui| {
                // Signal-flow strip (ba todo #1141): fits the 246 px
                // band at the 1320 px window width.
                controls::draw(ui, &self.params, &self.viz);
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            // Hero buffer view (ba todo #1139) + direct manipulation
            // (ba todo #1144): tap drag = time, cloud drag = pitch,
            // scroll = density.
            let layout = hero::draw(ui, &self.params, &self.viz);
            hero::interact(ui, &self.params, &self.viz, &layout, &mut self.hero_drag);
        });
    }
}

/// Tone of a non-interactive header status pill.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ChipTone {
    /// Idle/inactive.
    Dim,
    /// Lit lavender (grain activity, division).
    Accent,
    /// Lit mint (voice lock).
    Good,
    /// Lit amber (freeze).
    Warm,
}

/// Small status pill (prototype header chips) — display only.
fn status_chip(ui: &mut egui::Ui, label: &str, tone: ChipTone) {
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
        ChipTone::Dim => (theme::BG_1, theme::LINE_2, theme::TEXT_3),
        ChipTone::Accent => (
            theme::ACCENT_DIM,
            theme::ACCENT.gamma_multiply(0.6),
            theme::ACCENT_SOFT,
        ),
        ChipTone::Good => (
            theme::GOOD.gamma_multiply(0.12),
            theme::GOOD.gamma_multiply(0.5),
            theme::GOOD,
        ),
        ChipTone::Warm => (
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

/// The 46 px header (ba todo #1140, design doc #264 req-6/req-8):
/// title → preset combo → live viz readouts → right-aligned
/// GRAINS/VOICE/FREEZE status chips. Pure function of params + viz
/// reads (plus the last-loaded preset name).
fn draw_header(ui: &mut egui::Ui, app: &mut GranularEditorApp) {
    ui.horizontal_centered(|ui| {
        ui.add_space(12.0);
        ui.label(
            egui::RichText::new("RESONANCE GRANULAR DELAY")
                .strong()
                .color(theme::ACCENT),
        );
        ui.add_space(14.0);
        ui.separator();
        ui.add_space(8.0);

        let params: Vec<&dyn Param> = (0..crate::params::PARAM_COUNT)
            .map(|i| app.params.param_at(i))
            .collect();
        preset_bar(
            ui,
            "granular_preset",
            &mut app.preset_editor,
            &app.bank,
            &app.presets,
            &params,
            "— preset —",
        );
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(8.0);

        // Live readouts from the viz atomics: BPM (when the host
        // provides one), the effective delay (Repitch glide / Fade
        // commit value, ba todo #1076) and the division chip while
        // synced.
        let bpm = app.viz.read_bpm();
        if bpm > 0.0 {
            ui.label(egui::RichText::new(format!("{bpm:.1} BPM")).color(theme::TEXT_1));
            ui.add_space(10.0);
        }
        let delay_ms = app.viz.read_delay_ms();
        ui.label(egui::RichText::new(format!("{delay_ms:.1} ms")).color(theme::TEXT_2));
        if app.params.sync.value() {
            if let Some(label) = DIVISION_LABELS.get(app.params.division.value() as usize) {
                ui.add_space(8.0);
                status_chip(ui, label, ChipTone::Accent);
            }
        }

        // Right-aligned status chips: FREEZE (warm when latched),
        // VOICE (Pitch-Sync scheduler only; mint while the PSOLA path
        // is engaged, em dash while unvoiced), GRAINS n (accent while
        // grains sound).
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(12.0);
            let frozen = app.params.freeze.value();
            status_chip(
                ui,
                "FREEZE",
                if frozen { ChipTone::Warm } else { ChipTone::Dim },
            );
            ui.add_space(6.0);

            if app.params.scheduler.value() == 2 {
                let engaged = app.viz.read_engaged();
                let hz = app.viz.read_period_hz();
                let voices = app.viz.read_psola_voices();
                if engaged && hz > 0.0 {
                    status_chip(
                        ui,
                        &format!("VOICE {hz:.0} Hz · {voices}v"),
                        ChipTone::Good,
                    );
                } else {
                    status_chip(ui, "VOICE —", ChipTone::Dim);
                }
                ui.add_space(6.0);
            }

            let grains = app.viz.read_active_grains();
            status_chip(
                ui,
                &format!("GRAINS {grains}"),
                if grains > 0 {
                    ChipTone::Accent
                } else {
                    ChipTone::Dim
                },
            );
        });
    });
}
