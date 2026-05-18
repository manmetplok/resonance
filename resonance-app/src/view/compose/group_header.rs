//! Group separator inserted between SECTION lanes (scale + chords) and
//! TRACKS lanes (synth + drums) in the Compose layout. Reads as a slim,
//! dim banner — colored bullet, letter-spaced uppercase tag, dim
//! subtitle, and an optional trailing count on the right.

use iced::widget::{container, row, text, Space};
use iced::{alignment, Color, Element, Length};

use crate::message::Message;
use crate::theme;

/// Which color family a group header reads as. Section-level groups
/// (chord lane / scale) use the lavender accent; track-level groups (synth
/// + drum lanes) use the warm amber accent.
#[derive(Debug, Clone, Copy)]
pub(super) enum GroupKind {
    Section,
    Tracks,
}

impl GroupKind {
    fn accent(self) -> Color {
        match self {
            GroupKind::Section => theme::ACCENT_SOFT,
            GroupKind::Tracks => theme::WARM,
        }
    }

    fn dot(self) -> Color {
        match self {
            GroupKind::Section => theme::ACCENT,
            GroupKind::Tracks => theme::WARM,
        }
    }
}

/// Group separator inserted between SECTION lanes (scale + chords) and
/// TRACKS lanes (synth + drums). Reads as a slim, dim banner — colored
/// bullet, letter-spaced uppercase tag, dim subtitle, and an optional
/// trailing count on the right.
pub(super) fn group_header<'a>(
    tag: impl Into<String>,
    sub: impl Into<String>,
    count: impl Into<String>,
    kind: GroupKind,
) -> Element<'a, Message> {
    let dot = container(Space::new(Length::Fixed(6.0), Length::Fixed(6.0))).style(
        move |_theme| container::Style {
            background: Some(iced::Background::Color(kind.dot())),
            border: iced::Border {
                color: kind.dot(),
                width: 0.0,
                radius: 3.0.into(),
            },
            ..Default::default()
        },
    );
    let tag_text = text(tag.into())
        .size(10)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(kind.accent());
    let sub_text = text(sub.into()).size(11).color(theme::TEXT_3);
    let mut head = row![dot, tag_text, sub_text]
        .spacing(8)
        .align_y(alignment::Vertical::Center);
    let count_str = count.into();
    if !count_str.is_empty() {
        head = head.push(Space::with_width(Length::Fill));
        head = head.push(
            text(count_str)
                .size(10)
                .font(theme::MONO_FONT)
                .color(theme::TEXT_3),
        );
    }
    container(head)
        .padding([8, 14])
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
