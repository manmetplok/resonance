//! Pattern picker strip above the drumroll lane.
//!
//! Mirrors the conventions established by the lane-inspector group chips
//! (see `lane_inspector/drums/mod.rs::group_selector`): each pattern
//! shows up as a chip with a colour dot and group count; the assigned
//! pattern carries the warm-tint border. Adjacent controls let the user
//! rename/duplicate/delete the assigned pattern, plus an "+ Add" button.
//!
//! All colours, radii, and font tokens come from `theme.rs` per
//! `ux-guidelines.md`. Hover/pressed states are derived from the
//! pattern's accent colour for the active chip and from the base
//! `ghost_button_style` for inactive chips.

use iced::widget::{button, container, row, text, text_input, Space};
use iced::{alignment, Color, Element, Length};

use crate::compose::messages::DrumGroupsMessage;
use crate::compose::{ComposeMessage, SectionDefinitionState};
use crate::message::Message;
use crate::theme;
use crate::Resonance;

use super::super::tracks::NAME_COLUMN_WIDTH;

/// Row height in the picker strip. Matches the section/tracks group
/// headers above so visually the picker sits in the same band.
const STRIP_HEIGHT: f32 = 32.0;
const LABEL_GUTTER: f32 = 8.0;

/// Build the picker strip. `width` is the fixed workspace width — the
/// same value `chord_lane` and `tracks` use so the picker aligns with
/// the rest of the lane stack.
pub fn pattern_picker<'a>(
    app: &'a Resonance,
    definition: &'a SectionDefinitionState,
    width: f32,
) -> Element<'a, Message> {
    // Side panel — "PATTERN · {section}" tag, matching the lane-side
    // header treatment used by the chord / vocal / drum lanes. The
    // workspace_width subtracts NAME_COLUMN_WIDTH internally; we mirror
    // that by reserving a left column for the tag.
    let side = container(
        column_label("PATTERN", &definition.name),
    )
    .width(Length::Fixed(NAME_COLUMN_WIDTH))
    .height(Length::Fixed(STRIP_HEIGHT))
    .padding([0, 12])
    .align_y(alignment::Vertical::Center)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_1)),
        ..Default::default()
    });

    let assigned_id = app
        .compose
        .pattern_for_definition(definition)
        .map(|p| p.id);
    let renaming_id = app.compose.drumroll.renaming_pattern_id;

    let mut chips: Vec<Element<'a, Message>> = Vec::new();
    for p in &app.compose.drum_patterns {
        let active = Some(p.id) == assigned_id;
        let chip = if renaming_id == Some(p.id) {
            rename_chip(p.id, &app.compose.drumroll.renaming_pattern_text)
        } else {
            pattern_chip(definition.id, p.id, &p.name, p.color, p.group_count(), active)
        };
        chips.push(chip);
    }

    // Trailing actions — duplicate / rename / delete / add. Duplicate &
    // rename act on the *assigned* pattern (so the user clicks the chip
    // they care about, then hits the action); they're disabled when no
    // pattern is assigned.
    let actions = action_buttons(definition.id, assigned_id);

    let chip_row = iced::widget::Row::with_children(chips)
        .spacing(6)
        .align_y(alignment::Vertical::Center)
        .wrap();

    let body = row![chip_row, Space::new().width(Length::Fill), actions]
        .spacing(8)
        .align_y(alignment::Vertical::Center)
        .height(Length::Fixed(STRIP_HEIGHT));

    let right = container(body)
        .padding([0, 12])
        .width(Length::Fill)
        .height(Length::Fixed(STRIP_HEIGHT))
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        });

    container(row![side, right].spacing(0))
        .width(Length::Fixed(width))
        .height(Length::Fixed(STRIP_HEIGHT))
        .into()
}

/// Two-line tag rendered in the lane's side column ("PATTERN" / section
/// name). Matches `lane_side::draw`'s aesthetic but as a real Iced
/// element so the picker strip can sit next to interactive chips
/// without a Canvas wrapping the lot.
fn column_label<'a>(kicker: &str, name: &str) -> Element<'a, Message> {
    let kicker = kicker.to_string();
    let name = name.to_string();
    iced::widget::column![
        text(kicker)
            .size(9)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
        text(name).size(12).color(theme::TEXT_1),
    ]
    .spacing(0)
    .into()
}

/// One picker chip — colour dot, name, group count badge. Active state
/// borrows the lane-inspector `group_tab_style` palette so picker and
/// inspector read as the same vocabulary.
fn pattern_chip<'a>(
    definition_id: u64,
    pattern_id: u64,
    name: &str,
    color: [u8; 3],
    group_count: usize,
    active: bool,
) -> Element<'a, Message> {
    let color = u8_color(color);
    let dot = container(Space::new().width(Length::Fixed(6.0)).height(Length::Fixed(6.0)))
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(color)),
            border: iced::Border {
                color,
                width: 0.0,
                radius: 999.0.into(),
            },
            ..Default::default()
        });
    let label = text(name.to_string()).size(11).color(if active {
        theme::TEXT_1
    } else {
        theme::TEXT_2
    });
    let count = text(format!("{}", group_count))
        .size(9)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_4);
    let inner = row![dot, label, count]
        .spacing(6)
        .align_y(alignment::Vertical::Center);

    let chip_color = color;
    button(inner)
        .padding([5, LABEL_GUTTER as u16])
        .style(move |_theme, status| chip_style(active, chip_color, status))
        .on_press(Message::Compose(ComposeMessage::DrumGroups(
            DrumGroupsMessage::AssignPattern {
                definition_id,
                pattern_id: Some(pattern_id),
            },
        )))
        .into()
}

fn chip_style(active: bool, color: Color, status: button::Status) -> button::Style {
    let bg = if active {
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
            color: if active { color } else { theme::LINE_2 },
            width: 1.0,
            radius: theme::RADIUS_SM.into(),
        },
        ..Default::default()
    }
}

/// Replace a chip with a single-line text input while a pattern is
/// being renamed.
fn rename_chip<'a>(pattern_id: u64, current_text: &str) -> Element<'a, Message> {
    let input = text_input("Pattern name", current_text)
        .size(11)
        .width(Length::Fixed(160.0))
        .on_input(|s| {
            Message::Compose(ComposeMessage::DrumGroups(
                DrumGroupsMessage::UpdateRenamePatternText(s),
            ))
        })
        .on_submit(Message::Compose(ComposeMessage::DrumGroups(
            DrumGroupsMessage::CommitRenamePattern,
        )));
    let _ = pattern_id;
    container(input)
        .padding([2, 4])
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_3)),
            border: iced::Border {
                color: theme::WARM,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Trailing action cluster — duplicate, rename, delete, add. Each
/// button is a ghost-style icon-text combo so the strip stays compact.
fn action_buttons<'a>(
    definition_id: u64,
    assigned_id: Option<u64>,
) -> Element<'a, Message> {
    let rename = ghost_action_button(
        "Rename",
        assigned_id.map(|id| {
            Message::Compose(ComposeMessage::DrumGroups(
                DrumGroupsMessage::BeginRenamePattern { pattern_id: id },
            ))
        }),
    );
    let dup = ghost_action_button(
        "Duplicate",
        assigned_id.map(|id| {
            Message::Compose(ComposeMessage::DrumGroups(
                DrumGroupsMessage::DuplicatePattern { pattern_id: id },
            ))
        }),
    );
    let del = ghost_action_button(
        "Delete",
        assigned_id.map(|id| {
            Message::Compose(ComposeMessage::DrumGroups(
                DrumGroupsMessage::DeletePattern { pattern_id: id },
            ))
        }),
    );
    let add = ghost_action_button(
        "+ Add",
        Some(Message::Compose(ComposeMessage::DrumGroups(
            DrumGroupsMessage::AddPattern,
        ))),
    );
    // Suppress the unused warning when no pattern is assigned.
    let _ = definition_id;
    row![rename, dup, del, add]
        .spacing(4)
        .align_y(alignment::Vertical::Center)
        .into()
}

fn ghost_action_button<'a>(
    label: &'static str,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    let body = text(label).size(10).color(theme::TEXT_2);
    let mut btn = button(body)
        .padding([4, 8])
        .style(|_theme, status| theme::ghost_button_style(status));
    if let Some(msg) = on_press {
        btn = btn.on_press(msg);
    }
    btn.into()
}

fn u8_color(rgb: [u8; 3]) -> Color {
    Color::from_rgb(
        rgb[0] as f32 / 255.0,
        rgb[1] as f32 / 255.0,
        rgb[2] as f32 / 255.0,
    )
}
