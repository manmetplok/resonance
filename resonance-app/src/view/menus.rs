//! Overlay menus (add-track popover, marker context menu / rename, etc.)
use iced::widget::{
    button, column, container, mouse_area, opaque, row, scrollable, stack, text, text_input, Space,
};
use iced::{alignment, Color, Element, Length};

use crate::message::*;
use crate::presets::TrackPreset;
use crate::state::{ArrangementMarker, MarkerMenuState, MarkerRenameState};
use crate::theme::{self, fa};
use crate::Resonance;

/// Swatch palette offered by the marker recolor row. Mirrors the
/// auto-assign palette in `update/marker.rs` so a recolored marker can land
/// back on any of the default flag colours.
const MARKER_PALETTE: [[u8; 3]; 6] = [
    [0xE5, 0x4B, 0x4B], // red
    [0xE5, 0x9B, 0x33], // orange
    [0xE5, 0xD0, 0x33], // yellow
    [0x5C, 0xC4, 0x6B], // green
    [0x3D, 0x8B, 0xE5], // blue
    [0x9B, 0x5C, 0xE5], // violet
];

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

    // Position the popup just below the "+" button, which lives at the
    // right edge of the global-shelf header strip: window chrome +
    // transport bar + ruler + (section band, when sections exist) +
    // shelf header.
    let section_band_h = if r.compose.placements.is_empty() {
        0.0
    } else {
        theme::SECTION_BAND_HEIGHT
    };
    let top_pad = super::transport::CHROME_HEIGHT
        + super::transport::TRANSPORT_HEIGHT
        + theme::RULER_HEIGHT
        + section_band_h
        + theme::GLOBAL_SHELF_HEADER_HEIGHT
        + 2.0;
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

/// Render the arrangement-marker overlay — either the right-click context
/// menu or the inline rename field, whichever is active (todo #369). The
/// caller mounts this only while `marker_menu` / `marker_rename` is set.
/// Positioned in window space at the anchor captured when the interaction
/// began. Falls back to an empty element when the target marker has
/// vanished (e.g. deleted straight from the menu) so the stack drops away.
pub(crate) fn view_marker_overlay(r: &Resonance) -> Element<'_, Message> {
    // Inline rename takes priority — opening it always clears the menu.
    if let Some(rename) = &r.interaction.marker_rename {
        return marker_rename_overlay(rename);
    }
    if let Some(menu) = &r.interaction.marker_menu {
        if let Some(marker) = r.markers.get(menu.marker_id) {
            return marker_menu_overlay(r, menu, marker);
        }
    }
    Space::new().into()
}

/// A single full-width text row in the marker context menu.
fn marker_menu_item(label: &str, msg: Message) -> Element<'_, Message> {
    button(text(label).size(12).color(theme::TEXT))
        .on_press(msg)
        .width(Length::Fill)
        .padding([5, 10])
        .style(|_theme, status| theme::transport_button_style(status))
        .into()
}

/// A thin horizontal separator between marker-menu groups.
fn marker_menu_sep() -> Element<'static, Message> {
    container(Space::new().width(Length::Fill).height(1))
        .style(theme::separator_bg)
        .into()
}

/// A colour swatch button in the recolor row.
fn marker_swatch(id: u64, color: [u8; 3]) -> Element<'static, Message> {
    let fill = Color::from_rgb8(color[0], color[1], color[2]);
    button(
        container(Space::new().width(16).height(14)).style(move |_theme| container::Style {
            background: Some(iced::Background::Color(fill)),
            border: iced::Border {
                color: theme::SEPARATOR,
                width: 1.0,
                radius: 3.0.into(),
            },
            ..Default::default()
        }),
    )
    .on_press(Message::Marker(MarkerMessage::Recolor(id, color)))
    .padding(2)
    .style(|_theme, status| theme::small_button_style(status))
    .into()
}

/// The right-click context menu for a marker: Rename, Recolor (palette),
/// Delete, Loop to section, Play from here, and Convert to region / point.
fn marker_menu_overlay<'a>(
    r: &'a Resonance,
    menu: &'a MarkerMenuState,
    marker: &'a ArrangementMarker,
) -> Element<'a, Message> {
    let id = marker.id;

    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::MarkerUi(MarkerUiMessage::CloseMenu));

    // Recolor swatch row.
    let mut swatches = row![].spacing(4);
    for color in MARKER_PALETTE {
        swatches = swatches.push(marker_swatch(id, color));
    }

    // Convert flips between point <-> region. Promoting to a region spans
    // the marker's bar to the next bar so the new region is visible and
    // grid-aligned; demoting drops the end back to a point.
    let convert_item = if marker.is_region() {
        marker_menu_item(
            "Convert to point",
            Message::Marker(MarkerMessage::SetRegionEnd(id, None)),
        )
    } else {
        let (bar, _) = r.tempo_map.sample_to_bar(marker.start_sample, r.sample_rate);
        let end = r
            .tempo_map
            .bar_to_sample(bar.saturating_add(1))
            .max(marker.start_sample + 1);
        marker_menu_item(
            "Convert to region",
            Message::Marker(MarkerMessage::SetRegionEnd(id, Some(end))),
        )
    };

    let menu_col = column![
        marker_menu_item(
            "Rename",
            Message::MarkerUi(MarkerUiMessage::BeginRename {
                id,
                x: menu.x,
                y: menu.y,
            }),
        ),
        container(
            column![
                text("Recolor").size(10).color(theme::TEXT_DIM),
                Space::new().height(3),
                swatches,
            ]
            .spacing(0),
        )
        .padding([5, 10]),
        marker_menu_sep(),
        marker_menu_item(
            "Loop to section",
            Message::Marker(MarkerMessage::LoopToRegion(id)),
        ),
        marker_menu_item(
            "Play from here",
            Message::Marker(MarkerMessage::PlayFromMarker(id)),
        ),
        convert_item,
        marker_menu_sep(),
        marker_menu_item("Delete", Message::Marker(MarkerMessage::Delete(id))),
    ]
    .spacing(1)
    .width(180);

    let menu_box = container(opaque(menu_col)).style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::PANEL)),
        border: iced::Border {
            color: theme::SEPARATOR,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..Default::default()
    });

    let positioned = container(menu_box)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Horizontal::Left)
        .align_y(alignment::Vertical::Top)
        .padding(iced::Padding {
            top: menu.y,
            right: 0.0,
            bottom: 0.0,
            left: menu.x,
        });

    stack![backdrop, positioned].into()
}

/// The floating inline rename field for a marker. Commits on Enter or
/// click-away; the edit buffer lives in `marker_rename` state.
fn marker_rename_overlay(rename: &MarkerRenameState) -> Element<'_, Message> {
    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::MarkerUi(MarkerUiMessage::CommitRename));

    let field = text_input("Marker name", &rename.text)
        .on_input(|s| Message::MarkerUi(MarkerUiMessage::RenameChanged(s)))
        .on_submit(Message::MarkerUi(MarkerUiMessage::CommitRename))
        .size(12)
        .padding([4, 6])
        .width(160);

    let boxed = container(opaque(field)).style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::PANEL)),
        border: iced::Border {
            color: theme::ACCENT,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..Default::default()
    });

    let positioned = container(boxed)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Horizontal::Left)
        .align_y(alignment::Vertical::Top)
        .padding(iced::Padding {
            top: rename.y,
            right: 0.0,
            bottom: 0.0,
            left: rename.x,
        });

    stack![backdrop, positioned].into()
}
