//! Column 2: active group detail — name + colour palette swatch row,
//! assigned-pad list, footer with Clear / Delete actions.

use iced::widget::{button, column, container, row, scrollable, text, text_input, Space};
use iced::{alignment, Color, Element, Length};

use crate::compose::drumroll::{DrumGroup, GROUP_PALETTE};
use crate::compose::messages::DrumGroupsMessage;
use crate::compose::ComposeMessage;
use crate::message::Message;
use crate::theme;
use crate::Resonance;

use super::{col_head, column_panel, empty_hint, separator_below, u8_color};

pub(super) fn group_detail_column<'a>(r: &'a Resonance) -> Element<'a, Message> {
    let groups = &r.compose.drum_groups;
    let active = r
        .compose
        .drumroll
        .managing_group_id
        .and_then(|id| groups.iter().find(|g| g.id == id));

    let body: Element<'a, Message> = if let Some(group) = active {
        detail_body(group)
    } else {
        empty_hint("No group selected")
    };

    let pads_count = active.map(|g| g.pads.len()).unwrap_or(0);
    let meta = if active.is_some() {
        format!("{} pad{}", pads_count, if pads_count == 1 { "" } else { "s" })
    } else {
        String::new()
    };
    let head = col_head("GROUP", meta);

    column_panel(
        column![head, separator_below(), Space::with_height(8), body]
            .spacing(0)
            .into(),
        Length::Fill,
    )
}

fn detail_body<'a>(group: &'a DrumGroup) -> Element<'a, Message> {
    let color = u8_color(group.color);
    let id = group.id;

    let swatch = container(Space::new(Length::Fixed(22.0), Length::Fixed(22.0)))
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(color)),
            border: iced::Border {
                color,
                width: 0.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        });
    let name_input = text_input("Group name", &group.name)
        .on_input(move |s| {
            Message::Compose(ComposeMessage::DrumGroups(DrumGroupsMessage::RenameGroup {
                group_id: id,
                name: s,
            }))
        })
        .size(20)
        .padding([4, 8])
        .style(|_theme, status| {
            let _ = status;
            iced::widget::text_input::Style {
                background: iced::Background::Color(Color::TRANSPARENT),
                border: iced::Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: 0.0.into(),
                },
                icon: theme::TEXT_2,
                placeholder: Color { a: 0.4, ..theme::TEXT_3 },
                value: theme::TEXT_1,
                selection: Color { a: 0.35, ..theme::WARM },
            }
        });

    let mut palette_row = iced::widget::Row::new().spacing(4);
    for &c in GROUP_PALETTE {
        let color = u8_color(c);
        let is_active = c == group.color;
        let btn = button(Space::new(Length::Fixed(14.0), Length::Fixed(14.0)))
            .padding(0)
            .style(move |_theme, status| {
                let _ = status;
                button::Style {
                    background: Some(iced::Background::Color(color)),
                    text_color: Color::TRANSPARENT,
                    border: iced::Border {
                        color: if is_active {
                            theme::TEXT_1
                        } else {
                            theme::LINE_2
                        },
                        width: if is_active { 2.0 } else { 1.0 },
                        radius: theme::RADIUS_XS.into(),
                    },
                    ..Default::default()
                }
            })
            .on_press(Message::Compose(ComposeMessage::DrumGroups(
                DrumGroupsMessage::SetGroupColor {
                    group_id: id,
                    color: c,
                },
            )));
        palette_row = palette_row.push(btn);
    }

    let head_row = row![
        swatch,
        name_input.width(Length::Fill),
        palette_row,
    ]
    .spacing(10)
    .align_y(alignment::Vertical::Center);

    // Assigned pads list.
    let assigned_title = text("ASSIGNED PADS")
        .size(9.5)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(theme::TEXT_3);

    let assigned_list: Element<'a, Message> = if group.pads.is_empty() {
        empty_hint("No pads yet — click pads on the right to add them.")
    } else {
        let mut rows: Vec<Element<'a, Message>> = Vec::new();
        for p in &group.pads {
            let note = p.note;
            let row_inner = row![
                text(p.name.clone()).size(12).color(theme::TEXT_1).width(Length::Fill),
                text(format!("{}", p.note)).size(10).font(theme::MONO_FONT).color(theme::TEXT_3),
                button(text("\u{00d7}").size(15).color(theme::TEXT_3))
                    .padding([0, 6])
                    .style(|_theme, status| theme::small_button_style(status))
                    .on_press(Message::Compose(ComposeMessage::DrumGroups(
                        DrumGroupsMessage::TogglePadAssignment {
                            group_id: id,
                            note,
                        },
                    ))),
            ]
            .spacing(8)
            .align_y(alignment::Vertical::Center);
            rows.push(
                container(row_inner)
                    .padding([7, 10])
                    .width(Length::Fill)
                    .style({
                        let c = color;
                        move |_theme| container::Style {
                            background: Some(iced::Background::Color(theme::BG_2)),
                            border: iced::Border {
                                color: Color { a: 0.40, ..c },
                                width: 1.0,
                                radius: theme::RADIUS_MD.into(),
                            },
                            ..Default::default()
                        }
                    })
                    .into(),
            );
        }
        column(rows).spacing(4).into()
    };

    // Footer: Clear pads + Delete group.
    let clear_btn = button(text("Clear pads").size(11).color(theme::TEXT_2))
        .padding([6, 11])
        .style(|_theme, status| theme::ghost_button_style(status))
        .on_press(Message::Compose(ComposeMessage::DrumGroups(
            DrumGroupsMessage::ClearGroupPads { group_id: id },
        )));
    let delete_btn = button(text("Delete group").size(11).color(theme::BAD))
        .padding([6, 11])
        .style(|_theme, status| {
            let _ = status;
            button::Style {
                background: Some(iced::Background::Color(Color::TRANSPARENT)),
                text_color: theme::BAD,
                border: iced::Border {
                    color: Color { a: 0.4, ..theme::BAD },
                    width: 1.0,
                    radius: theme::RADIUS_MD.into(),
                },
                ..Default::default()
            }
        })
        .on_press(Message::Compose(ComposeMessage::DrumGroups(
            DrumGroupsMessage::DeleteGroup { group_id: id },
        )));

    let footer = row![Space::with_width(Length::Fill), clear_btn, delete_btn].spacing(6);

    let body = column![
        head_row,
        Space::with_height(14),
        assigned_title,
        Space::with_height(8),
        assigned_list,
        Space::with_height(14),
        footer,
    ]
    .spacing(0);

    container(
        scrollable(body).direction(scrollable::Direction::Vertical(
            scrollable::Scrollbar::default(),
        )),
    )
    .height(Length::Fill)
    .into()
}
