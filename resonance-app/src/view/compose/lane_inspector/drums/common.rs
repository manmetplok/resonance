//! Shared helpers for the drum-lane right-rail inspector panels.

use iced::widget::{column, container, row, text, Space};
use iced::{alignment, Color, Element, Length};

use crate::message::Message;
use crate::theme;

pub(super) const BEATS_PER_BAR: u32 = 4;

pub(super) fn rail_card<'a>(content: Element<'a, Message>) -> Element<'a, Message> {
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

pub(super) fn rail_dot(color: Color) -> Element<'static, Message> {
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

pub(super) fn section_head<'a>(title: &str, hint: &str) -> Element<'a, Message> {
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

pub(super) fn field<'a>(label: &str, value: f32, slider_el: iced::widget::Slider<'a, f32, Message>) -> Element<'a, Message> {
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

pub(super) fn u8_color(rgb: [u8; 3]) -> Color {
    Color::from_rgb(
        rgb[0] as f32 / 255.0,
        rgb[1] as f32 / 255.0,
        rgb[2] as f32 / 255.0,
    )
}
