//! Drum-lane right-rail inspector — group selector, meter (grid/cycle/
//! phase + polyrhythm/polymeter presets), articulation mix, rhythm
//! settings, generate.

use iced::widget::{button, column, container, row, text, Space};
use iced::{alignment, Color, Element, Length};

use crate::compose::drumroll::DrumGroup;
use crate::compose::messages::DrumGroupsMessage;
use crate::compose::{ComposeMessage, DrumrollViewState, SectionDefinitionState};
use crate::message::Message;
use crate::state::TrackState;
use crate::theme;

mod articulation;
mod common;
mod generate;
mod meter;
mod rhythm;

use articulation::articulation_mix_panel;
use common::{rail_card, rail_dot, u8_color};
use generate::generate_panel;
use meter::meter_panel;
use rhythm::rhythm_panel;

pub(super) fn drum_body<'a>(
    _definition: &'a SectionDefinitionState,
    _track: &'a TrackState,
    drumroll_state: &'a DrumrollViewState,
    drum_groups: &'a [DrumGroup],
    _clip_id: Option<u64>,
) -> Element<'a, Message> {
    let selected_group_id = drumroll_state
        .selected_group_id
        .or_else(|| drum_groups.first().map(|g| g.id));
    let group = selected_group_id
        .and_then(|id| drum_groups.iter().find(|g| g.id == id));

    let Some(group) = group else {
        return text("No drum groups — open the manager to add one.")
            .size(11)
            .color(theme::TEXT_DIM)
            .into();
    };

    let base_grid = drumroll_state.base_grid.max(2);
    let base_cycle = drumroll_state.base_cycle.max(1);

    column![
        group_selector(drum_groups, selected_group_id),
        Space::new().height(12),
        meter_panel(group, base_grid, base_cycle),
        Space::new().height(12),
        articulation_mix_panel(group),
        Space::new().height(12),
        rhythm_panel(group),
        Space::new().height(12),
        generate_panel(group),
    ]
    .spacing(0)
    .into()
}

// ===========================================================================
// Group selector + manage button
// ===========================================================================

fn group_selector<'a>(
    groups: &'a [DrumGroup],
    selected: Option<u64>,
) -> Element<'a, Message> {
    let title = row![
        rail_dot(theme::WARM),
        text("Drum group").size(12).color(theme::TEXT_1),
        Space::new().width(Length::Fill),
        button(text("+ Manage").size(11).color(theme::TEXT_2))
            .padding([4, 9])
            .style(|_theme, status| theme::ghost_button_style(status))
            .on_press(Message::Compose(ComposeMessage::DrumGroups(
                DrumGroupsMessage::OpenManager,
            ))),
    ]
    .align_y(alignment::Vertical::Center)
    .spacing(8);

    let mut tab_items: Vec<Element<'a, Message>> = Vec::new();
    for g in groups {
        let on = Some(g.id) == selected;
        let color = u8_color(g.color);
        let dot = container(Space::new().width(Length::Fixed(5.0)).height(Length::Fixed(5.0)))
            .style(move |_theme| container::Style {
                background: Some(iced::Background::Color(color)),
                border: iced::Border {
                    color,
                    width: 0.0,
                    radius: 999.0.into(),
                },
                ..Default::default()
            });
        let label = text(g.name.clone()).size(11).color(if on {
            theme::TEXT_1
        } else {
            theme::TEXT_2
        });
        let count = text(format!("{}", g.pads.len()))
            .size(9)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_4);
        let inner = row![dot, label, count]
            .spacing(6)
            .align_y(alignment::Vertical::Center);
        let group_id = g.id;
        let group_color = color;
        let btn: Element<'a, Message> = button(inner)
            .padding([6, 9])
            .style(move |_theme, status| group_tab_style(on, group_color, status))
            .on_press(Message::Compose(ComposeMessage::DrumGroups(
                DrumGroupsMessage::SelectGroup { group_id },
            )))
            .into();
        tab_items.push(btn);
    }
    let tab_row = iced::widget::Row::with_children(tab_items).spacing(4).wrap();

    let hint = text("One rhythm per group · hits distributed across articulations by the mix below.")
        .size(10)
        .color(theme::TEXT_3);

    rail_card(
        column![
            title,
            Space::new().height(8),
            tab_row,
            Space::new().height(6),
            hint,
        ]
        .spacing(0)
        .into(),
    )
}

fn group_tab_style(on: bool, color: Color, status: button::Status) -> button::Style {
    let bg = if on {
        Color { a: 0.18, ..color }
    } else if matches!(status, button::Status::Hovered) {
        theme::BG_3
    } else {
        Color::TRANSPARENT
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: theme::TEXT_1,
        border: iced::Border {
            color: if on { color } else { theme::LINE_2 },
            width: 1.0,
            radius: theme::RADIUS_SM.into(),
        },
        ..Default::default()
    }
}
