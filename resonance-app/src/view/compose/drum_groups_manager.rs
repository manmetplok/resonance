//! Drum Groups Manager modal — three-column layout for building drum
//! groups out of kit pads:
//!
//!   1. Groups list (left, 240 px).
//!   2. Active group detail (middle).
//!   3. Kit pad picker (right) with filter + categories.

use iced::widget::{
    button, column, container, mouse_area, opaque, row, scrollable, stack, text, text_input,
    Space,
};
use iced::{alignment, Color, Element, Length};
use std::collections::BTreeMap;

use crate::compose::drumroll::{DrumGroup, KitPadInfo, GROUP_PALETTE};
use crate::compose::messages::DrumGroupsMessage;
use crate::compose::ComposeMessage;
use crate::message::Message;
use crate::theme;
use crate::Resonance;

pub fn view<'a>(r: &'a Resonance) -> Element<'a, Message> {
    if !r.compose.drumroll.manager_open {
        return container(Space::with_height(0)).into();
    }

    let backdrop = mouse_area(
        container(Space::new(Length::Fill, Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba(
                    0.0, 0.0, 0.0, 0.7,
                ))),
                ..Default::default()
            }),
    )
    .on_press(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::CloseManager,
    )));

    let header = modal_header();
    let body = three_column_body(r);

    let sheet = container(column![header, body].spacing(0))
        .width(Length::Fixed(1100.0))
        .height(Length::Fixed(660.0))
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            border: iced::Border {
                color: theme::LINE,
                width: 1.0,
                radius: theme::RADIUS_XL.into(),
            },
            ..Default::default()
        });

    let centered = container(opaque(sheet))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill);

    stack![backdrop, centered].into()
}

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

fn modal_header<'a>() -> Element<'a, Message> {
    let title = column![
        text("DRUMS")
            .size(10)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
        text("Manage drum groups")
            .size(22)
            .font(theme::SERIF_ITALIC_FONT)
            .color(theme::TEXT_1),
        text(
            "A drum group shares one generated rhythm. Hits are distributed across the group's pads by the articulation mix — ideal for grouping hi-hat closed/open/half-open so a single 16th-note pattern moves through them naturally."
        )
        .size(11)
        .color(theme::TEXT_3),
    ]
    .spacing(2);

    let cancel = button(text("Cancel").size(12).color(theme::TEXT_2))
        .padding([8, 14])
        .style(|_theme, status| theme::ghost_button_style(status))
        .on_press(Message::Compose(ComposeMessage::DrumGroups(
            DrumGroupsMessage::CloseManager,
        )));
    let done = button(
        text("Done")
            .size(12)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(Color::from_rgb(0.10, 0.08, 0.04)),
    )
    .padding([8, 16])
    .style(|_theme, status| {
        let bg = match status {
            button::Status::Hovered => Color {
                r: (theme::WARM.r * 1.08).min(1.0),
                g: (theme::WARM.g * 1.08).min(1.0),
                b: (theme::WARM.b * 1.08).min(1.0),
                a: 1.0,
            },
            button::Status::Pressed => Color {
                r: theme::WARM.r * 0.85,
                g: theme::WARM.g * 0.85,
                b: theme::WARM.b * 0.85,
                a: 1.0,
            },
            _ => theme::WARM,
        };
        button::Style {
            background: Some(iced::Background::Color(bg)),
            text_color: Color::from_rgb(0.10, 0.08, 0.04),
            border: iced::Border {
                color: theme::WARM,
                width: 0.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        }
    })
    .on_press(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::CloseManager,
    )));

    container(
        row![title, Space::with_width(Length::Fill), cancel, done]
            .spacing(8)
            .align_y(alignment::Vertical::Top),
    )
    .padding([16, 20])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_1)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    })
    .into()
}

// ---------------------------------------------------------------------------
// Body — three columns
// ---------------------------------------------------------------------------

fn three_column_body<'a>(r: &'a Resonance) -> Element<'a, Message> {
    let groups_column = groups_list_column(r);
    let detail_column = group_detail_column(r);
    let kit_column = kit_picker_column(r);

    row![groups_column, detail_column, kit_column]
        .spacing(0)
        .height(Length::Fill)
        .into()
}

fn col_head<'a>(title: &str, meta: String) -> Element<'a, Message> {
    let owned_title = title.to_string();
    container(
        row![
            text(owned_title)
                .size(10)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(theme::TEXT_3),
            Space::with_width(Length::Fill),
            text(meta).size(10).font(theme::MONO_FONT).color(theme::TEXT_3),
        ]
        .align_y(alignment::Vertical::Center),
    )
    .padding([8, 0])
    .width(Length::Fill)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_1)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 0.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    })
    .into()
}

fn separator_below<'a>() -> Element<'a, Message> {
    container(Space::with_height(1))
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::LINE_2)),
            ..Default::default()
        })
        .into()
}

fn column_panel<'a>(content: Element<'a, Message>, width: Length) -> Element<'a, Message> {
    container(content)
        .padding([14, 16])
        .width(width)
        .height(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}

// ----- Column 1: groups list ----------------------------------------------

fn groups_list_column<'a>(r: &'a Resonance) -> Element<'a, Message> {
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

// ----- Column 2: detail ---------------------------------------------------

fn group_detail_column<'a>(r: &'a Resonance) -> Element<'a, Message> {
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

// ----- Column 3: kit picker -----------------------------------------------

fn kit_picker_column<'a>(r: &'a Resonance) -> Element<'a, Message> {
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
                .style(|_theme, status| theme::borderless_text_input_style(_theme, status)),
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
            Space::with_height(8),
            search,
            Space::with_height(6),
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
            let dot = container(Space::new(Length::Fixed(5.0), Length::Fixed(5.0)))
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

fn empty_hint<'a>(msg: &str) -> Element<'a, Message> {
    let msg = msg.to_string();
    container(text(msg).size(11).color(theme::TEXT_3))
        .padding([16, 14])
        .width(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(Color::TRANSPARENT)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_LG.into(),
            },
            ..Default::default()
        })
        .into()
}

fn u8_color(rgb: [u8; 3]) -> Color {
    Color::from_rgb(
        rgb[0] as f32 / 255.0,
        rgb[1] as f32 / 255.0,
        rgb[2] as f32 / 255.0,
    )
}
