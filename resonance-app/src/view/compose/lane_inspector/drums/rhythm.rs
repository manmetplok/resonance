//! Rhythm settings panel — density, swing, accent, humanize, fills.

use iced::widget::{column, row, slider, text, Space};
use iced::{alignment, Element, Length};

use crate::compose::drumroll::DrumGroup;
use crate::compose::messages::DrumGroupsMessage;
use crate::compose::ComposeMessage;
use crate::message::Message;
use crate::theme;

use super::common::{field, rail_card, rail_dot};

pub(super) fn rhythm_panel<'a>(group: &'a DrumGroup) -> Element<'a, Message> {
    let title = row![
        rail_dot(theme::WARM),
        text("Rhythm").size(12).color(theme::WARM),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);

    let id = group.id;
    let density = slider(0.0..=1.0, group.density, move |v| {
        Message::Compose(ComposeMessage::DrumGroups(
            DrumGroupsMessage::SetGroupDensity {
                group_id: id,
                density: v,
            },
        ))
    })
    .step(0.01)
    .width(Length::Fill);
    let swing = slider(0.0..=1.0, group.swing, move |v| {
        Message::Compose(ComposeMessage::DrumGroups(
            DrumGroupsMessage::SetGroupSwing {
                group_id: id,
                swing: v,
            },
        ))
    })
    .step(0.01)
    .width(Length::Fill);
    let accent = slider(0.0..=1.0, group.accent, move |v| {
        Message::Compose(ComposeMessage::DrumGroups(
            DrumGroupsMessage::SetGroupAccent {
                group_id: id,
                accent: v,
            },
        ))
    })
    .step(0.01)
    .width(Length::Fill);
    let humanize = slider(0.0..=1.0, group.humanize, move |v| {
        Message::Compose(ComposeMessage::DrumGroups(
            DrumGroupsMessage::SetGroupHumanize {
                group_id: id,
                humanize: v,
            },
        ))
    })
    .step(0.01)
    .width(Length::Fill);
    let fills = slider(0.0..=1.0, group.fills, move |v| {
        Message::Compose(ComposeMessage::DrumGroups(
            DrumGroupsMessage::SetGroupFills {
                group_id: id,
                fills: v,
            },
        ))
    })
    .step(0.01)
    .width(Length::Fill);

    rail_card(
        column![
            title,
            Space::new().height(8),
            field("Density", group.density, density),
            Space::new().height(6),
            field("Swing", group.swing, swing),
            Space::new().height(6),
            field("Accent", group.accent, accent),
            Space::new().height(6),
            field("Humanize", group.humanize, humanize),
            Space::new().height(6),
            field("Fills (last bar)", group.fills, fills),
            Space::new().height(6),
            text(group.style.clone()).size(10).color(theme::TEXT_4),
        ]
        .spacing(0)
        .into(),
    )
}
