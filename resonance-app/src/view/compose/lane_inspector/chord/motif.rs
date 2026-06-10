//! Section motif block — Source select + Complexity slider + a small
//! preview card. Also hosts the manual-motif canvas embedding.

use iced::widget::{button, canvas, column, pick_list, slider, text, Space};
use iced::{Element, Length};

use resonance_music_theory::{ManualMotifNote, MotifSource, Scale};

use crate::compose::messages::{ChordInspectorMsg, MotifSourceKind};
use crate::compose::{ComposeMessage, SectionDefinitionState};
use crate::message::*;
use crate::theme;

use super::preview_canvas::motif_preview_card;
use super::{field_label, section_header, MotifLenPick};

/// Section motif block — Source select + Complexity slider + a small
/// preview card. Matches the design's "Section motif" panel.
pub(super) fn motif_section_block<'a>(
    definition: &'a SectionDefinitionState,
    collapsed: bool,
) -> Element<'a, Message> {
    let definition_id = definition.id;
    let params = *definition.motif_source.params();

    if collapsed {
        return section_header(
            "Section motif",
            crate::compose::RailPanelKey::SectionMotif,
            true,
        );
    }

    let source_kind = if definition.motif_source.is_manual() {
        MotifSourceKind::Manual
    } else {
        MotifSourceKind::Generated
    };
    let source_picker = pick_list(
        vec![MotifSourceKind::Generated, MotifSourceKind::Manual],
        Some(source_kind),
        move |kind| {
            Message::Compose(ComposeMessage::ChordInspector {
                definition_id,
                msg: ChordInspectorMsg::SetMotifSourceKind(kind),
            })
        },
    )
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);

    let complexity_slider = slider(0.0..=1.0, params.complexity, move |v| {
        Message::Compose(ComposeMessage::ChordInspector {
            definition_id,
            msg: ChordInspectorMsg::SetMotifComplexity(v),
        })
    })
    .step(0.01)
    .width(Length::Fill);

    let mut col = column![
        section_header("Section motif", crate::compose::RailPanelKey::SectionMotif, false),
        Space::new().height(10),
        field_label("SOURCE"),
        Space::new().height(4),
        source_picker,
        Space::new().height(10),
        field_label(format!("COMPLEXITY · {:.2}", params.complexity)),
        Space::new().height(6),
        complexity_slider,
    ]
    .spacing(0);

    // Motif preview card mirroring the design's MOTIF stats + dashes.
    col = col.push(Space::new().height(10));
    col = col.push(motif_preview_card(definition));

    match &definition.motif_source {
        MotifSource::Generated(_) => {
            let leap_slider = slider(0.0..=1.0, params.leap_chance, move |v| {
                Message::Compose(ComposeMessage::ChordInspector {
                    definition_id,
                    msg: ChordInspectorMsg::SetMotifLeapChance(v),
                })
            })
            .step(0.01)
            .width(Length::Fill);

            let motif_len_options: Vec<MotifLenPick> = vec![
                MotifLenPick(0),
                MotifLenPick(2),
                MotifLenPick(3),
                MotifLenPick(4),
                MotifLenPick(5),
                MotifLenPick(6),
            ];
            let motif_len_picker = pick_list(
                motif_len_options,
                Some(MotifLenPick(params.motif_len)),
                move |pick| {
                    Message::Compose(ComposeMessage::ChordInspector {
                        definition_id,
                        msg: ChordInspectorMsg::SetMotifLen(pick.0),
                    })
                },
            )
            .text_size(12)
            .padding([5, 8])
            .width(Length::Fill);

            let regen_btn = button(text("Regenerate motif").size(12).color(theme::TEXT_1))
                .padding([6, 10])
                .width(Length::Fill)
                .style(|_theme, status| theme::ghost_button_style(status))
                .on_press(Message::Compose(ComposeMessage::ChordInspector {
                    definition_id,
                    msg: ChordInspectorMsg::RegenerateMotif,
                }));

            let seed_label = text(format!("seed · 0x{:016X}", params.seed))
                .size(10)
                .font(theme::MONO_FONT)
                .color(theme::TEXT_3);

            col = col
                .push(Space::new().height(10))
                .push(field_label(format!(
                    "LEAP CHANCE · {:.2}",
                    params.leap_chance
                )))
                .push(Space::new().height(6))
                .push(leap_slider)
                .push(Space::new().height(10))
                .push(field_label("MOTIF LENGTH"))
                .push(Space::new().height(4))
                .push(motif_len_picker)
                .push(Space::new().height(10))
                .push(regen_btn)
                .push(Space::new().height(6))
                .push(seed_label)
                .push(Space::new().height(8))
                .push(
                    text("Click a cell to add a note. Right-click to toggle accent. Scroll a note to cycle duration.")
                        .size(10)
                        .color(theme::TEXT_3),
                );
        }
        MotifSource::Manual { notes, .. } => {
            let canvas_widget = manual_motif_canvas(definition_id, notes, definition.scale);
            let clear_btn = button(text("Clear motif").size(12).color(theme::TEXT_1))
                .padding([6, 10])
                .width(Length::Fill)
                .style(|_theme, status| theme::ghost_button_style(status))
                .on_press(Message::Compose(ComposeMessage::ChordInspector {
                    definition_id,
                    msg: ChordInspectorMsg::ClearManualMotif,
                }));

            col = col
                .push(Space::new().height(8))
                .push(canvas_widget)
                .push(Space::new().height(8))
                .push(
                    text("Click empty cell to add a note. Click the bottom row to insert a rest. Click the start of a note/rest to remove it. Right-click to toggle accent. Scroll on a note to cycle its duration.")
                        .size(10)
                        .color(theme::TEXT_3),
                )
                .push(Space::new().height(8))
                .push(clear_btn);
        }
    }

    col.into()
}

fn manual_motif_canvas<'a>(
    definition_id: u64,
    notes: &'a [ManualMotifNote],
    scale: Option<Scale>,
) -> Element<'a, Message> {
    use crate::view::compose::manual_motif_canvas::{CELL_H, CELL_W, GRID_COLS, TOTAL_ROWS};
    let canvas_w = CELL_W * GRID_COLS as f32;
    let canvas_h = CELL_H * TOTAL_ROWS as f32;
    canvas(
        crate::view::compose::manual_motif_canvas::ManualMotifCanvas {
            definition_id,
            notes,
            scale,
        },
    )
    .width(Length::Fixed(canvas_w))
    .height(Length::Fixed(canvas_h))
    .into()
}
