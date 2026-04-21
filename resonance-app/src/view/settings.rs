//! Settings overlay (opened from the transport bar gear icon). Shows
//! project open / save / save-as buttons.
use iced::widget::{button, column, container, mouse_area, opaque, row, stack, text, Space};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::theme::{self, fa};
use crate::Resonance;

pub(crate) fn view_settings_overlay(_r: &Resonance) -> Element<'_, Message> {
    let backdrop = mouse_area(
        container(Space::new(Length::Fill, Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba(
                    0.0, 0.0, 0.0, 0.6,
                ))),
                ..Default::default()
            }),
    )
    .on_press(Message::Ui(UiMessage::CloseSettings));

    let title = text("Settings").size(20).color(theme::ACCENT);

    let section = |label: &'static str| text(label).size(11).color(theme::TEXT_DIM);

    let open_btn = wide_button(
        fa::FOLDER_OPEN,
        "Open Project...",
        Message::ProjectIo(ProjectIoMessage::OpenProject),
    );
    let save_btn = wide_button(
        fa::FLOPPY_DISK,
        "Save Project",
        Message::ProjectIo(ProjectIoMessage::SaveProject),
    );
    let save_as_btn = wide_button(
        fa::FLOPPY_DISK,
        "Save Project As...",
        Message::ProjectIo(ProjectIoMessage::SaveProjectAs),
    );

    let close_btn = button(text("Close").size(13).color(theme::TEXT))
        .on_press(Message::Ui(UiMessage::CloseSettings))
        .padding([6, 14])
        .style(|_theme, status| theme::transport_button_style(status));

    let dialog_content = column![
        title,
        Space::with_height(16),
        section("Project"),
        Space::with_height(6),
        open_btn,
        save_btn,
        save_as_btn,
        Space::with_height(20),
        row![Space::with_width(Length::Fill), close_btn,],
    ]
    .spacing(6)
    .padding(24)
    .width(420);

    let dialog = container(dialog_content).style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::PANEL)),
        border: iced::Border {
            color: theme::SEPARATOR,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    });

    let centered = container(opaque(dialog))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill);

    stack![backdrop, centered].into()
}

fn wide_button<'a>(
    icon: char,
    label: &'a str,
    on_press: Message,
) -> iced::widget::Button<'a, Message> {
    button(
        row![
            theme::icon(icon).size(14).color(theme::TEXT),
            Space::with_width(10),
            text(label).size(13).color(theme::TEXT),
        ]
        .align_y(alignment::Vertical::Center),
    )
    .on_press(on_press)
    .padding([8, 14])
    .width(Length::Fill)
    .style(|_theme, status| theme::transport_button_style(status))
}
