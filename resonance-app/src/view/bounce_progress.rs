//! Progress modal shown while a bounce-in-place run is in flight.
//! Blocks every other UI surface so the user can't disturb the engine
//! mid-render (transport, track edits, plugin tweaks all gate on
//! `Resonance::bounce_in_progress`). A Cancel button sends
//! `AudioCommand::CancelBounce`; for the offline path the engine
//! aborts cooperatively between chunks, for the realtime path it
//! pauses the transport, restores the mute snapshot, and removes the
//! freshly-added empty target track.

use iced::widget::{button, column, container, mouse_area, opaque, progress_bar, row, stack, text, Space};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::state::BounceMode;
use crate::theme;
use crate::Resonance;

pub(crate) fn view_bounce_progress_overlay<'a>(r: &'a Resonance) -> Element<'a, Message> {
    let Some(state) = r.bounce_in_progress.as_ref() else {
        return Space::new(Length::Fixed(0.0), Length::Fixed(0.0)).into();
    };

    // Backdrop: an opaque mouse_area that swallows clicks so nothing
    // behind it can be interacted with. No on_press — clicking the
    // backdrop must NOT close the modal (cancel is intentional).
    let backdrop = mouse_area(
        container(Space::new(Length::Fill, Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba(
                    0.0, 0.0, 0.0, 0.7,
                ))),
                ..Default::default()
            }),
    );

    let title_str = match state.mode {
        BounceMode::Offline => format!("Bouncing \"{}\"", state.source_name),
        BounceMode::Realtime => format!("Recording \"{}\"", state.source_name),
    };
    let title = text(title_str).size(18).color(theme::ACCENT);

    let detail = match state.mode {
        BounceMode::Offline => "Rendering through the instrument and effect chain offline.",
        BounceMode::Realtime => {
            "Playing the timeline and capturing the external instrument's audio return."
        }
    };

    let pct = (state.fraction * 100.0).round() as u32;
    let bar = progress_bar(0.0..=1.0, state.fraction).height(Length::Fixed(14.0));

    let cancel = button(text("Cancel").size(13).color(iced::Color::WHITE))
        .on_press(Message::Track(TrackMessage::Bounce(
            BounceMessage::CancelInProgress,
        )))
        .padding([8, 18])
        .style(|_t, status| theme::transport_button_style(status));

    let dialog_content = column![
        title,
        Space::with_height(8),
        text(detail).size(13).color(theme::TEXT),
        Space::with_height(16),
        bar,
        Space::with_height(6),
        text(format!("{pct}%")).size(12).color(theme::TEXT_DIM),
        Space::with_height(20),
        row![Space::with_width(Length::Fill), cancel]
            .align_y(alignment::Vertical::Center),
    ]
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
