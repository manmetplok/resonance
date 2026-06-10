//! Per-track header cell for the Arrange view track-header column:
//! 28×28 instrument glyph + name/kind stacked column + a slim 4-button
//! row (mute / solo / arm / monitor). Selection paints a 2px lavender
//! stripe on the left edge.
//!
//! Mono toggle, FX bypass, and bounce-in-place stay on the mixer strip
//! where channel-strip controls live; the Arrange header is for arranging.
use iced::widget::{column, container, mouse_area, row, text, Space};
use iced::{alignment, Color, Element, Length};

use crate::message::*;
use crate::state::{self, TrackState};
use crate::theme::{self, fa};
use crate::view::controls::{
    delete_button, monitor_button, mute_button, record_arm_button, solo_button,
};
use crate::util::short;
use crate::Resonance;

pub(super) fn view_track_header(
    _r: &Resonance,
    track: &TrackState,
    is_selected: bool,
) -> Element<'static, Message> {
    let track_id = track.id;

    // ---- Glyph (28×28 rounded BG_2 square with the track's instrument icon) ----
    let glyph_char = glyph_for_track(track);
    let glyph = container(
        theme::icon(glyph_char)
            .size(13)
            .color(theme::TEXT_2),
    )
    .width(28)
    .height(28)
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: theme::RADIUS_MD.into(),
        },
        ..Default::default()
    });

    // ---- Name (top) + kind (bottom) ----
    let name = text(track.name.clone())
        .size(13)
        .font(theme::UI_FONT_MEDIUM)
        .color(theme::TEXT_1)
        .wrapping(iced::widget::text::Wrapping::None);
    let kind_str = kind_label_for_track(track);
    let kind = text(kind_str)
        .size(10)
        .color(theme::TEXT_3)
        .wrapping(iced::widget::text::Wrapping::None);

    let name_col = column![
        container(name).width(Length::Fill).clip(true),
        container(kind).width(Length::Fill).clip(true),
    ]
    .spacing(2);

    // ---- 4 mini buttons: Mute / Solo / Arm / Monitor ----
    let buttons = row![
        mute_button(
            track.muted,
            Message::Track(TrackMessage::ToggleMute(track.id)),
            12,
        ),
        solo_button(
            track.soloed,
            Message::Track(TrackMessage::ToggleSolo(track.id)),
            12,
        ),
        record_arm_button(track.record_armed, track.id, 12),
        monitor_button(track.monitor_enabled, track.id, 12),
    ]
    .spacing(5)
    .align_y(alignment::Vertical::Center);

    // ---- Top-right delete (tiny, hugs the corner) ----
    let del = delete_button(
        Message::Track(TrackMessage::RequestRemoveTrack(track.id)),
        11,
    );

    // Top of the cell: name + kind, with delete in the corner.
    let top_row = row![
        glyph,
        Space::new().width(12),
        name_col,
        Space::new().width(6),
        del,
    ]
    .spacing(0)
    .align_y(alignment::Vertical::Center);

    // Bottom of the cell: 4-button row, right-aligned to keep the glyph +
    // name visually the dominant element.
    let button_row = row![Space::new().width(Length::Fill), buttons]
        .align_y(alignment::Vertical::Center);

    let body_col = column![top_row, Space::new().height(8), button_row,]
        .spacing(0)
        .height(Length::Fill);

    // ---- Background, left selection stripe, hairline bottom ----
    let bg = if track.record_armed {
        theme::PANEL_ARMED
    } else if is_selected {
        theme::BG_2
    } else {
        theme::BG_1
    };
    let stripe_color = if track.record_armed {
        theme::BAD
    } else if is_selected {
        theme::ACCENT
    } else {
        Color::TRANSPARENT
    };

    let body = container(body_col)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(iced::Padding {
            top: 10.0,
            right: 24.0,
            bottom: 10.0,
            left: 24.0,
        });

    let body_with_bg = container(body)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(bg)),
            ..Default::default()
        });

    let stripe = container(Space::new().height(Length::Fill))
        .width(2)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(stripe_color)),
            ..Default::default()
        });

    // The cell is `TRACK_HEIGHT - 1` so that `cell + hairline` together
    // sum to exactly `TRACK_HEIGHT` — matching the canvas's per-row
    // pitch. Without this trim, every column row was 1 px taller than
    // the canvas row and headers drifted down 1 px per track.
    let cell = row![stripe, body_with_bg].height(theme::TRACK_HEIGHT - 1.0);

    // 1px hairline below each cell so rows separate without a heavy border.
    let hairline = container(Space::new().width(Length::Fill))
        .height(1)
        .style(theme::separator_bg);

    let stack = column![cell, hairline].spacing(0);

    mouse_area(stack)
        .on_press(Message::Ui(UiMessage::SelectTrack(Some(track_id))))
        .into()
}

/// Pick a Font Awesome glyph for the given track. Uses the persisted
/// `instrument_icon` for instrument tracks; audio tracks get a microphone.
fn glyph_for_track(track: &TrackState) -> char {
    match track.track_type {
        resonance_audio::types::TrackType::Audio => fa::MICROPHONE,
        resonance_audio::types::TrackType::Instrument => track.instrument_icon.glyph(),
        resonance_audio::types::TrackType::Vocal => fa::MICROPHONE,
    }
}

/// Build the small descriptor line under the track name. Examples:
/// - Audio track: "Audio · MIC 1" (or just "Audio" when no input is set)
/// - Instrument track with plugin: "Resonance Wave"
/// - Drum track: "Kit · Resonance Drums"
/// - Track with no plugin yet: "Instrument" or "Audio"
fn kind_label_for_track(track: &TrackState) -> String {
    use resonance_audio::types::TrackType;
    let plugin_name = track
        .plugins
        .first()
        .map(|p| p.plugin_name.clone())
        .unwrap_or_default();
    // The track-list column leaves ~160px for the name/kind stack once
    // the glyph, paddings, and delete button are accounted for, so the
    // kind line cannot exceed ~22 chars at 10px before it wraps.
    // `short()` enforces an ellipsis well within that.
    match track.track_type {
        TrackType::Audio => match track.input_device_name.as_deref() {
            Some(dev) if !dev.is_empty() => format!("Audio · {}", short(dev, 14)),
            _ => "Audio".to_string(),
        },
        TrackType::Instrument => {
            if plugin_name.is_empty() {
                if track.instrument_type == state::InstrumentType::Drum {
                    "Drum kit".to_string()
                } else {
                    "Instrument".to_string()
                }
            } else if track.instrument_type == state::InstrumentType::Drum {
                format!("Kit · {}", short(&plugin_name, 14))
            } else {
                short(&plugin_name, 22)
            }
        }
        TrackType::Vocal => "Vocal".to_string(),
    }
}
