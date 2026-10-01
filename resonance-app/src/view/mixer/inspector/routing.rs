//! ROUTING group for the mixer inspector — where the track's signal
//! comes from and goes to: input device / MIDI in, output destination,
//! MIDI out.
//!
//! Sends have their own SENDS group, and the external-hardware pairing
//! lives in TRACK (mixer-cleanup.md §3.1). An external-instrument track's
//! input and MIDI out are part of that pairing, so for one ROUTING shows
//! only the output picker.

use iced::widget::{column, Space};
use iced::Element;
use resonance_audio::types::TrackType;

use crate::message::Message;
use crate::state::{MixerInspectorGroup, TrackState};

pub(super) fn routing_group(
    r: &crate::Resonance,
    track: &TrackState,
    collapsed: bool,
) -> Element<'static, Message> {
    let header = super::widgets::group_header("ROUTING", MixerInspectorGroup::Routing, collapsed);
    if collapsed {
        return header;
    }

    let output_block = super::io::output_block(r, track);
    if r.devices.external_instruments.contains_key(&track.id) {
        return column![header, Space::new().height(10), output_block]
            .spacing(0)
            .into();
    }

    let input_block: Element<'static, Message> = match track.track_type {
        TrackType::Audio => super::io::audio_input_block(r, track),
        TrackType::Instrument | TrackType::Vocal => super::io::midi_input_block(r, track),
    };
    let mut col = column![
        header,
        Space::new().height(10),
        input_block,
        Space::new().height(8),
        output_block,
    ]
    .spacing(0);
    if track.track_type.accepts_midi() && track.sub_track.is_none() {
        col = col
            .push(Space::new().height(8))
            .push(super::io::midi_output_block(r, track));
    }
    col.into()
}
