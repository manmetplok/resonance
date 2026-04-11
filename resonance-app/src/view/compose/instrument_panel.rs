use iced::widget::{button, column, container, pick_list, row, text, text_input, Space};
use iced::{alignment, Element, Font, Length};

use crate::compose::ComposeMessage;
use crate::message::*;
use crate::state::{InstrumentIcon, InstrumentType, TrackRole, TrackState};
use crate::theme;

/// Dropdown-friendly wrapper around `Option<TrackRole>`. Iced's pick_list
/// needs a concrete type with Display; `Option<T>` doesn't implement it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RolePick {
    None,
    Pad,
    Bass,
    Lead,
}

impl RolePick {
    const ALL: [RolePick; 4] = [
        RolePick::None,
        RolePick::Pad,
        RolePick::Bass,
        RolePick::Lead,
    ];
}

impl std::fmt::Display for RolePick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            RolePick::None => "None",
            RolePick::Pad => "Pad",
            RolePick::Bass => "Bass",
            RolePick::Lead => "Lead",
        })
    }
}

impl From<Option<TrackRole>> for RolePick {
    fn from(r: Option<TrackRole>) -> Self {
        match r {
            None => RolePick::None,
            Some(TrackRole::Pad) => RolePick::Pad,
            Some(TrackRole::Bass) => RolePick::Bass,
            Some(TrackRole::Lead) => RolePick::Lead,
        }
    }
}

impl From<RolePick> for Option<TrackRole> {
    fn from(p: RolePick) -> Self {
        match p {
            RolePick::None => None,
            RolePick::Pad => Some(TrackRole::Pad),
            RolePick::Bass => Some(TrackRole::Bass),
            RolePick::Lead => Some(TrackRole::Lead),
        }
    }
}

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
        .on_input(move |s| Message::Track(TrackMessage::SetTrackName(track_id, s)))
        .size(12)
        .padding([4, 6])
        .width(Length::Fill);

    let type_picker = pick_list(
        InstrumentType::ALL.to_vec(),
        Some(track.instrument_type),
        move |ty| Message::Track(TrackMessage::SetInstrumentType(track_id, ty)),
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let icon_picker = pick_list(
        InstrumentIcon::ALL.to_vec(),
        Some(track.instrument_icon),
        move |icon| Message::Track(TrackMessage::SetInstrumentIcon(track_id, icon)),
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let role_picker = pick_list(
        RolePick::ALL.to_vec(),
        Some(RolePick::from(track.role)),
        move |pick| {
            Message::Compose(ComposeMessage::SetTrackRole {
                track_id,
                role: Option::<TrackRole>::from(pick),
            })
        },
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
        Space::with_height(8),
        text("Role").size(10).color(theme::TEXT_DIM),
        role_picker,
        text("Tagged tracks are auto-targeted by Derive.")
            .size(10)
            .color(theme::TEXT_DIM),
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
