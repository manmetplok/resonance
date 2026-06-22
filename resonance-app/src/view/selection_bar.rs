//! Floating "Group selected" action bar (epic #36, doc #200, todo #684).
//!
//! When two or more track headers are selected in the Arrange view a small
//! pill floats above the bottom edge reading "N tracks selected · Group ⌘G".
//! The button (and the `Cmd-G` shortcut) folds the selection into a fresh
//! group. The bar is deliberately *non-modal*: it fills the surface so it can
//! centre itself, but only the pill is `opaque`, so every click outside it
//! falls through to the arrange view underneath.

use iced::widget::{button, container, opaque, row, text};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::theme;
use crate::Resonance;

pub(crate) fn view_selection_bar(r: &Resonance) -> Element<'_, Message> {
    let count = r.interaction.selected_tracks.len();

    let label = text(format!("{count} tracks selected"))
        .size(13)
        .font(theme::UI_FONT_MEDIUM)
        .color(theme::TEXT_1);

    let dot = text("\u{00b7}").size(13).color(theme::TEXT_3);

    let group_btn = button(
        text("Group  \u{2318}G")
            .size(13)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::BG_0),
    )
    .on_press(Message::Group(GroupMessage::CreateGroupFromSelection))
    .padding([7, 16])
    .style(|_theme, status| theme::primary_button_style(status));

    let pill = container(
        row![label, dot, group_btn]
            .spacing(12)
            .align_y(alignment::Vertical::Center),
    )
    .padding([8, 12])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::ACCENT_LINE,
            width: 1.0,
            radius: theme::RADIUS_XL.into(),
        },
        ..Default::default()
    });

    // Fill the surface so the pill can centre horizontally and hug the
    // bottom edge. The outer container is transparent and event-transparent
    // (no `mouse_area`); only the `opaque` pill captures clicks, leaving the
    // arrange view below fully interactive.
    container(opaque(pill))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .align_y(alignment::Vertical::Bottom)
        .padding(iced::Padding {
            top: 0.0,
            right: 0.0,
            bottom: 28.0,
            left: 0.0,
        })
        .into()
}
