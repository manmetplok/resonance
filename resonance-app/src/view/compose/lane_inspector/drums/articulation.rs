//! Articulation mix panel — per-pad weight sliders with a stacked-bar
//! readout above.

use iced::widget::{column, container, row, slider, text, Space};
use iced::{alignment, Color, Element, Length};

use crate::compose::drumroll::DrumGroup;
use crate::compose::messages::DrumGroupsMessage;
use crate::compose::ComposeMessage;
use crate::message::Message;
use crate::theme;

use super::common::{rail_card, rail_dot, u8_color};

pub(super) fn articulation_mix_panel<'a>(group: &'a DrumGroup) -> Element<'a, Message> {
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
