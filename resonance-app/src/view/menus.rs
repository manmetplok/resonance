//! Overlay menus (add-track popover, etc.)
use iced::widget::{
    button, column, container, mouse_area, opaque, row, scrollable, stack, text, Space,
};
use iced::{alignment, Color, Element, Length};

use crate::message::*;
use crate::presets::TrackPreset;
use crate::theme::{self, fa};
use crate::Resonance;

/// Render a single preset row in the add-track menu.
fn preset_button(preset: &TrackPreset, is_user: bool) -> Element<'_, Message> {
    let icon_char = preset.instrument_icon.glyph();
    let icon_color = if preset.track_type == "instrument" {
        Color::from_rgb(0.3, 0.75, 0.8)
    } else {
        theme::TEXT
    };

    let mut btn_row = row![
        theme::icon(icon_char).size(12).color(icon_color),
        Space::new().width(6),
        text(&preset.name).size(12).color(theme::TEXT),
    ]
    .align_y(alignment::Vertical::Center);

    if is_user {
        // Show a small delete button for user presets.
        let name = preset.name.clone();
        let del = button(text("\u{00d7}").size(10).color(theme::TEXT_DIM))
            .on_press(Message::Track(TrackMessage::DeleteUserPreset(name)))
            .style(|_theme, status| theme::small_button_style(status))
            .padding([0, 3]);
        btn_row = btn_row.push(Space::new().width(Length::Fill)).push(del);
    }

    let preset_clone = preset.clone();
    button(btn_row)
        .on_press(Message::Track(TrackMessage::AddTrackFromPreset(Box::new(
            preset_clone,
        ))))
        .width(Length::Fill)
        .padding([4, 10])
        .style(|_theme, status| theme::transport_button_style(status))
        .into()
}

pub(crate) fn view_add_track_menu(r: &Resonance) -> Element<'_, Message> {
    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
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
            Space::new().width(8),
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
            Space::new().width(8),
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

    // Warm tint matches the Compose vocal-lane accent so the user
    // associates the menu item with where the track will appear.
    let vocal_btn = button(
        row![
            theme::icon(fa::MICROPHONE).size(14).color(theme::WARM),
            Space::new().width(8),
            text("Vocal").size(13).color(theme::WARM),
        ]
        .align_y(alignment::Vertical::Center),
    )
    .on_press(Message::Track(TrackMessage::AddVocalTrack))
    .width(Length::Fill)
    .padding([6, 10])
    .style(|_theme, status| theme::transport_button_style(status));

    let mut menu = column![
        text("Add Track").size(11).color(theme::TEXT_DIM),
        Space::new().height(4),
        audio_btn,
        inst_btn,
        vocal_btn,
    ]
    .spacing(2);

    // Default presets section.
    if !r.default_presets.is_empty() {
        menu = menu
            .push(Space::new().height(4))
            .push(container(Space::new().width(Length::Fill).height(1)).style(theme::separator_bg))
            .push(Space::new().height(4))
            .push(text("Presets").size(10).color(theme::TEXT_DIM));
        for preset in &r.default_presets {
            menu = menu.push(preset_button(preset, false));
        }
    }

    // User presets section.
    if !r.user_presets.is_empty() {
        menu = menu
            .push(Space::new().height(4))
            .push(container(Space::new().width(Length::Fill).height(1)).style(theme::separator_bg))
            .push(Space::new().height(4))
            .push(text("User Presets").size(10).color(theme::TEXT_DIM));
        for preset in &r.user_presets {
            menu = menu.push(preset_button(preset, true));
        }
    }

    let menu_content = menu.padding(8).width(200);

    // Wrap in a scrollable so long preset lists don't overflow the window.
    let scrollable_menu = scrollable(menu_content).height(Length::Shrink);

    let menu_container = container(opaque(scrollable_menu)).style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::PANEL)),
        border: iced::Border {
            color: theme::SEPARATOR,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..Default::default()
    });

    // Position the popup just below the "+" button in the track header ruler area.
    let top_pad = 48.0 + theme::RULER_HEIGHT + 2.0;
    let positioned = container(menu_container)
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
