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
use crate::view::mixer::picks::PortChoice;
use crate::Resonance;

/// Transient state for the dialog. Lives on `Resonance::bounce_dialog`
/// while the overlay is open; the realtime bounce kicks off when the
/// user confirms with `selected_device` set.
#[derive(Debug, Clone)]
pub struct BounceDialogState {
    pub source_track_id: TrackId,
    /// Selected input device name. `None` until the user picks one.
    pub selected_device: Option<String>,
    /// Selected starting input channel (0-indexed). Defaults to 0. In
    /// stereo mode the right channel is `selected_port + 1`.
    pub selected_port: u16,
    /// Capture as mono (single channel duplicated to L/R) vs stereo
    /// (a pair of consecutive channels). Defaults to stereo because
    /// almost every external instrument returns a stereo pair.
    pub mono: bool,
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

    // Port picker — stereo (default) shows pairs "1/2", "3/4", ...
    // (the second channel of the pair is `port + 1`); mono shows
    // every channel 1..=N for capturing a single signal.
    let device_channels = selected_device.as_ref().map(|d| d.channels).unwrap_or(0);
    let port_section: Element<'_, Message> = if device_channels > 0 {
        let last_valid_index = if dialog.mono {
            device_channels
        } else {
            device_channels.saturating_sub(1)
        };
        let ports: Vec<PortChoice> = (0..last_valid_index)
            .map(|i| PortChoice {
                index: i,
                mono: dialog.mono,
            })
            .collect();
        let selected = PortChoice {
            index: dialog.selected_port.min(last_valid_index.saturating_sub(1)),
            mono: dialog.mono,
        };
        let pick = pick_list(ports, Some(selected), |choice: PortChoice| {
            Message::Track(TrackMessage::Bounce(BounceMessage::PickPort(choice.index)))
        })
        .text_size(13)
        .width(Length::Fixed(160.0));

        // Stereo / mono toggle. Two small buttons that swap the picker
        // labels between pair- and single-channel form. The active mode
        // is rendered with the accent border so the user can tell which
        // is selected at a glance — `transport_button_style` alone has
        // no disabled/selected variant.
        let toggle_btn = |label: &'static str, selected: bool, on_press: Option<Message>| {
            let mut b = button(text(label).size(12).color(if selected {
                iced::Color::WHITE
            } else {
                theme::TEXT
            }))
            .padding([4, 10])
            .style(move |_t, status| {
                let mut s = theme::transport_button_style(status);
                if selected {
                    s.border.color = theme::ACCENT;
                    s.background = Some(iced::Background::Color(iced::Color::from_rgb(
                        0.2, 0.2, 0.2,
                    )));
                }
                s
            });
            if let Some(msg) = on_press {
                b = b.on_press(msg);
            }
            b
        };
        let stereo_btn = toggle_btn(
            "Stereo",
            !dialog.mono,
            if dialog.mono {
                Some(Message::Track(TrackMessage::Bounce(BounceMessage::SetMono(false))))
            } else {
                None
            },
        );
        let mono_btn = toggle_btn(
            "Mono",
            dialog.mono,
            if !dialog.mono {
                Some(Message::Track(TrackMessage::Bounce(BounceMessage::SetMono(true))))
            } else {
                None
            },
        );

        row![
            text("Channel").size(13).color(theme::TEXT_DIM),
            pick,
            Space::with_width(8),
            stereo_btn,
            mono_btn,
        ]
        .spacing(8)
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
