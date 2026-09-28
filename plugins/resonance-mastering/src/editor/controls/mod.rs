//! Per-stage control panels.
//!
//! The editor renders a tab bar at the top of the window and a matching
//! stage-controls panel above the history traces. Each tab maps to one
//! DSP stage; its file in this module draws the knobs / toggles /
//! dropdowns for that stage and commits values back to the atomic
//! plugin params.

use plugin_gui_core::egui;

use crate::assistant::Genre;
use crate::params::MasteringParams;
use crate::viz::MasteringViz;

use super::theme;
use super::TargetSource;

mod assistant;
mod clipper;
mod dither;
mod eq;
mod glue;
pub mod gr_meter;
mod imager;
mod limiter;
pub mod multiband;
mod saturator;
mod widgets;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(Default)]
pub enum StageTab {
    #[default]
    Assistant,
    CorrectiveEq,
    Glue,
    Saturator,
    TonalEq,
    Multiband,
    Imager,
    Clipper,
    Limiter,
    Dither,
}


const TABS: &[(StageTab, &str)] = &[
    (StageTab::Assistant, "Assistant"),
    (StageTab::CorrectiveEq, "Corrective EQ"),
    (StageTab::Glue, "Glue Comp"),
    (StageTab::Saturator, "Saturator"),
    (StageTab::TonalEq, "Tonal EQ"),
    (StageTab::Multiband, "Multiband"),
    (StageTab::Imager, "Imager"),
    (StageTab::Clipper, "Clipper"),
    (StageTab::Limiter, "Limiter"),
    (StageTab::Dither, "Dither"),
];

/// Height of the stage-controls panel for a tab.
///
/// Most stages fit one row of knobs under a title row. The Assistant is
/// a whole workflow, and the Multiband is four band columns of two knob
/// lines each — both need more room, and giving it to them here is what
/// keeps the other seven tabs compact.
pub fn stage_panel_height(tab: StageTab) -> f32 {
    match tab {
        StageTab::Assistant => 380.0,
        StageTab::Multiband => 300.0,
        _ => 260.0,
    }
}

pub fn draw_tab_bar(ui: &mut egui::Ui, current: &mut StageTab) {
    ui.add_space(4.0);
    ui.horizontal_centered(|ui| {
        ui.add_space(12.0);
        for (tab, label) in TABS {
            let selected = *current == *tab;
            let mut rich = egui::RichText::new(*label).size(13.0);
            if selected {
                rich = rich.strong().color(theme::ACCENT);
            } else {
                rich = rich.color(theme::TEXT_DIM);
            }
            if ui.selectable_label(selected, rich).clicked() {
                *current = *tab;
            }
            ui.add_space(4.0);
        }
    });
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_stage_panel(
    ui: &mut egui::Ui,
    stage: StageTab,
    params: &MasteringParams,
    viz: &MasteringViz,
    selected_genre: &mut Genre,
    target_source: &mut TargetSource,
    reference_path: &mut String,
) {
    let rect = ui.available_rect_before_wrap();
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    match stage {
        StageTab::Assistant => assistant::draw(
            ui,
            params,
            &viz.assistant,
            selected_genre,
            target_source,
            reference_path,
        ),
        StageTab::CorrectiveEq => eq::draw(ui, &params.corrective_eq, "Corrective EQ"),
        StageTab::Glue => glue::draw(ui, &params.glue_compressor),
        StageTab::Saturator => saturator::draw(ui, &params.saturator),
        StageTab::TonalEq => eq::draw(ui, &params.tonal_eq, "Tonal EQ"),
        StageTab::Multiband => multiband::draw(ui, &params.multiband, viz.band_gr_db()),
        StageTab::Imager => imager::draw(ui, &params.imager),
        StageTab::Clipper => clipper::draw(ui, &params.clipper),
        StageTab::Limiter => limiter::draw(ui, &params.limiter),
        StageTab::Dither => dither::draw(ui, &params.dither),
    }
}
