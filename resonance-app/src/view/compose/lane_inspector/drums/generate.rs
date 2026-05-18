//! Generate panel — primary "Generate <group>" button plus the
//! all-groups regenerate fallback and the seed readout.

use iced::widget::{button, column, row, text, Space};
use iced::{alignment, Color, Element, Length};

use crate::compose::drumroll::DrumGroup;
use crate::compose::messages::DrumGroupsMessage;
use crate::compose::ComposeMessage;
use crate::message::Message;
use crate::theme;

use super::common::{rail_card, u8_color};

pub(super) fn generate_panel<'a>(group: &'a DrumGroup) -> Element<'a, Message> {
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
