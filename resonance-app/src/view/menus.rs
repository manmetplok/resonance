//! Overlay menus (add-track popover, marker context menu / rename, etc.)
use iced::widget::{
    button, column, container, mouse_area, opaque, row, scrollable, stack, text, text_input, Space,
};
use iced::{alignment, Color, Element, Length};

use crate::message::*;
use crate::presets::TrackPreset;
use crate::state::{
    ArrangementMarker, FreezeStatus, MarkerMenuState, MarkerRenameState, PresetSaveState,
    TrackMenuState, TrackState,
};
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
        .on_press(Message::Track(TrackMessage::AddTrackFromPreset {
            preset: Box::new(preset_clone),
            id_hint: None,
            name: None,
        }))
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

    // External-instrument track: pairs a hardware synth's MIDI output with
    // its audio return (doc #251 gap 1). A sliders/hardware-panel icon and
    // the lavender ACCENT tint distinguish it from the built-in Instrument
    // entry (whose cyan tint means "software instrument").
    let ext_inst_btn = button(
        row![
            theme::icon(fa::SLIDERS).size(14).color(theme::ACCENT),
            Space::new().width(8),
            text("Ext Instrument").size(13).color(theme::ACCENT),
        ]
        .align_y(alignment::Vertical::Center),
    )
    .on_press(Message::Track(TrackMessage::AddExternalInstrumentTrack))
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
        ext_inst_btn,
        vocal_btn,
    ]
    .spacing(2);

    // Default presets section.
    if !r.presets.default_presets.is_empty() {
        menu = menu
            .push(Space::new().height(4))
            .push(container(Space::new().width(Length::Fill).height(1)).style(theme::separator_bg))
            .push(Space::new().height(4))
            .push(text("Presets").size(10).color(theme::TEXT_DIM));
        for preset in &r.presets.default_presets {
            menu = menu.push(preset_button(preset, false));
        }
    }

    // User presets section.
    if !r.presets.user_presets.is_empty() {
        menu = menu
            .push(Space::new().height(4))
            .push(container(Space::new().width(Length::Fill).height(1)).style(theme::separator_bg))
            .push(Space::new().height(4))
            .push(text("User Presets").size(10).color(theme::TEXT_DIM));
        for preset in &r.presets.user_presets {
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
    if let Some(rename) = &r.ui.interaction.marker_rename {
        return marker_rename_overlay(rename);
    }
    if let Some(menu) = &r.ui.interaction.marker_menu {
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

// ---------------------------------------------------------------------
// Track context menu (design doc #181, ba todo #581)
// ---------------------------------------------------------------------

/// The right-click context menu for an arrange track: the freeze surface's
/// convenience entries — Freeze track (⌘F) / Unfreeze track / Freeze
/// selected tracks / Freeze all tracks (⇧⌘F) / Reveal freeze cache….
/// Renders nothing when no menu is open (or its track is gone).
pub(crate) fn view_track_menu_overlay(r: &Resonance) -> Element<'_, Message> {
    // The name prompt takes over from the menu that opened it.
    if let Some(prompt) = &r.ui.interaction.preset_save {
        return preset_save_overlay(prompt);
    }
    if let Some(menu) = &r.ui.interaction.track_menu {
        if let Some(track) = r.registry.tracks.iter().find(|t| t.id == menu.track_id) {
            return track_menu_overlay(r, menu, track);
        }
    }
    Space::new().into()
}

/// A single full-width row in the track context menu: label on the left,
/// optional keyboard-shortcut hint right-aligned in the mono font. A
/// disabled entry renders dimmed with no `on_press` (iced then feeds the
/// style closure `Status::Disabled`).
fn track_menu_item(
    label: &'static str,
    shortcut: Option<String>,
    msg: Message,
    enabled: bool,
) -> Element<'static, Message> {
    let label_color = if enabled { theme::TEXT } else { theme::TEXT_DIM };
    let mut inner = row![text(label).size(12).color(label_color)]
        .align_y(alignment::Vertical::Center);
    if let Some(hint) = shortcut {
        inner = inner.push(Space::new().width(Length::Fill)).push(
            text(hint)
                .size(10)
                .font(theme::MONO_FONT)
                .color(theme::TEXT_DIM),
        );
    }
    let mut btn = button(inner)
        .width(Length::Fill)
        .padding([5, 10])
        .style(|_theme, status| theme::transport_button_style(status));
    if enabled {
        btn = btn.on_press(msg);
    }
    btn.into()
}

/// Build the open track context menu overlay: click-away backdrop plus the
/// floating menu box anchored at the row that was right-clicked.
///
/// Enabled / disabled states (design doc #181):
/// - **Freeze track** — freezable (instrument / vocal, not a sub-track)
///   and currently live (idle or failed);
/// - **Unfreeze track** — disabled when live: only a frozen / stale track
///   has a cache to detach;
/// - **Freeze selected tracks** — some selected track is freezable;
/// - **Freeze all tracks** — some freezable track is still live;
/// - **Reveal freeze cache…** — the project has been saved (an unsaved
///   project has no cache directory yet).
fn track_menu_overlay<'a>(
    r: &'a Resonance,
    menu: &'a TrackMenuState,
    track: &'a TrackState,
) -> Element<'a, Message> {
    use crate::update::freeze as freeze_logic;

    let id = track.id;
    let status = r.freeze.status(id);
    let is_live = matches!(status, FreezeStatus::Idle | FreezeStatus::Failed { .. });
    let can_freeze = freeze_logic::freezable(track).is_ok() && is_live;
    let can_unfreeze = status.is_frozen();
    // Both batch entries require something the queue would actually
    // render: a freezable track that is still live (`start_batch` skips
    // frozen / mid-render tracks).
    let batch_would_freeze = |tid: &resonance_audio::types::TrackId| {
        matches!(
            r.freeze.status(*tid),
            FreezeStatus::Idle | FreezeStatus::Failed { .. }
        )
    };
    let can_freeze_selected = freeze_logic::selected_freezable_tracks(r)
        .iter()
        .any(batch_would_freeze);
    let can_freeze_all = freeze_logic::freezable_tracks(r)
        .iter()
        .any(batch_would_freeze);
    let can_reveal = r.io.project_path.is_some();

    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::Ui(UiMessage::CloseTrackMenu));

    let menu_col = column![
        track_menu_item(
            "Freeze track",
            None,
            Message::Freeze(FreezeMessage::FreezeTrack(id)),
            can_freeze,
        ),
        track_menu_item(
            "Unfreeze track",
            None,
            Message::Freeze(FreezeMessage::UnfreezeTrack(id)),
            can_unfreeze,
        ),
        marker_menu_sep(),
        track_menu_item(
            "Freeze selected tracks",
            crate::view::shortcut_hint::chord_text(r, crate::commands::CommandId::FreezeSelectedTracks),
            Message::Freeze(FreezeMessage::FreezeSelectedTracks),
            can_freeze_selected,
        ),
        track_menu_item(
            "Freeze all tracks",
            crate::view::shortcut_hint::chord_text(r, crate::commands::CommandId::FreezeAllTracks),
            Message::Freeze(FreezeMessage::FreezeAllTracks),
            can_freeze_all,
        ),
        marker_menu_sep(),
        // The other half of a preset menu that could only ever be read
        // from (ba todo #1303): every track carries what a preset needs.
        track_menu_item(
            "Save as preset\u{2026}",
            None,
            Message::Track(TrackMessage::OpenSavePresetPrompt(id)),
            true,
        ),
        marker_menu_sep(),
        track_menu_item(
            "Reveal freeze cache\u{2026}",
            None,
            Message::Freeze(FreezeMessage::RevealFreezeCache),
            can_reveal,
        ),
    ]
    .spacing(1)
    .width(210);

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

/// The "Save track as preset" prompt (ba todo #1303, finding P1).
///
/// One field and two buttons, and the only subtlety is the one that
/// matters: when the typed name already exists the action button says
/// **Overwrite**, because saving replaces the file. That is the same
/// promise `track.save_preset` keeps with its `overwrite` flag — a
/// preset is never silently replaced on either surface.
fn preset_save_overlay(prompt: &PresetSaveState) -> Element<'_, Message> {
    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba(
                    0.0, 0.0, 0.0, 0.5,
                ))),
                ..Default::default()
            }),
    )
    .on_press(Message::Track(TrackMessage::CloseSavePresetPrompt));

    let named = !prompt.name.trim().is_empty();
    let save_msg = Message::Track(TrackMessage::SaveTrackAsPreset {
        track_id: prompt.track_id,
        name: prompt.name.clone(),
        // The button's label already told the user which of the two
        // things it is about to do.
        overwrite: prompt.exists,
    });

    let field = text_input("Preset name", &prompt.name)
        .on_input(|s| Message::Track(TrackMessage::SetSavePresetName(s)))
        .on_submit(save_msg.clone())
        .size(13)
        .padding([6, 8])
        .width(Length::Fill);

    let mut body = column![
        text("Save track as preset").size(14).color(theme::TEXT),
        Space::new().height(8),
        field,
    ]
    .spacing(2)
    .width(300);

    if prompt.exists {
        body = body.push(
            text(format!(
                "Replaces the existing preset {:?}",
                prompt.name.trim()
            ))
            .size(10)
            .color(theme::WARM),
        );
    }

    let mut save_btn = button(
        text(if prompt.exists { "Overwrite" } else { "Save" })
            .size(12)
            .color(theme::TEXT),
    )
    .padding([4, 12])
    .style(|_theme, status| theme::transport_button_style(status));
    if named {
        save_btn = save_btn.on_press(save_msg);
    }

    let cancel_btn = button(text("Cancel").size(12).color(theme::TEXT_DIM))
        .on_press(Message::Track(TrackMessage::CloseSavePresetPrompt))
        .padding([4, 12])
        .style(|_theme, status| theme::transport_button_style(status));

    body = body.push(Space::new().height(10)).push(
        row![
            Space::new().width(Length::Fill),
            cancel_btn,
            Space::new().width(6),
            save_btn,
        ]
        .align_y(alignment::Vertical::Center),
    );

    let boxed = container(opaque(body.padding(14))).style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::PANEL)),
        border: iced::Border {
            color: theme::SEPARATOR,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..Default::default()
    });

    let centered = container(boxed)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill);

    stack![backdrop, centered].into()
}
