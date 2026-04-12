//! Confirmation dialog shown when the user tries to close the window
//! while the project has unsaved changes. Follows the same backdrop +
//! centered-dialog pattern as the confirm-delete-track overlay.
use iced::widget::{button, column, container, mouse_area, opaque, row, stack, text, Space};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::theme;
use crate::Resonance;

pub(crate) fn view_confirm_quit_overlay<'a>(
    _r: &'a Resonance,
) -> Element<'a, Message> {
    // Backdrop swallows pointer input so the DAW behind is inert while
    // the dialog is up. Clicking the dimmed area cancels the quit.
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
    .on_press(Message::Ui(UiMessage::CancelQuit));

    let title = text("Unsaved Changes").size(20).color(theme::ACCENT);
    let explanation = text(
        "You have unsaved changes. What would you like to do?",
    )
    .size(13)
    .color(theme::TEXT);

    let cancel_btn = button(text("Cancel").size(13).color(theme::TEXT))
        .on_press(Message::Ui(UiMessage::CancelQuit))
        .padding([8, 18])
        .style(|_theme, status| theme::transport_button_style(status));

    let discard_btn = button(text("Discard & Quit").size(13).color(iced::Color::WHITE))
        .on_press(Message::Ui(UiMessage::ConfirmDiscardAndQuit))
        .padding([8, 18])
        .style(|_theme, status| theme::destructive_button_style(status));

    let save_btn = button(text("Save & Quit").size(13).color(iced::Color::WHITE))
        .on_press(Message::Ui(UiMessage::ConfirmSaveAndQuit))
        .padding([8, 18])
        .style(|_theme, status| theme::accent_button_style(status));

    let button_row = row![Space::with_width(Length::Fill), cancel_btn, discard_btn, save_btn]
        .spacing(8)
        .align_y(alignment::Vertical::Center);

    let dialog_content = column![
        title,
        Space::with_height(10),
        explanation,
        Space::with_height(20),
        button_row,
    ]
    .spacing(4)
    .padding(24)
    .width(440);

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
