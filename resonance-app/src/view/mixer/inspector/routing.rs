//! Generic ROUTING group for the mixer inspector — input device / MIDI
//! in / MIDI out / output pickers, plus read-only Send placeholders.
//! External-instrument tracks delegate to `external_instrument` instead.

use iced::widget::{column, container, row, text, Space};
use iced::{alignment, Element, Length};
use resonance_audio::types::TrackType;

use crate::message::Message;
use crate::state::{MixerInspectorGroup, TrackState};
use crate::theme;

pub(super) fn routing_group(
    r: &crate::Resonance,
    track: &TrackState,
    collapsed: bool,
) -> Element<'static, Message> {
    // External-instrument tracks replace the generic input / MIDI-out
    // routing with the dedicated "External Instrument" group (doc #169).
    if let Some(ext) = r.external_instruments.get(&track.id) {
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

    column![
        super::widgets::group_header("ROUTING", MixerInspectorGroup::Routing, false),
        Space::new().height(10),
        input_block,
        Space::new().height(8),
        output_block,
        Space::new().height(8),
        midi_out_block,
        Space::new().height(4),
        routing_row("Send A", "(none)", true),
        routing_row("Send B", "(none)", true),
    ]
    .spacing(0)
    .into()
}

/// Read-only routing row used for Send A/B placeholders.
fn routing_row(
    label: &'static str,
    value: &'static str,
    muted: bool,
) -> Element<'static, Message> {
    let value_color = if muted { theme::TEXT_4 } else { theme::TEXT_1 };
    let r_row = row![
        text(label).size(11).color(theme::TEXT_3),
        Space::new().width(Length::Fill),
        text(value).size(12).font(theme::MONO_FONT).color(value_color),
    ]
    .align_y(alignment::Vertical::Center)
    .padding([6, 0]);

    column![
        r_row,
        container(Space::new().width(Length::Fill))
            .height(1)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::LINE_2)),
                ..Default::default()
            }),
    ]
    .spacing(0)
    .into()
}
