//! Column 1: groups list — one tile per drum group plus the "New group"
//! row at the bottom.

use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{alignment, Color, Element, Length};

use crate::compose::drumroll::DrumGroup;
use crate::compose::messages::DrumGroupsMessage;
use crate::compose::ComposeMessage;
use crate::message::Message;
use crate::theme;
use crate::Resonance;

use super::{col_head, column_panel, separator_below, u8_color};

pub(super) fn groups_list_column<'a>(r: &'a Resonance) -> Element<'a, Message> {
    let groups = &r.compose.drum_groups;
    let head = col_head("GROUPS", format!("{}", groups.len()));

    let active = r.compose.drumroll.managing_group_id;

    let mut items: Vec<Element<'a, Message>> = Vec::new();
    for g in groups {
        items.push(group_tile(g, Some(g.id) == active));
    }
    items.push(add_group_button());

    let list = column(items).spacing(4);
    let list_scroll = container(
        scrollable(list).direction(scrollable::Direction::Vertical(
            scrollable::Scrollbar::default(),
        )),
    )
    .height(Length::Fill);

    column_panel(
        column![head, separator_below(), Space::with_height(8), list_scroll].spacing(0).into(),
        Length::Fixed(240.0),
    )
}

fn group_tile<'a>(g: &'a DrumGroup, active: bool) -> Element<'a, Message> {
    let color = u8_color(g.color);
    let dot = container(Space::new(Length::Fixed(8.0), Length::Fixed(8.0)))
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(color)),
            border: iced::Border {
                color,
                width: 0.0,
                radius: 999.0.into(),
            },
            ..Default::default()
        });

    let name = text(g.name.clone())
        .size(13)
        .font(theme::SERIF_ITALIC_FONT)
        .color(if active { theme::TEXT_1 } else { theme::TEXT_2 });

    let count = text(format!("{}", g.pads.len()))
        .size(11)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_3);

    let inner = row![dot, name, Space::with_width(Length::Fill), count]
        .spacing(8)
        .align_y(alignment::Vertical::Center);

    let group_id = g.id;
    button(inner)
        .padding([9, 11])
        .width(Length::Fill)
        .style(move |_theme, status| group_tile_style(active, color, status))
        .on_press(Message::Compose(ComposeMessage::DrumGroups(
            DrumGroupsMessage::ManagerSelectGroup { group_id },
        )))
        .into()
}

fn group_tile_style(active: bool, color: Color, status: button::Status) -> button::Style {
    let bg = if active {
        Color { a: 0.14, ..color }
    } else if matches!(status, button::Status::Hovered) {
        theme::BG_2
    } else {
        Color::TRANSPARENT
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: theme::TEXT_1,
        border: iced::Border {
            color: if active { color } else { theme::LINE_2 },
            width: 1.0,
            radius: theme::RADIUS_LG.into(),
        },
        ..Default::default()
    }
}

fn add_group_button<'a>() -> Element<'a, Message> {
    button(
        row![
            text("+").size(13).color(theme::TEXT_3),
            text("New group").size(11.5).color(theme::TEXT_3),
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center),
    )
    .padding([9, 11])
    .width(Length::Fill)
    .style(|_theme, status| {
        let bg = match status {
            button::Status::Hovered => theme::BG_2,
            _ => Color::TRANSPARENT,
        };
        button::Style {
            background: Some(iced::Background::Color(bg)),
            text_color: theme::TEXT_3,
            border: iced::Border {
                color: theme::LINE,
                width: 1.0,
                radius: theme::RADIUS_LG.into(),
            },
            ..Default::default()
        }
    })
    .on_press(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::AddGroup,
    )))
    .into()
}
