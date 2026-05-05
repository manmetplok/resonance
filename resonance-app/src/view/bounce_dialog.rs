//! "Bounce in place" dialog shown when the user invokes bounce on an
//! instrument track driven by an external MIDI device. Asks the user to
//! pick the audio input + channel that the external instrument's return
//! is wired into, then triggers the realtime bounce.
//!
//! Same backdrop + centered-dialog pattern as `confirm_delete_track`.
use iced::widget::{button, column, container, mouse_area, opaque, pick_list, row, stack, text, Space};
use iced::{alignment, Element, Length};

use resonance_audio::types::{InputDeviceInfo, TrackId};

use crate::message::*;
use crate::theme;
use crate::Resonance;

/// Transient state for the dialog. Lives on `Resonance::bounce_dialog`
/// while the overlay is open; the realtime bounce kicks off when the
/// user confirms with `selected_device` set.
#[derive(Debug, Clone)]
pub struct BounceDialogState {
    pub source_track_id: TrackId,
    /// Selected input device name. `None` until the user picks one.
    pub selected_device: Option<String>,
    /// Selected starting input channel (0-indexed). Defaults to 0.
    pub selected_port: u16,
}

pub(crate) fn view_bounce_dialog_overlay<'a>(r: &'a Resonance) -> Element<'a, Message> {
    let Some(dialog) = r.bounce_dialog.as_ref() else {
        return Space::new(Length::Fixed(0.0), Length::Fixed(0.0)).into();
    };

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
    .on_press(Message::Track(TrackMessage::Bounce(BounceMessage::Cancel)));

    let track_name = r
        .registry
        .tracks
        .iter()
        .find(|t| t.id == dialog.source_track_id)
        .map(|t| t.name.as_str())
        .unwrap_or("track");

    let title = text("Bounce in place").size(20).color(theme::ACCENT);
    let explanation = text(format!(
        "\"{track_name}\" plays an external MIDI instrument. Pick the audio input that's listening to that instrument's return — the bounce will record from there while the timeline plays."
    ))
    .size(13)
    .color(theme::TEXT);

    // Device picker.
    let selected_device = dialog
        .selected_device
        .as_ref()
        .and_then(|name| r.input_devices.iter().find(|d| &d.name == name))
        .cloned();
    let device_picker = pick_list(
        r.input_devices.clone(),
        selected_device.clone(),
        |device: InputDeviceInfo| {
            Message::Track(TrackMessage::Bounce(BounceMessage::PickDevice(Some(device.name))))
        },
    )
    .placeholder("Select input device")
    .text_size(13)
    .width(Length::Fill);

    // Port picker — list every channel 1..=N for the selected device.
    let device_channels = selected_device.as_ref().map(|d| d.channels).unwrap_or(0);
    let port_section: Element<'_, Message> = if device_channels > 0 {
        let ports: Vec<u16> = (0..device_channels).collect();
        let pick = pick_list(
            ports,
            Some(dialog.selected_port),
            |p: u16| Message::Track(TrackMessage::Bounce(BounceMessage::PickPort(p))),
        )
        .text_size(13)
        .width(Length::Fixed(120.0));
        row![text("Channel").size(13).color(theme::TEXT_DIM), pick]
            .spacing(10)
            .align_y(alignment::Vertical::Center)
            .into()
    } else {
        text("Pick a device first").size(11).color(theme::TEXT_DIM).into()
    };

    let cancel_btn = button(text("Cancel").size(13).color(theme::TEXT))
        .on_press(Message::Track(TrackMessage::Bounce(BounceMessage::Cancel)))
        .padding([8, 18])
        .style(|_theme, status| theme::transport_button_style(status));

    let mut confirm_btn = button(text("Bounce").size(13).color(iced::Color::WHITE))
        .padding([8, 18])
        .style(|_theme, status| theme::transport_button_style(status));
    if dialog.selected_device.is_some() {
        confirm_btn = confirm_btn.on_press(Message::Track(TrackMessage::Bounce(BounceMessage::Confirm)));
    }

    let button_row = row![Space::with_width(Length::Fill), cancel_btn, confirm_btn]
        .spacing(8)
        .align_y(alignment::Vertical::Center);

    let dialog_content = column![
        title,
        Space::with_height(10),
        explanation,
        Space::with_height(16),
        text("Audio input").size(12).color(theme::TEXT_DIM),
        device_picker,
        Space::with_height(8),
        port_section,
        Space::with_height(20),
        button_row,
    ]
    .spacing(4)
    .padding(24)
    .width(460);

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
