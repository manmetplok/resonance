//! Chord generator + section motif body — the main inspector body for
//! a chord lane.

use iced::widget::{button, column, pick_list, row, text, Space};
use iced::{alignment, Element, Length};

use resonance_music_theory::TableRegistry;

use crate::compose::messages::ChordInspectorMsg;
use crate::compose::{ComposeMessage, SectionDefinitionState};
use crate::message::*;
use crate::theme;

use super::motif::motif_section_block;
use super::{
    current_table_id, degree_picks_from, field_label, section_header, table_picks, toggle_row,
    DegreePick,
};

pub(in crate::view::compose::lane_inspector) fn chord_body<'a>(
    definition: &'a SectionDefinitionState,
    table_registry: &'a TableRegistry,
    collapsed_panels: &std::collections::HashSet<crate::compose::RailPanelKey>,
) -> Element<'a, Message> {
    use crate::compose::RailPanelKey;

    let definition_id = definition.id;
    let has_scale = definition.scale.is_some();
    let current_table = current_table_id(definition);

    let gen_collapsed = collapsed_panels.contains(&RailPanelKey::ChordGenerator);
    let motif_collapsed = collapsed_panels.contains(&RailPanelKey::SectionMotif);
    let motif_block = motif_section_block(definition, motif_collapsed);

    // Folded generator: header only, motif panel still follows.
    if gen_collapsed {
        return column![
            section_header("Chord generator", RailPanelKey::ChordGenerator, true),
            Space::new().height(20),
            motif_block,
        ]
        .spacing(0)
        .into();
    }

    let tables = table_picks();
    let current_pick = tables.iter().find(|t| t.id == current_table).cloned();
    let table_picker = pick_list(tables, current_pick, move |pick| {
        Message::Compose(ComposeMessage::ChordInspector {
            definition_id,
            msg: ChordInspectorMsg::SetTable(pick.id),
        })
    })
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);

    let current_length = match &definition.generator_spec {
        Some(resonance_music_theory::GeneratorSpec::MarkovProgression { length, .. }) => *length,
        None => definition.generate_params.chord_count as u8,
    };
    let count_picker = pick_list(count_options(), Some(current_length), move |n| {
        Message::Compose(ComposeMessage::ChordInspector {
            definition_id,
            msg: ChordInspectorMsg::SetLength(n),
        })
    })
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);

    let beats_picker = pick_list(beats_options(), Some(definition.beats_per_chord), move |n| {
        Message::Compose(ComposeMessage::ChordInspector {
            definition_id,
            msg: ChordInspectorMsg::SetBeatsPerChord(n),
        })
    })
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);

    let table_degrees = table_registry
        .get(&current_table)
        .map(|t| t.degrees())
        .unwrap_or_default();
    let degree_options = degree_picks_from(&table_degrees);

    let current_start = match &definition.generator_spec {
        Some(resonance_music_theory::GeneratorSpec::MarkovProgression { start, .. }) => {
            DegreePick(*start)
        }
        None => DegreePick(None),
    };
    let current_end = match &definition.generator_spec {
        Some(resonance_music_theory::GeneratorSpec::MarkovProgression { end, .. }) => {
            DegreePick(*end)
        }
        None => DegreePick(None),
    };

    let start_picker = pick_list(degree_options.clone(), Some(current_start), move |pick| {
        Message::Compose(ComposeMessage::ChordInspector {
            definition_id,
            msg: ChordInspectorMsg::SetStartDegree(pick.0),
        })
    })
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);

    let end_picker = pick_list(degree_options, Some(current_end), move |pick| {
        Message::Compose(ComposeMessage::ChordInspector {
            definition_id,
            msg: ChordInspectorMsg::SetEndDegree(pick.0),
        })
    })
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);

    let sevenths = toggle_row(
        "Seventh chords",
        definition.seventh_chords,
        Message::Compose(ComposeMessage::ChordInspector {
            definition_id,
            msg: ChordInspectorMsg::SetSeventhChords(!definition.seventh_chords),
        }),
    );

    // Generate primary action + ↻ regenerate ghost.
    let gen_btn: Element<'_, Message> = {
        let label_color = if has_scale {
            theme::BG_0
        } else {
            theme::TEXT_3
        };
        let mut b = button(
            text("Generate")
                .size(12)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(label_color),
        )
        .padding([8, 14])
        .width(Length::Fill)
        .style(move |_theme, status| {
            if has_scale {
                theme::primary_button_style(status)
            } else {
                theme::ghost_button_style(status)
            }
        });
        if has_scale {
            b = b.on_press(Message::Compose(ComposeMessage::ChordInspector {
                definition_id,
                msg: ChordInspectorMsg::Generate,
            }));
        }
        b.into()
    };

    let regen_active = has_scale && definition.generated_material.is_some();
    let regen_btn: Element<'_, Message> = {
        let icon_color = if regen_active {
            theme::TEXT_1
        } else {
            theme::TEXT_3
        };
        let mut b = button(
            iced::widget::container(theme::icon(theme::fa::ARROW_ROTATE_LEFT).size(13).color(icon_color))
                .center_x(Length::Fill)
                .center_y(Length::Fill),
        )
        .padding(0)
        .width(Length::Fixed(40.0))
        .height(Length::Fixed(34.0))
        .style(|_theme, status| theme::ghost_button_style(status));
        if regen_active {
            b = b.on_press(Message::Compose(ComposeMessage::ChordInspector {
                definition_id,
                msg: ChordInspectorMsg::Regenerate,
            }));
        }
        b.into()
    };

    let action_row = row![gen_btn, Space::new().width(8), regen_btn]
        .spacing(0)
        .align_y(alignment::Vertical::Center);

    let lock_count = definition
        .generated_material
        .as_ref()
        .map(|m| m.chords.iter().filter(|c| c.locked).count())
        .unwrap_or(0);
    let lock_label: Element<'_, Message> = if lock_count > 0 {
        text(format!("{lock_count} chord(s) locked"))
            .size(10)
            .color(theme::TEXT_3)
            .into()
    } else {
        text("Click a chord to toggle lock")
            .size(10)
            .color(theme::TEXT_3)
            .into()
    };

    let scale_helper: Element<'_, Message> = if !has_scale {
        text("Pick a scale above to enable generation.")
            .size(10)
            .color(theme::TEXT_3)
            .into()
    } else {
        Space::new().height(0).into()
    };

    let seed_label = text(format!("seed · 0x{:016X}", definition.generator_seed))
        .size(10)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_3);

    let two_cols = |left: Element<'a, Message>, right: Element<'a, Message>| {
        row![
            column![left].width(Length::FillPortion(1)),
            Space::new().width(10),
            column![right].width(Length::FillPortion(1)),
        ]
        .spacing(0)
    };

    let chords_count_block = column![
        field_label("CHORDS"),
        Space::new().height(4),
        count_picker,
    ];
    let beats_block = column![
        field_label("BEATS / CHORD"),
        Space::new().height(4),
        beats_picker,
    ];
    let start_block = column![
        field_label("START °"),
        Space::new().height(4),
        start_picker,
    ];
    let end_block = column![field_label("END °"), Space::new().height(4), end_picker,];

    column![
        section_header("Chord generator", RailPanelKey::ChordGenerator, false),
        Space::new().height(10),
        field_label("STYLE"),
        Space::new().height(4),
        table_picker,
        Space::new().height(10),
        two_cols(chords_count_block.into(), beats_block.into()),
        Space::new().height(10),
        two_cols(start_block.into(), end_block.into()),
        Space::new().height(10),
        sevenths,
        Space::new().height(12),
        action_row,
        Space::new().height(8),
        seed_label,
        Space::new().height(6),
        lock_label,
        scale_helper,
        Space::new().height(20),
        motif_block,
    ]
    .spacing(0)
    .into()
}

// ---------------------------------------------------------------------------
// Cached pick_list option vectors.
//
// `pick_list` takes its options by value (a `Borrow<[T]>` slice), so a
// fresh `Vec<u8>`/`Vec<u32>` would be allocated on every repaint while
// this inspector body is visible. These statics are populated on first
// access and reused thereafter. See view-layer performance memory.
// ---------------------------------------------------------------------------

/// Chord-count options (1..=16), cached as a static slice.
fn count_options() -> &'static [u8] {
    static V: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    V.get_or_init(|| (1..=16).collect())
}

/// Beats-per-chord options (1, 2, 4, 8, 16), cached as a static slice.
fn beats_options() -> &'static [u32] {
    static V: std::sync::OnceLock<Vec<u32>> = std::sync::OnceLock::new();
    V.get_or_init(|| vec![1, 2, 4, 8, 16])
}
