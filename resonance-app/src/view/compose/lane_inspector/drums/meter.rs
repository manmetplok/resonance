//! Meter panel: grid/cycle/phase steppers + polyrhythm/polymeter presets.

use iced::widget::{button, column, container, pick_list, row, slider, text, Space};
use iced::{alignment, Color, Element, Length};

use crate::compose::drumroll::{grid_label, DrumGroup};
use crate::compose::messages::DrumGroupsMessage;
use crate::compose::ComposeMessage;
use crate::message::Message;
use crate::theme;

use super::common::{rail_card, rail_dot, section_head, u8_color, BEATS_PER_BAR};

pub(super) fn meter_panel<'a>(group: &'a DrumGroup, base_grid: u8, base_cycle: u32) -> Element<'a, Message> {
    let color = u8_color(group.color);

    let title = row![
        rail_dot(color),
        text(format!("{} · meter", group.name)).size(12).color(theme::WARM),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);

    // Reference banner.
    let ref_label = format!(
        "{}/{} · {} · {} steps",
        BEATS_PER_BAR, BEATS_PER_BAR, grid_label(base_grid), base_cycle
    );
    let ref_banner = container(
        column![
            row![
                text("REFERENCE")
                    .size(9)
                    .font(theme::UI_FONT_SEMIBOLD)
                    .color(theme::TEXT_3),
                Space::with_width(Length::Fill),
                text("SECTION BASE")
                    .size(9)
                    .font(theme::UI_FONT_SEMIBOLD)
                    .color(theme::TEXT_4),
            ],
            Space::with_height(4),
            text(ref_label).size(11).font(theme::MONO_FONT).color(theme::TEXT_1),
            Space::with_height(2),
            text("Ratios below set this group's relationship to the section's felt pulse.")
                .size(10)
                .color(theme::TEXT_3),
        ]
        .spacing(0),
    )
    .padding([9, 11])
    .width(Length::Fill)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_1)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: theme::RADIUS_LG.into(),
        },
        ..Default::default()
    });

    // Grid + cycle steppers (as a pick_list for grid, slider/stepper for cycle).
    let grid_pick = pick_list(
        vec![2u8, 3, 4, 5, 6, 7],
        Some(group.grid),
        {
            let id = group.id;
            move |g: u8| {
                Message::Compose(ComposeMessage::DrumGroups(
                    DrumGroupsMessage::SetGroupGrid { group_id: id, grid: g },
                ))
            }
        },
    )
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);

    let cycle_slider = slider(1.0..=32.0, group.cycle as f32, {
        let id = group.id;
        move |v: f32| {
            Message::Compose(ComposeMessage::DrumGroups(
                DrumGroupsMessage::SetGroupCycle {
                    group_id: id,
                    cycle: v.round() as u32,
                },
            ))
        }
    })
    .step(1.0)
    .width(Length::Fill);

    let grid_field = column![
        text("GRID")
            .size(10)
            .color(theme::TEXT_3),
        Space::with_height(4),
        grid_pick,
    ];
    let cycle_field = column![
        row![
            text("CYCLE").size(10).color(theme::TEXT_3),
            Space::with_width(Length::Fill),
            text(format!("{}", group.cycle))
                .size(10)
                .font(theme::MONO_FONT)
                .color(theme::TEXT_2),
        ],
        Space::with_height(4),
        cycle_slider,
    ];

    let phase_max = group.cycle.max(1);
    let phase_slider = slider(0.0..=phase_max as f32, group.phase as f32, {
        let id = group.id;
        move |v: f32| {
            Message::Compose(ComposeMessage::DrumGroups(
                DrumGroupsMessage::SetGroupPhase {
                    group_id: id,
                    phase: v.round() as u32,
                },
            ))
        }
    })
    .step(1.0)
    .width(Length::Fill);

    let phase_field = column![
        row![
            text(format!("PHASE · {}/{}", group.phase, group.cycle))
                .size(10)
                .color(theme::TEXT_3),
        ],
        Space::with_height(4),
        phase_slider,
    ];

    // Polyrhythm presets — different subdivision over same bar length.
    let polyrhythm_chips = polyrhythm_chip_row(group, base_grid, base_cycle);

    // Polymeter presets — same subdivision, different cycle.
    let polymeter_chips = polymeter_chip_row(group, base_grid, base_cycle);

    // Realign readout.
    let realign_bars = group.realign_bars(base_grid, base_cycle);
    let realign_text = if realign_bars <= 1 {
        "every bar".to_string()
    } else {
        format!("every {} bars", realign_bars)
    };
    let realign_banner = container(
        row![
            text("REALIGNS WITH REF")
                .size(9)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(theme::TEXT_3),
            Space::with_width(Length::Fill),
            text(realign_text).size(11).font(theme::MONO_FONT).color(color),
        ]
        .align_y(alignment::Vertical::Center),
    )
    .padding([8, 10])
    .width(Length::Fill)
    .style({
        let c = color;
        move |_theme| container::Style {
            background: Some(iced::Background::Color(Color { a: 0.08, ..c })),
            border: iced::Border {
                color: Color { a: 0.30, ..c },
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        }
    });

    rail_card(
        column![
            title,
            Space::with_height(8),
            ref_banner,
            Space::with_height(10),
            row![grid_field, Space::with_width(10), cycle_field].spacing(0),
            Space::with_height(8),
            phase_field,
            Space::with_height(10),
            section_head("POLYRHYTHM", "different subdivision, same bar"),
            Space::with_height(6),
            polyrhythm_chips,
            Space::with_height(10),
            section_head("POLYMETER", "same subdivision, different cycle"),
            Space::with_height(6),
            polymeter_chips,
            Space::with_height(10),
            realign_banner,
        ]
        .spacing(0)
        .into(),
    )
}

fn polyrhythm_chip_row<'a>(group: &'a DrumGroup, base_grid: u8, base_cycle: u32) -> Element<'a, Message> {
    let base_cycle_f = base_cycle as f32;
    let base_grid_f = base_grid as f32;
    let presets: Vec<(String, u8, u32)> = vec![
        (grid_label(base_grid).to_string(), base_grid, base_cycle),
        (
            "3 : 4".to_string(),
            3,
            ((3.0 * base_cycle_f / base_grid_f).round()) as u32,
        ),
        (
            "5 : 4".to_string(),
            5,
            ((5.0 * base_cycle_f / base_grid_f).round()) as u32,
        ),
        (
            "6 : 4".to_string(),
            6,
            ((6.0 * base_cycle_f / base_grid_f).round()) as u32,
        ),
        (
            "7 : 4".to_string(),
            7,
            ((7.0 * base_cycle_f / base_grid_f).round()) as u32,
        ),
    ];
    let group_id = group.id;
    let color = u8_color(group.color);
    let mut items: Vec<Element<'a, Message>> = Vec::new();
    for (lab, grid, cycle) in presets {
        let on = group.grid == grid && group.cycle == cycle;
        let label = lab.clone();
        let btn: Element<'a, Message> = button(text(label).size(10.5).font(theme::MONO_FONT))
            .padding([4, 9])
            .style(move |_theme, status| preset_chip_style(on, color, status))
            .on_press(Message::Compose(ComposeMessage::DrumGroups(
                DrumGroupsMessage::SetGroupMeter {
                    group_id,
                    grid,
                    cycle,
                },
            )))
            .into();
        items.push(btn);
    }
    iced::widget::Row::with_children(items).spacing(4).wrap().into()
}

fn polymeter_chip_row<'a>(group: &'a DrumGroup, base_grid: u8, base_cycle: u32) -> Element<'a, Message> {
    let presets: Vec<(String, u32)> = vec![
        (format!("{} : {}", base_cycle, base_cycle), base_cycle),
        (format!("5 : {}", base_cycle), 5),
        (format!("7 : {}", base_cycle), 7),
        (format!("9 : {}", base_cycle), 9),
        (format!("11 : {}", base_cycle), 11),
        (
            format!("{} : {}", base_cycle + 3, base_cycle),
            base_cycle + 3,
        ),
    ];
    let group_id = group.id;
    let color = u8_color(group.color);
    let mut items: Vec<Element<'a, Message>> = Vec::new();
    for (lab, cycle) in presets {
        let on = group.grid == base_grid && group.cycle == cycle;
        let label = lab.clone();
        let btn: Element<'a, Message> = button(text(label).size(10.5).font(theme::MONO_FONT))
            .padding([4, 9])
            .style(move |_theme, status| preset_chip_style(on, color, status))
            .on_press(Message::Compose(ComposeMessage::DrumGroups(
                DrumGroupsMessage::SetGroupMeter {
                    group_id,
                    grid: base_grid,
                    cycle,
                },
            )))
            .into();
        items.push(btn);
    }
    iced::widget::Row::with_children(items).spacing(4).wrap().into()
}

fn preset_chip_style(on: bool, color: Color, status: button::Status) -> button::Style {
    let bg = if on {
        Color { a: 0.25, ..color }
    } else if matches!(status, button::Status::Hovered) {
        theme::BG_3
    } else {
        Color::TRANSPARENT
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: if on { color } else { theme::TEXT_3 },
        border: iced::Border {
            color: if on { color } else { theme::LINE_2 },
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    }
}
