//! Confirmation dialog shown when the user tries to delete a track
//! that contains audio or MIDI clips. Follows the same backdrop +
//! centered-dialog pattern as the startup and settings overlays.
use iced::widget::{button, column, container, mouse_area, opaque, row, stack, text, Space};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::theme;
use crate::Resonance;

pub(crate) fn view_confirm_delete_track_overlay<'a>(
    r: &'a Resonance,
    track_id: resonance_audio::types::TrackId,
) -> Element<'a, Message> {
    // Backdrop swallows pointer input so the DAW behind is inert while
    // the dialog is up. Clicking the dimmed area cancels deletion.
    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba(
                    0.0, 0.0, 0.0, 0.6,
                ))),
                ..Default::default()
            }),
    )
    .on_press(Message::Track(TrackMessage::CancelRemoveTrack));

    let track_name = r
        .registry
        .tracks
        .iter()
        .find(|t| t.id == track_id)
        .map(|t| t.name.as_str())
        .unwrap_or("this track");

    let title = text("Delete track?")
        .size(20)
        .font(theme::SERIF_ITALIC_FONT)
        .color(theme::TEXT_1);
    let explanation = text(format!(
        "\"{}\" contains clips. Are you sure you want to delete it?",
        track_name,
    ))
    .size(13)
    .color(theme::TEXT_2);

    let cancel_btn = button(text("Cancel").size(13).color(theme::TEXT_1))
        .on_press(Message::Track(TrackMessage::CancelRemoveTrack))
        .padding([8, 18])
        .style(|_theme, status| theme::ghost_button_style(status));

    let delete_btn = button(
        text("Delete")
            .size(13)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_1),
    )
    .on_press(Message::Track(TrackMessage::ConfirmRemoveTrack))
    .padding([8, 18])
    .style(|_theme, status| theme::destructive_button_style(status));

    let button_row = row![Space::new().width(Length::Fill), cancel_btn, delete_btn]
        .spacing(8)
        .align_y(alignment::Vertical::Center);

    let dialog_content = column![
        title,
        Space::new().height(10),
        explanation,
        Space::new().height(20),
        button_row,
    ]
    .spacing(4)
    .padding(24)
    .width(400);

    let dialog = container(dialog_content).style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE,
            width: 1.0,
            radius: theme::RADIUS_XL.into(),
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
