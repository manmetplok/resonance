//! Drum-lane right-rail inspector — group selector, meter (grid/cycle/
//! phase + polyrhythm/polymeter presets), articulation mix, rhythm
//! settings, generate.

use iced::widget::{button, column, container, pick_list, row, slider, text, Space};
use iced::{alignment, Color, Element, Length};

use crate::compose::drumroll::{grid_label, DrumGroup};
use crate::compose::messages::DrumGroupsMessage;
use crate::compose::{ComposeMessage, DrumrollViewState, SectionDefinitionState};
use crate::message::Message;
use crate::state::TrackState;
use crate::theme;

const BEATS_PER_BAR: u32 = 4;

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
        Space::with_height(12),
        meter_panel(group, base_grid, base_cycle),
        Space::with_height(12),
        articulation_mix_panel(group),
        Space::with_height(12),
        rhythm_panel(group),
        Space::with_height(12),
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
        Space::with_width(Length::Fill),
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
        let dot = container(Space::new(Length::Fixed(5.0), Length::Fixed(5.0)))
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
            Space::with_height(8),
            tab_row,
            Space::with_height(6),
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

// ===========================================================================
// Meter panel
// ===========================================================================

fn meter_panel<'a>(group: &'a DrumGroup, base_grid: u8, base_cycle: u32) -> Element<'a, Message> {
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

// ===========================================================================
// Articulation mix
// ===========================================================================

fn articulation_mix_panel<'a>(group: &'a DrumGroup) -> Element<'a, Message> {
    let color = u8_color(group.color);
    let title = row![
        rail_dot(color),
        text(format!("{} · articulation mix", group.name))
            .size(12)
            .color(theme::WARM),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);

    let total = group.total_weight().max(1) as f32;
    let mut stack = iced::widget::Row::new().spacing(0);
    for (i, p) in group.pads.iter().enumerate() {
        let pct = (p.weight as f32) / total;
        // Build a colored segment.
        let alpha = 0.40 + (i as f32 * 0.16).clamp(0.0, 0.55);
        let seg_color = Color { a: alpha, ..color };
        let seg = container(Space::with_height(Length::Fixed(10.0)))
            .width(Length::FillPortion((pct * 1000.0) as u16))
            .height(Length::Fixed(10.0))
            .style(move |_theme| container::Style {
                background: Some(iced::Background::Color(seg_color)),
                ..Default::default()
            });
        stack = stack.push(seg);
    }
    let stack_bar = container(stack)
        .width(Length::Fill)
        .padding(0)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_XS.into(),
            },
            ..Default::default()
        });

    let mut rows: Vec<Element<'a, Message>> = Vec::new();
    for (i, p) in group.pads.iter().enumerate() {
        let group_id = group.id;
        let pct = (p.weight as f32) / total;
        let pct_pct = (pct * 100.0).round() as i32;

        let name = text(p.name.clone())
            .size(11)
            .color(theme::TEXT_2)
            .width(Length::Fixed(82.0));
        let s = slider(0.0..=100.0, p.weight as f32, move |v| {
            Message::Compose(ComposeMessage::DrumGroups(
                DrumGroupsMessage::SetPadWeight {
                    group_id,
                    pad_index: i,
                    weight: v.round() as u32,
                },
            ))
        })
        .step(1.0)
        .width(Length::Fill);
        let pct_text = text(format!("{}%", pct_pct))
            .size(10)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_3)
            .width(Length::Fixed(36.0));
        rows.push(
            row![name, s, pct_text]
                .spacing(10)
                .align_y(alignment::Vertical::Center)
                .into(),
        );
    }

    rail_card(
        column![
            title,
            Space::with_height(8),
            stack_bar,
            Space::with_height(8),
            column(rows).spacing(8),
        ]
        .spacing(0)
        .into(),
    )
}

// ===========================================================================
// Rhythm settings
// ===========================================================================

fn rhythm_panel<'a>(group: &'a DrumGroup) -> Element<'a, Message> {
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
            Space::with_height(8),
            field("Density", group.density, density),
            Space::with_height(6),
            field("Swing", group.swing, swing),
            Space::with_height(6),
            field("Accent", group.accent, accent),
            Space::with_height(6),
            field("Humanize", group.humanize, humanize),
            Space::with_height(6),
            field("Fills (last bar)", group.fills, fills),
            Space::with_height(6),
            text(group.style.clone()).size(10).color(theme::TEXT_4),
        ]
        .spacing(0)
        .into(),
    )
}

// ===========================================================================
// Generate panel
// ===========================================================================

fn generate_panel<'a>(group: &'a DrumGroup) -> Element<'a, Message> {
    let color = u8_color(group.color);
    let id = group.id;

    let generate_btn = button(
        text(format!("Generate {}", group.name))
            .size(12)
            .color(Color::from_rgb(0.05, 0.04, 0.12))
            .align_x(alignment::Horizontal::Center),
    )
    .padding([8, 14])
    .width(Length::Fill)
    .style(move |_theme, status| {
        let bg = match status {
            button::Status::Hovered => Color {
                r: (color.r * 1.1).min(1.0),
                g: (color.g * 1.1).min(1.0),
                b: (color.b * 1.1).min(1.0),
                a: 1.0,
            },
            button::Status::Pressed => Color {
                r: color.r * 0.85,
                g: color.g * 0.85,
                b: color.b * 0.85,
                a: 1.0,
            },
            _ => color,
        };
        button::Style {
            background: Some(iced::Background::Color(bg)),
            text_color: Color::from_rgb(0.05, 0.04, 0.12),
            border: iced::Border {
                color,
                width: 0.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        }
    })
    .on_press(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::GenerateGroup { group_id: id },
    )));
    let reroll = button(text("↻").size(13).color(theme::TEXT_1))
        .padding([8, 14])
        .style(|_theme, status| theme::ghost_button_style(status))
        .on_press(Message::Compose(ComposeMessage::DrumGroups(
            DrumGroupsMessage::GenerateGroup { group_id: id },
        )));
    let regen_all = button(
        text("Regenerate all groups")
            .size(11)
            .color(theme::TEXT_2)
            .align_x(alignment::Horizontal::Center),
    )
    .padding([7, 12])
    .width(Length::Fill)
    .style(|_theme, status| theme::ghost_button_style(status))
    .on_press(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::GenerateAllGroups,
    )));

    let seed_line = text(format!("seed · 0x{:016X}", group.seed))
        .size(10)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_3);

    rail_card(
        column![
            row![generate_btn, Space::with_width(8), reroll].spacing(0),
            Space::with_height(8),
            regen_all,
            Space::with_height(4),
            seed_line,
        ]
        .spacing(0)
        .into(),
    )
}

// ===========================================================================
// Shared helpers
// ===========================================================================

fn rail_card<'a>(content: Element<'a, Message>) -> Element<'a, Message> {
    container(content)
        .padding([12, 12])
        .width(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_LG.into(),
            },
            ..Default::default()
        })
        .into()
}

fn rail_dot(color: Color) -> Element<'static, Message> {
    container(Space::new(Length::Fixed(6.0), Length::Fixed(6.0)))
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(color)),
            border: iced::Border {
                color,
                width: 0.0,
                radius: 999.0.into(),
            },
            ..Default::default()
        })
        .into()
}

fn section_head<'a>(title: &str, hint: &str) -> Element<'a, Message> {
    row![
        text(title.to_string())
            .size(10)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
        Space::with_width(Length::Fill),
        text(hint.to_string()).size(10).color(theme::TEXT_4),
    ]
    .align_y(alignment::Vertical::Center)
    .into()
}

fn field<'a>(label: &str, value: f32, slider_el: iced::widget::Slider<'a, f32, Message>) -> Element<'a, Message> {
    column![
        row![
            text(label.to_string()).size(10).color(theme::TEXT_3),
            Space::with_width(Length::Fill),
            text(format!("{:.2}", value))
                .size(10)
                .font(theme::MONO_FONT)
                .color(theme::TEXT_2),
        ],
        Space::with_height(4),
        slider_el,
    ]
    .spacing(0)
    .into()
}

fn u8_color(rgb: [u8; 3]) -> Color {
    Color::from_rgb(
        rgb[0] as f32 / 255.0,
        rgb[1] as f32 / 255.0,
        rgb[2] as f32 / 255.0,
    )
}
