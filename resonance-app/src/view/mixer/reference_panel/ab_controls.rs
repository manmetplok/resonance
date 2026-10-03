//! A/B control stack for the active reference: the Mix/Reference switch,
//! the waveform overview, the marker + loop row, the loudness-match toggle,
//! and the level trim slider.

use iced::widget::{button, column, container, row, slider, text, Space};
use iced::{alignment, Color, Element, Length};

use resonance_audio::types::{ABSource, ReferenceId};

use crate::message::*;
use crate::reference::{ReferenceEntry, ReferenceMarkerState, ReferenceMessage, ReferenceState};
use crate::theme::{self};
use crate::util::format_db_signed;

use super::waveform;

/// The A/B control stack for the active reference: the Mix/Reference
/// switch, the waveform overview, the marker + loop row, the
/// loudness-match toggle and the level trim.
pub(super) fn ab_controls<'a>(
    state: &'a ReferenceState,
    entry: &'a ReferenceEntry,
) -> Element<'a, Message> {
    container(
        column![
            ab_switch(state.monitor.ab_source),
            Space::new().height(14),
            waveform::waveform(entry),
            Space::new().height(10),
            marker_row(state, entry),
            Space::new().height(14),
            loudness_row(state),
            Space::new().height(12),
            trim_row(state.trim_db),
        ]
        .spacing(0),
    )
    .width(Length::Fill)
    .padding([16, 16])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: theme::RADIUS_MD.into(),
        },
        ..Default::default()
    })
    .into()
}

/// Two-segment A/B switch. **A · Mix** lights lavender (ACCENT) and
/// **B · Reference** lights amber (WARM); pressing a segment selects that
/// source outright. A "hold B" hint nods to the momentary key.
fn ab_switch(source: ABSource) -> Element<'static, Message> {
    let seg = |label: &str, on: bool, color: Color, set: ABSource| {
        button(
            text(label.to_string())
                .size(12)
                .font(theme::UI_FONT_MEDIUM),
        )
        .width(Length::Fill)
        .padding([8, 12])
        .on_press(Message::Reference(ReferenceMessage::SetAbSource(set)))
        .style(move |_theme, status| ab_segment_style(on, color, status))
    };

    column![
        row![
            seg(
                "A \u{00b7} Mix",
                source == ABSource::Mix,
                theme::ACCENT,
                ABSource::Mix,
            ),
            seg(
                "B \u{00b7} Reference",
                source == ABSource::Reference,
                theme::WARM,
                ABSource::Reference,
            ),
        ]
        .spacing(8),
        Space::new().height(6),
        text("Hold B to monitor the reference")
            .size(10)
            .color(theme::TEXT_3),
    ]
    .spacing(0)
    .into()
}

/// Style for one A/B segment. Active lights with a tint of its source
/// colour (lavender Mix / amber Reference); inactive keeps a visible
/// BG_2 + hairline card so the control always reads as two segments.
fn ab_segment_style(active: bool, color: Color, status: button::Status) -> button::Style {
    let (bg, text_color, border_color) = if active {
        let a = match status {
            button::Status::Hovered => 0.22,
            button::Status::Pressed => 0.30,
            _ => 0.16,
        };
        (Color { a, ..color }, color, color)
    } else {
        let bg = match status {
            button::Status::Hovered | button::Status::Pressed => theme::BG_3,
            _ => theme::BG_2,
        };
        (bg, theme::TEXT_2, theme::LINE_2)
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color,
        border: iced::Border {
            color: border_color,
            width: 1.0,
            radius: theme::RADIUS_SM.into(),
        },
        ..Default::default()
    }
}

/// The marker + loop affordances under the waveform: an "Add marker"
/// button (drops one at the current cursor), the loop-to-mix chip, then a
/// wrap of removable marker chips.
fn marker_row<'a>(state: &'a ReferenceState, entry: &'a ReferenceEntry) -> Element<'a, Message> {
    let add = button(text("Add marker").size(11).font(theme::UI_FONT_MEDIUM))
        .padding([6, 10])
        .on_press(Message::Reference(ReferenceMessage::AddMarker {
            ref_id: entry.id,
            position_samples: entry.position_samples,
            label: format!("Marker {}", entry.markers.len() + 1),
        }))
        .style(|_theme, status| theme::small_button_style(status));

    let loop_chip = button(text("Loop to mix").size(11).font(theme::UI_FONT_MEDIUM))
        .padding([6, 10])
        .on_press(Message::Reference(ReferenceMessage::ToggleLoopToMix))
        .style(move |_theme, status| {
            theme::toggle_button_style(state.monitor.loop_to_mix, theme::ACCENT, true, status)
        });

    let mut chips = row![].spacing(6).align_y(alignment::Vertical::Center);
    for mk in &entry.markers {
        chips = chips.push(marker_chip(entry.id, mk));
    }

    column![
        row![add, loop_chip, Space::new().width(Length::Fill)].spacing(8),
        Space::new().height(if entry.markers.is_empty() { 0 } else { 8 }),
        chips,
    ]
    .spacing(0)
    .into()
}

/// One removable marker chip: its label and a × that removes it.
fn marker_chip(ref_id: ReferenceId, mk: &ReferenceMarkerState) -> Element<'static, Message> {
    let remove = button(text("\u{00d7}").size(12).color(theme::TEXT_2))
        .on_press(Message::Reference(ReferenceMessage::RemoveMarker {
            ref_id,
            marker_id: mk.id,
        }))
        .padding([1, 5])
        .style(|_theme, status| theme::small_button_style(status));

    container(
        row![
            text(mk.label.clone()).size(10).color(theme::TEXT_2),
            remove,
        ]
        .spacing(2)
        .align_y(alignment::Vertical::Center),
    )
    .padding([2, 4])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_3)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: theme::RADIUS_XS.into(),
        },
        ..Default::default()
    })
    .into()
}

/// The loudness-match toggle, with the engine-reported gain offset shown
/// alongside once matching is engaged.
fn loudness_row(state: &ReferenceState) -> Element<'static, Message> {
    let matched = state.loudness_match;
    let offset_db = state.monitor.offset_db;

    let toggle = button(
        text("Match loudness")
            .size(12)
            .font(theme::UI_FONT_MEDIUM),
    )
    .padding([7, 12])
    .on_press(Message::Reference(ReferenceMessage::ToggleLoudnessMatch))
    .style(move |_theme, status| {
        theme::toggle_button_style(matched, theme::GOOD, true, status)
    });

    let offset = if matched {
        text(format_db_signed(offset_db, true))
            .size(11)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_2)
    } else {
        text("off").size(11).color(theme::TEXT_3)
    };

    row![toggle, Space::new().width(Length::Fill), offset]
        .align_y(alignment::Vertical::Center)
        .into()
}

/// The reference level trim: a ±12 dB slider with a monospace readout.
fn trim_row(trim_db: f32) -> Element<'static, Message> {
    let trim = slider(-12.0..=12.0f32, trim_db, |v| {
        Message::Reference(ReferenceMessage::TrimChanged(v))
    })
    .step(0.1);

    column![
        row![
            text("Trim").size(11).color(theme::TEXT_3),
            Space::new().width(Length::Fill),
            text(format_db_signed(trim_db, true))
                .size(11)
                .font(theme::MONO_FONT)
                .color(theme::TEXT_2),
        ]
        .align_y(alignment::Vertical::Center),
        Space::new().height(6),
        trim,
    ]
    .spacing(0)
    .into()
}
