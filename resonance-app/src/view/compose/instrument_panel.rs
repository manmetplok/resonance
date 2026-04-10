use iced::widget::{button, column, container, pick_list, row, text, text_input, Space};
use iced::{alignment, Element, Font, Length};

use crate::compose::ComposeMessage;
use crate::message::Message;
use crate::state::{InstrumentIcon, InstrumentType, TrackState};
use crate::theme;

pub const PANEL_WIDTH: f32 = 240.0;

/// Right-side panel body that replaces the scale picker when an instrument
/// track is selected in the Compose tab. Shows the icon + name preview, an
/// editable name field, and pickers for type (Synth/Drum) and icon.
pub fn view<'a>(track: &'a TrackState) -> Element<'a, Message> {
    let track_id = track.id;

    let heading = text("Instrument").size(13).color(theme::ACCENT);

    let close_btn = button(text("Done").size(12))
        .on_press(Message::Compose(ComposeMessage::ClearInstrumentDetails))
        .padding([4, 10])
        .style(|_theme, status| theme::transport_button_style(status));

    let preview = row![
        text(track.instrument_icon.glyph().to_string())
            .size(32)
            .font(theme::ICON_FONT)
            .color(theme::ACCENT),
        Space::with_width(12),
        text(track.name.clone())
            .size(16)
            .color(theme::TEXT)
            .font(Font::DEFAULT)
            .wrapping(iced::widget::text::Wrapping::None),
    ]
    .spacing(0)
    .align_y(alignment::Vertical::Center);

    let name_input = text_input("Name", &track.name)
        .on_input(move |s| Message::SetTrackName(track_id, s))
        .size(12)
        .padding([4, 6])
        .width(Length::Fill);

    let type_picker = pick_list(
        InstrumentType::ALL.to_vec(),
        Some(track.instrument_type),
        move |ty| Message::SetInstrumentType(track_id, ty),
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let icon_picker = pick_list(
        InstrumentIcon::ALL.to_vec(),
        Some(track.instrument_icon),
        move |icon| Message::SetInstrumentIcon(track_id, icon),
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let content = column![
        row![heading, Space::with_width(Length::Fill), close_btn]
            .align_y(alignment::Vertical::Center),
        Space::with_height(10),
        preview,
        Space::with_height(14),
        text("Name").size(10).color(theme::TEXT_DIM),
        name_input,
        Space::with_height(8),
        text("Type").size(10).color(theme::TEXT_DIM),
        type_picker,
        Space::with_height(8),
        text("Icon").size(10).color(theme::TEXT_DIM),
        icon_picker,
    ]
    .spacing(4)
    .padding(12);

    container(content)
        .width(Length::Fixed(PANEL_WIDTH))
        .height(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::PANEL)),
            border: iced::Border {
                color: theme::SEPARATOR,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}
