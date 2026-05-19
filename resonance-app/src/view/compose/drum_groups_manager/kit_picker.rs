//! Column 3: kit-pad picker — categorised list with a filter input and
//! per-pad assign/unassign rows.

use iced::widget::{column, container, mouse_area, row, scrollable, text, text_input, Space};
use iced::{alignment, Color, Element, Length};
use std::collections::BTreeMap;

use crate::compose::drumroll::{DrumGroup, KitPadInfo};
use crate::compose::messages::DrumGroupsMessage;
use crate::compose::ComposeMessage;
use crate::message::Message;
use crate::theme;
use crate::Resonance;

use super::{col_head, column_panel, separator_below, u8_color};

pub(super) fn kit_picker_column<'a>(r: &'a Resonance) -> Element<'a, Message> {
    let head = col_head(
        "KIT",
        format!("Drummica · {} pads", r.compose.kit_pads.len()),
    );

    let filter = &r.compose.drumroll.manager_filter;
    let search = container(
        row![
            text("\u{1f50d}").size(11).color(theme::TEXT_3),
            text_input("Filter pads…", filter)
                .on_input(|s| {
                    Message::Compose(ComposeMessage::DrumGroups(
                        DrumGroupsMessage::ManagerSetFilter(s),
                    ))
                })
                .size(12)
                .padding([4, 6])
                .style(theme::borderless_text_input_style),
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center),
    )
    .padding([6, 10])
    .width(Length::Fill)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: theme::RADIUS_MD.into(),
        },
        ..Default::default()
    });

    let filter_lc = filter.to_ascii_lowercase();
    let active = r.compose.drumroll.managing_group_id;
    let mut by_cat: BTreeMap<&String, Vec<&KitPadInfo>> = BTreeMap::new();
    for pad in &r.compose.kit_pads {
        if !filter_lc.is_empty()
            && !pad.name.to_ascii_lowercase().contains(&filter_lc)
            && !pad.category.to_ascii_lowercase().contains(&filter_lc)
        {
            continue;
        }
        by_cat.entry(&pad.category).or_default().push(pad);
    }

    // pad note -> group id
    let mut owner: std::collections::HashMap<u8, u64> = std::collections::HashMap::new();
    for g in &r.compose.drum_groups {
        for p in &g.pads {
            owner.insert(p.note, g.id);
        }
    }

    let mut content: Vec<Element<'a, Message>> = Vec::new();
    for (cat, pads) in &by_cat {
        let head = text(cat.to_ascii_uppercase())
            .size(9.5)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_4);
        content.push(container(head).padding([8, 4]).into());
        for p in pads {
            content.push(pad_row(p, active, &owner, &r.compose.drum_groups));
        }
    }
    let pad_list = container(
        scrollable(column(content).spacing(3))
            .direction(scrollable::Direction::Vertical(
                scrollable::Scrollbar::default(),
            )),
    )
    .height(Length::Fill);

    let hint: Element<'a, Message> = text("A pad belongs to one group at a time. Adding it to this group removes it from any other.")
        .size(10.5)
        .color(theme::TEXT_3)
        .into();

    column_panel(
        column![
            head,
            separator_below(),
            Space::new().height(8),
            search,
            Space::new().height(6),
            pad_list,
            container(hint).padding([8, 4]),
        ]
        .spacing(0)
        .into(),
        Length::Fixed(380.0),
    )
}

fn pad_row<'a>(
    pad: &'a KitPadInfo,
    active: Option<u64>,
    owner: &std::collections::HashMap<u8, u64>,
    groups: &'a [DrumGroup],
) -> Element<'a, Message> {
    let owner_id = owner.get(&pad.note).copied();
    let in_active = owner_id == active && active.is_some();
    let owner_group = owner_id.and_then(|id| groups.iter().find(|g| g.id == id));
    let active_group = active.and_then(|id| groups.iter().find(|g| g.id == id));
    let active_color = active_group.map(|g| u8_color(g.color)).unwrap_or(theme::TEXT_3);

    let name = text(pad.name.clone())
        .size(12)
        .color(if owner_id.is_some() { theme::TEXT_1 } else { theme::TEXT_2 })
        .width(Length::Fill);
    let note = text(format!("{}", pad.note))
        .size(9.5)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_3);

    let mut row_widget = iced::widget::Row::new()
        .spacing(8)
        .align_y(alignment::Vertical::Center)
        .push(name)
        .push(note);

    if let Some(g) = owner_group {
        if !in_active {
            let owner_color = u8_color(g.color);
            let dot = container(Space::new().width(Length::Fixed(5.0)).height(Length::Fixed(5.0)))
                .style(move |_theme| container::Style {
                    background: Some(iced::Background::Color(owner_color)),
                    border: iced::Border {
                        color: owner_color,
                        width: 0.0,
                        radius: 999.0.into(),
                    },
                    ..Default::default()
                });
            let owner_label = text(g.name.clone()).size(9.5).color(owner_color);
            let pill = container(
                row![dot, owner_label]
                    .spacing(5)
                    .align_y(alignment::Vertical::Center),
            )
            .padding([1, 7])
            .style({
                let c = owner_color;
                move |_theme| container::Style {
                    background: Some(iced::Background::Color(Color { a: 0.10, ..c })),
                    border: iced::Border {
                        color: Color { a: 0.50, ..c },
                        width: 1.0,
                        radius: 999.0.into(),
                    },
                    ..Default::default()
                }
            });
            row_widget = row_widget.push(pill);
        }
    }

    let toggle_label = if in_active { "\u{2713}" } else { "+" };
    let toggle = container(
        text(toggle_label.to_string())
            .size(11)
            .color(if in_active { Color::BLACK } else { theme::TEXT_3 })
            .align_x(alignment::Horizontal::Center),
    )
    .padding(0)
    .width(Length::Fixed(18.0))
    .height(Length::Fixed(18.0))
    .align_x(alignment::Horizontal::Center)
    .align_y(alignment::Vertical::Center)
    .style({
        let c = active_color;
        move |_theme| container::Style {
            background: Some(iced::Background::Color(if in_active {
                c
            } else {
                Color::TRANSPARENT
            })),
            border: iced::Border {
                color: if in_active { c } else { theme::LINE },
                width: 1.0,
                radius: theme::RADIUS_XS.into(),
            },
            ..Default::default()
        }
    });

    row_widget = row_widget.push(toggle);

    let row_color = active_color;
    let bg_style: Box<dyn Fn(&iced::Theme) -> container::Style> = if in_active {
        Box::new(move |_theme| container::Style {
            background: Some(iced::Background::Color(Color { a: 0.10, ..row_color })),
            border: iced::Border {
                color: Color { a: 0.50, ..row_color },
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        })
    } else if owner_id.is_some() {
        Box::new(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        })
    } else {
        Box::new(|_theme| container::Style {
            background: Some(iced::Background::Color(Color::TRANSPARENT)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        })
    };

    let note_value = pad.note;
    let area = container(row_widget).padding([7, 10]).style(bg_style);
    mouse_area(area)
        .on_press(match active {
            Some(group_id) => Message::Compose(ComposeMessage::DrumGroups(
                DrumGroupsMessage::TogglePadAssignment {
                    group_id,
                    note: note_value,
                },
            )),
            None => Message::Tick,
        })
        .into()
}
