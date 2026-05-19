//! Pad generator parameter panel.

use iced::widget::{column, pick_list, slider, Space};
use iced::{Element, Length};

use resonance_audio::types::TrackId;

use crate::compose::messages::LaneInspectorMsg;
use crate::compose::ComposeMessage;
use crate::message::*;

use crate::view::compose::lane_inspector::label_with_info;

use super::{register_high_options, register_low_options, NotePick};

pub(super) fn pad_controls<'a>(
    definition_id: u64,
    track_id: TrackId,
    params: &'a resonance_music_theory::PadParams,
) -> Element<'a, Message> {
    let reg_lo_picker = pick_list(
        register_low_options(),
        Some(NotePick(params.register.0)),
        move |pick| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetPadRegisterLow(pick.0),
            })
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let reg_hi_picker = pick_list(
        register_high_options(),
        Some(NotePick(params.register.1)),
        move |pick| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetPadRegisterHigh(pick.0),
            })
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let vel_slider = slider(0.0..=1.0, params.velocity, move |v| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetPadVelocity(v),
        })
    })
    .step(0.01)
    .width(Length::Fill);

    column![
        label_with_info(
            "Register low",
            "Lowest MIDI note the pad voicings can reach. Voices that fall below this float up an octave."
        ),
        reg_lo_picker,
        label_with_info(
            "Register high",
            "Highest MIDI note the pad voicings can reach. Voices that rise above this drop an octave."
        ),
        reg_hi_picker,
        Space::new().height(4),
        label_with_info(
            format!("Velocity: {:.2}", params.velocity),
            "MIDI velocity (0–1) for every emitted pad voice."
        ),
        vel_slider,
    ]
    .spacing(2)
    .into()
}
