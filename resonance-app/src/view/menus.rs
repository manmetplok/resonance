//! Overlay menus (add-track popover, etc.)
use iced::widget::{button, column, container, mouse_area, opaque, row, stack, text, Space};
use iced::{alignment, Color, Element, Length};

use crate::message::*;
use crate::theme::{self, fa};
use crate::Resonance;

pub(crate) fn view_add_track_menu(_r: &Resonance) -> Element<'_, Message> {
    let backdrop = mouse_area(
        container(Space::new(Length::Fill, Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba(
                    0.0, 0.0, 0.0, 0.3,
                ))),
                ..Default::default()
            }),
    )
    .on_press(Message::Ui(UiMessage::CloseAddTrackMenu));

    let audio_btn = button(
        row![
            theme::icon(fa::MICROPHONE).size(14).color(theme::TEXT),
            Space::with_width(8),
            text("Audio").size(13).color(theme::TEXT),
        ]
        .align_y(alignment::Vertical::Center),
    )
    .on_press(Message::Track(TrackMessage::AddTrack))
    .width(Length::Fill)
    .padding([6, 10])
    .style(|_theme, status| theme::transport_button_style(status));

    let inst_btn = button(
        row![
            theme::icon(fa::MUSIC)
                .size(14)
                .color(Color::from_rgb(0.3, 0.75, 0.8)),
            Space::with_width(8),
            text("Instrument")
                .size(13)
                .color(Color::from_rgb(0.3, 0.75, 0.8)),
        ]
        .align_y(alignment::Vertical::Center),
    )
    .on_press(Message::Track(TrackMessage::AddInstrumentTrack))
    .width(Length::Fill)
    .padding([6, 10])
    .style(|_theme, status| theme::transport_button_style(status));

    let menu_content = column![
        text("Add Track").size(11).color(theme::TEXT_DIM),
        Space::with_height(4),
        audio_btn,
        inst_btn,
    ]
    .spacing(2)
    .padding(8)
    .width(180);

    let menu = container(opaque(menu_content)).style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::PANEL)),
        border: iced::Border {
            color: theme::SEPARATOR,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..Default::default()
    });

    // Position the popup just below the "+" button in the track header ruler area.
    // The + button sits near the left edge of the track header column (~12px in)
    // directly below the transport bar (transport height ~48px + ruler height 30px).
    let top_pad = 48.0 + theme::RULER_HEIGHT + 2.0;
    let positioned = container(menu)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Horizontal::Left)
        .align_y(alignment::Vertical::Top)
        .padding(iced::Padding {
            top: top_pad,
            right: 0.0,
            bottom: 0.0,
            left: 12.0,
        });

    stack![backdrop, positioned].into()
}
