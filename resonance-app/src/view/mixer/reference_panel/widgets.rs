//! Shared widget helpers: format chips, exclusion badge, error cards, placeholders.

use iced::widget::{button, container, row, text};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::theme::{self, fa};

pub(super) fn format_chip(label: &str) -> Element<'static, Message> {
    container(
        text(label.to_uppercase())
            .size(9)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
    )
    .padding([3, 8])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: theme::RADIUS_XS.into(),
        },
        ..Default::default()
    })
    .into()
}

/// The "not in exports" reassurance pill. A reference is a monitoring-only
/// surface, never bounced or stem-exported; this badge says so wherever the
/// panel needs to reassure the user (Empty state today, per design doc #198).
pub(super) fn exclusion_badge() -> Element<'static, Message> {
    container(
        row![
            text(fa::EYE.to_string())
                .font(theme::ICON_FONT)
                .size(9)
                .color(theme::GOOD),
            text("Not included in exports")
                .size(10)
                .font(theme::UI_FONT_MEDIUM)
                .color(theme::TEXT_2),
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center),
    )
    .padding([4, 10])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: theme::RADIUS_XS.into(),
        },
        ..Default::default()
    })
    .into()
}

/// Wrap `content` in the shared BAD-tinted card chrome (faint red wash +
/// BAD border) used by both the full-panel error body and per-entry cards.
pub(super) fn bad_card<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .width(Length::Fill)
        .padding([16, 16])
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(iced::Color {
                a: 0.08,
                ..theme::BAD
            })),
            border: iced::Border {
                color: iced::Color {
                    a: 0.5,
                    ..theme::BAD
                },
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        })
        .into()
}

pub(super) fn placeholder_card(label: &str, color: iced::Color) -> Element<'static, Message> {
    container(text(label.to_string()).size(12).color(color))
        .width(Length::Fill)
        .padding([12, 14])
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Heading row for an error card: a BAD-tinted info glyph + the title.
pub(super) fn error_heading(title: &str) -> Element<'static, Message> {
    row![
        text(fa::CIRCLE_INFO.to_string())
            .font(theme::ICON_FONT)
            .size(12)
            .color(theme::BAD),
        text(title.to_string())
            .size(13)
            .font(theme::UI_FONT_MEDIUM)
            .color(theme::TEXT_1),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center)
    .into()
}

/// The two error actions: a `dismiss` button (whose message drops the
/// failed entry or clears the notice) and a "Choose another…" button that
/// re-opens the file picker.
pub(super) fn error_actions(dismiss_msg: Message) -> Element<'static, Message> {
    let dismiss = button(text("Dismiss").size(12).font(theme::UI_FONT_MEDIUM))
        .on_press(dismiss_msg)
        .padding([7, 14])
        .style(|_theme, status| theme::small_button_style(status));

    let choose = button(
        text("Choose another\u{2026}")
            .size(12)
            .font(theme::UI_FONT_MEDIUM),
    )
    .on_press(Message::Reference(
        crate::reference::ReferenceMessage::PickFile,
    ))
    .padding([7, 14])
    .style(|_theme, status| theme::small_button_style(status));

    row![dismiss, choose].spacing(8).into()
}
