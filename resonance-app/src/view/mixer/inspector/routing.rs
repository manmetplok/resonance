//! Generic ROUTING group for the mixer inspector — input device / MIDI
//! in / MIDI out / output pickers, plus the aux-send slots.
//! External-instrument tracks delegate to `external_instrument` instead.

use iced::widget::{button, column, text, Space};
use iced::{alignment, Element, Length};
use resonance_audio::types::TrackType;

use crate::message::{ExternalInstrumentMessage, Message};
use crate::state::{MixerInspectorGroup, TrackState};
use crate::theme;

pub(super) fn routing_group(
    r: &crate::Resonance,
    track: &TrackState,
    collapsed: bool,
) -> Element<'static, Message> {
    // External-instrument tracks replace the generic input / MIDI-out
    // routing with the dedicated "External Instrument" group (doc #169).
    if let Some(ext) = r.devices.external_instruments.get(&track.id) {
        if collapsed {
            return super::widgets::group_header(
                "EXTERNAL INSTRUMENT",
                MixerInspectorGroup::Routing,
                true,
            );
        }
        return super::external_instrument::external_instrument_group(r, track, ext);
    }

    if collapsed {
        return super::widgets::group_header("ROUTING", MixerInspectorGroup::Routing, true);
    }

    let input_block: Element<'static, Message> = match track.track_type {
        TrackType::Audio => super::io::audio_input_block(r, track),
        TrackType::Instrument | TrackType::Vocal => super::io::midi_input_block(r, track),
    };
    let output_block = super::io::output_block(r, track);
    let midi_out_block: Element<'static, Message> =
        if track.track_type.accepts_midi() && track.sub_track.is_none() {
            super::io::midi_output_block(r, track)
        } else {
            Space::new().height(0).into()
        };

    // Enable affordance — only instrument tracks can become external
    // hardware instruments. Audio/Vocal never grow the toggle (mirrors the
    // EXTERNAL INSTRUMENT group's own instrument-only gating); master/bus
    // strips render their own inspector and never reach this view. Dispatches
    // `Enable`, which drops the user onto the onboarding ("Unassigned") card.
    let external_enable: Element<'static, Message> =
        if matches!(track.track_type, TrackType::Instrument) {
            column![
                Space::new().height(8),
                enable_external_row(track.id),
            ]
            .spacing(0)
            .into()
        } else {
            Space::new().height(0).into()
        };

    column![
        super::widgets::group_header("ROUTING", MixerInspectorGroup::Routing, false),
        Space::new().height(10),
        input_block,
        Space::new().height(8),
        output_block,
        Space::new().height(8),
        midi_out_block,
        Space::new().height(8),
        // The real aux-send slots (ba todo #1310). These replaced two
        // hardcoded read-only `Send A -> (none)` / `Send B -> (none)`
        // rows that named a feature the GUI could not reach: the whole
        // send graph shipped engine-first and was only ever driven over
        // the control API.
        super::sends::sends_block(r, track),
        external_enable,
    ]
    .spacing(0)
    .into()
}

/// Full-width understated action row that converts the (instrument) track
/// into an external hardware instrument. Styled like the other inspector
/// routing affordances — a hairline-bordered button that tints toward the
/// accent on hover — dispatching `ExternalInstrumentMessage::Enable`.
fn enable_external_row(track_id: resonance_audio::types::TrackId) -> Element<'static, Message> {
    button(
        text("External hardware instrument")
            .size(11)
            .align_x(alignment::Horizontal::Center)
            .width(Length::Fill),
    )
    .padding([7, 0])
    .width(Length::Fill)
    .on_press(Message::ExternalInstrument(ExternalInstrumentMessage::Enable(
        track_id,
    )))
    .style(|_theme, status| {
        let hovered = matches!(status, button::Status::Hovered);
        let (bg, border, txt) = if hovered {
            (theme::BG_3, theme::ACCENT_LINE, theme::TEXT_1)
        } else {
            (theme::BG_2, theme::LINE, theme::TEXT_3)
        };
        button::Style {
            background: Some(iced::Background::Color(bg)),
            text_color: txt,
            border: iced::Border {
                color: border,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        }
    })
    .into()
}
