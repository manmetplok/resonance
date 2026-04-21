/// Shared track/bus control widgets used by both the Arrange track header
/// and the Mixer channel strip. Factoring these here keeps icon choice,
/// color, and button style consistent across both surfaces.
use iced::widget::{button, column, container, row, text, vertical_slider, Space};
use iced::{alignment, Element, Font, Length};
use resonance_audio::types::{BusId, TrackId};

use crate::message::*;
use crate::theme::{self, fa};
use crate::util::format_db;

/// Record-arm toggle button (filled circle — red when armed).
pub fn record_arm_button<'a>(
    armed: bool,
    track_id: TrackId,
    size: u16,
) -> iced::widget::Button<'a, Message> {
    let color = if armed {
        theme::RECORD_RED
    } else {
        theme::TEXT_DIM
    };
    button(theme::icon(fa::CIRCLE).size(size).color(color))
        .on_press(Message::Track(TrackMessage::ToggleRecordArm(track_id)))
        .style(move |_theme, status| {
            if armed {
                theme::record_armed_button_style(status)
            } else {
                theme::small_button_style(status)
            }
        })
        .padding(2)
}

/// Mute toggle button (speaker-with-X — accent when muted).
pub fn mute_button<'a>(
    muted: bool,
    on_press: Message,
    size: u16,
) -> iced::widget::Button<'a, Message> {
    let color = if muted {
        theme::ACCENT
    } else {
        theme::TEXT_DIM
    };
    button(theme::icon(fa::VOLUME_XMARK).size(size).color(color))
        .on_press(on_press)
        .style(|_theme, status| theme::small_button_style(status))
        .padding(2)
}

/// Solo toggle button (headphones — yellow when soloed).
pub fn solo_button<'a>(
    soloed: bool,
    on_press: Message,
    size: u16,
) -> iced::widget::Button<'a, Message> {
    let color = if soloed {
        theme::SOLO_YELLOW
    } else {
        theme::TEXT_DIM
    };
    button(theme::icon(fa::HEADPHONES).size(size).color(color))
        .on_press(on_press)
        .style(|_theme, status| theme::small_button_style(status))
        .padding(2)
}

/// Input-monitor toggle button (eye — green when monitoring).
pub fn monitor_button<'a>(
    enabled: bool,
    track_id: TrackId,
    size: u16,
) -> iced::widget::Button<'a, Message> {
    let color = if enabled {
        theme::METRONOME_ON
    } else {
        theme::TEXT_DIM
    };
    button(theme::icon(fa::EYE).size(size).color(color))
        .on_press(Message::Track(TrackMessage::ToggleMonitor(track_id)))
        .style(move |_theme, status| {
            theme::toggle_button_style(enabled, theme::METRONOME_ON, true, status)
        })
        .padding(2)
}

/// Mono/Stereo toggle button. One hollow circle for mono, two for stereo.
pub fn mono_button<'a>(
    is_mono: bool,
    track_id: TrackId,
    size: u16,
) -> iced::widget::Button<'a, Message> {
    let glyph = if is_mono {
        fa::CIRCLE_HOLLOW
    } else {
        fa::CIRCLE_HOLLOW_DOUBLE
    };
    button(theme::icon(glyph).size(size).color(theme::TEXT))
        .on_press(Message::Track(TrackMessage::ToggleTrackMono(track_id)))
        .style(move |_theme, status| theme::mono_button_style(is_mono, status))
        .padding(2)
}

/// FX bypass toggle button. Small "FX" text label; dim when the
/// effects chain is active (normal state) and tinted with the accent
/// colour when bypassed (so the user can see at a glance which strips
/// have their chain disabled).
pub fn fx_bypass_button<'a>(
    bypassed: bool,
    on_press: Message,
    size: u16,
) -> iced::widget::Button<'a, Message> {
    let color = if bypassed {
        theme::ACCENT
    } else {
        theme::TEXT_DIM
    };
    button(text("FX").size(size).color(color))
        .on_press(on_press)
        .style(|_theme, status| theme::small_button_style(status))
        .padding([2, 3])
}

/// Trash/delete button (gray trash can).
pub fn delete_button<'a>(on_press: Message, size: u16) -> iced::widget::Button<'a, Message> {
    button(theme::icon(fa::TRASH).size(size).color(theme::TEXT_DIM))
        .on_press(on_press)
        .style(|_theme, status| theme::small_button_style(status))
        .padding(2)
}

/// Bus remove button — same style as delete, used on bus strips.
pub fn bus_remove_button<'a>(bus_id: BusId, size: u16) -> iced::widget::Button<'a, Message> {
    delete_button(Message::Bus(BusMessage::RemoveBus(bus_id)), size)
}

/// Convert linear amplitude to meter bar height (logarithmic/dB scale).
fn level_to_bar_height(level: f32, max_height: f32) -> f32 {
    if level < 0.0001 {
        return 0.0;
    }
    let db = 20.0 * level.log10();
    let normalized = (db + 60.0) / 66.0; // -60dB=0, +6dB=1
    normalized.clamp(0.0, 1.0) * max_height
}

/// Get meter color based on signal level (green / yellow / red).
fn level_color(level: f32) -> iced::Color {
    if level < 0.0001 {
        return theme::METRONOME_ON;
    }
    let db = 20.0 * level.log10();
    if db > 0.0 {
        theme::RECORD_RED
    } else if db > -6.0 {
        theme::SOLO_YELLOW
    } else {
        theme::METRONOME_ON
    }
}

/// Render a single vertical VU meter bar.
fn meter_bar_v<'a>(level: f32, max_height: f32) -> Element<'a, Message> {
    let bar_height = level_to_bar_height(level, max_height);
    let color = level_color(level);
    let spacer_height = (max_height - bar_height).max(0.0);

    let bar = container(Space::new(0.0, 0.0))
        .width(Length::Fill)
        .height(bar_height)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(color)),
            ..Default::default()
        });

    container(column![Space::new(Length::Fill, spacer_height), bar])
        .width(6)
        .height(max_height)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::METER_BG)),
            ..Default::default()
        })
        .into()
}

/// Render a stereo vertical VU meter (L + R bars side by side).
pub fn meter_v<'a>(level_l: f32, level_r: f32, height: f32) -> Element<'a, Message> {
    row![meter_bar_v(level_l, height), meter_bar_v(level_r, height)]
        .spacing(1)
        .into()
}

/// Render the shared fader + meter + dB label block used by tracks, busses,
/// and master. The caller supplies the message factory for the slider —
/// this lets the same widget drive track/bus/master volumes.
pub fn fader_section<'a, F>(
    level_l: f32,
    level_r: f32,
    volume_db: f32,
    on_change: F,
) -> Element<'a, Message>
where
    F: 'a + Fn(f32) -> Message,
{
    let fader = vertical_slider(-60.0..=6.0f32, volume_db, on_change)
        .height(theme::FADER_HEIGHT)
        .step(0.1);
    let meters = meter_v(level_l, level_r, theme::FADER_HEIGHT);
    let label = text(format_db(volume_db))
        .size(9)
        .font(Font::MONOSPACE)
        .color(theme::TEXT_DIM);
    column![
        container(
            row![meters, fader]
                .spacing(4)
                .align_y(alignment::Vertical::Center)
        )
        .width(Length::Fill)
        .center_x(Length::Fill),
        label,
    ]
    .spacing(2)
    .align_x(alignment::Horizontal::Center)
    .into()
}
