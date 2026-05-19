//! Column 1: patterns + groups — a compact pattern bank picker on top,
//! the list of groups inside the focused pattern below, ending with a
//! "+ New group" row.

use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{alignment, Color, Element, Length};

use crate::compose::drumroll::{DrumGroup, DrumPattern};
use crate::compose::messages::DrumGroupsMessage;
use crate::compose::ComposeMessage;
use crate::message::Message;
use crate::theme;
use crate::Resonance;

use super::{col_head, column_panel, separator_below, u8_color};

pub(super) fn groups_list_column<'a>(r: &'a Resonance) -> Element<'a, Message> {
    // Pattern picker block — chip per pattern + "+" add.
    let patterns_head = col_head(
        "PATTERNS",
        format!("{}", r.compose.drum_patterns.len()),
    );
    let active_pattern_id = r
        .compose
        .managing_pattern()
        .map(|p| p.id)
        .or(r.compose.default_drum_pattern_id);
    let mut pattern_items: Vec<Element<'a, Message>> = Vec::new();
    for p in &r.compose.drum_patterns {
        pattern_items.push(pattern_tile(p, Some(p.id) == active_pattern_id));
    }
    pattern_items.push(add_pattern_button());

    let active_groups: &[DrumGroup] = r
        .compose
        .managing_pattern()
        .map(|p| p.groups.as_slice())
        .unwrap_or(&[]);
    let groups_head = col_head("GROUPS", format!("{}", active_groups.len()));

    let active_group_id = r.compose.drumroll.managing_group_id;

    let mut items: Vec<Element<'a, Message>> = Vec::new();
    for g in active_groups {
        items.push(group_tile(g, Some(g.id) == active_group_id));
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
        column![
            patterns_head,
            separator_below(),
            Space::new().height(6),
            column(pattern_items).spacing(4),
            Space::new().height(10),
            groups_head,
            separator_below(),
            Space::new().height(8),
            list_scroll,
        ]
        .spacing(0)
        .into(),
        Length::Fixed(240.0),
    )
}

fn pattern_tile<'a>(p: &'a DrumPattern, active: bool) -> Element<'a, Message> {
    let color = u8_color(p.color);
    let dot =
        container(Space::new().width(Length::Fixed(8.0)).height(Length::Fixed(8.0)))
            .style(move |_theme| container::Style {
                background: Some(iced::Background::Color(color)),
                border: iced::Border {
                    color,
                    width: 0.0,
                    radius: 999.0.into(),
                },
                ..Default::default()
            });
    let name = text(p.name.clone())
        .size(12)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(if active { theme::TEXT_1 } else { theme::TEXT_2 });
    let group_count = text(format!("{} grp", p.groups.len()))
        .size(10)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_3);
    let inner = row![dot, name, Space::new().width(Length::Fill), group_count]
        .spacing(8)
        .align_y(alignment::Vertical::Center);
    let pattern_id = p.id;
    button(inner)
        .padding([7, 10])
        .width(Length::Fill)
        .style(move |_theme, status| group_tile_style(active, color, status))
        .on_press(Message::Compose(ComposeMessage::DrumGroups(
            DrumGroupsMessage::SelectPattern { pattern_id },
        )))
        .into()
}

fn add_pattern_button<'a>() -> Element<'a, Message> {
    button(
        row![
            text("+").size(12).color(theme::TEXT_3),
            text("New pattern").size(11).color(theme::TEXT_3),
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center),
    )
    .padding([7, 10])
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
        DrumGroupsMessage::AddPattern,
    )))
    .into()
}

fn group_tile<'a>(g: &'a DrumGroup, active: bool) -> Element<'a, Message> {
    let color = u8_color(g.color);
    let dot = container(Space::new().width(Length::Fixed(8.0)).height(Length::Fixed(8.0)))
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

    let inner = row![dot, name, Space::new().width(Length::Fill), count]
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
