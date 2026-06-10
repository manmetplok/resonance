//! Group separator inserted between SECTION lanes (scale + chords) and
//! TRACKS lanes (synth + drums) in the Compose layout. Reads as a slim,
//! dim banner — collapse caret, colored bullet, letter-spaced uppercase
//! tag, dim subtitle, and an optional trailing count on the right.
//! Clicking anywhere on the banner folds / unfolds the lanes under it.

use iced::widget::{button, row, text, Space};
use iced::{alignment, Color, Element, Length};

use crate::compose::{ComposeMessage, WorkspaceGroup};
use crate::message::Message;
use crate::theme;
use crate::view::controls::collapse_caret;

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

    fn workspace_group(self) -> WorkspaceGroup {
        match self {
            GroupKind::Section => WorkspaceGroup::Section,
            GroupKind::Tracks => WorkspaceGroup::Tracks,
        }
    }
}

/// Group separator inserted between SECTION lanes (scale + chords) and
/// TRACKS lanes (synth + drums). Reads as a slim, dim banner — collapse
/// caret, colored bullet, letter-spaced uppercase tag, dim subtitle,
/// and an optional trailing count on the right. The whole banner is a
/// click target that toggles the lanes under it (same caret pattern as
/// the Arrange global shelf header strip).
pub(super) fn group_header<'a>(
    tag: impl Into<String>,
    sub: impl Into<String>,
    count: impl Into<String>,
    kind: GroupKind,
    expanded: bool,
) -> Element<'a, Message> {
    let dot = iced::widget::container(
        Space::new().width(Length::Fixed(6.0)).height(Length::Fixed(6.0)),
    )
    .style(move |_theme| iced::widget::container::Style {
        background: Some(iced::Background::Color(kind.dot())),
        border: iced::Border {
            color: kind.dot(),
            width: 0.0,
            radius: 3.0.into(),
        },
        ..Default::default()
    });
    let tag_text = text(tag.into())
        .size(10)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(kind.accent());
    let sub_text = text(sub.into()).size(11).color(theme::TEXT_3);
    let mut head = row![collapse_caret(expanded), dot, tag_text, sub_text]
        .spacing(8)
        .align_y(alignment::Vertical::Center)
        .width(Length::Fill);
    let count_str = count.into();
    if !count_str.is_empty() {
        head = head.push(Space::new().width(Length::Fill));
        head = head.push(
            text(count_str)
                .size(10)
                .font(theme::MONO_FONT)
                .color(theme::TEXT_3),
        );
    }
    button(head)
        .padding([10, 14])
        .width(Length::Fill)
        .style(|_theme, status| {
            let bg = match status {
                button::Status::Hovered | button::Status::Pressed => theme::BG_2,
                _ => theme::BG_1,
            };
            button::Style {
                background: Some(iced::Background::Color(bg)),
                text_color: theme::TEXT_1,
                border: iced::Border {
                    color: theme::LINE_2,
                    width: 0.0,
                    radius: theme::RADIUS_XS.into(),
                },
                ..Default::default()
            }
        })
        .on_press(Message::Compose(ComposeMessage::ToggleWorkspaceGroup(
            kind.workspace_group(),
        )))
        .into()
}
