//! TRACK group — the track's own options (mixer-cleanup.md §3.5): the
//! mono toggle, Bounce in place (instrument tracks), and the external
//! hardware section — the "External hardware…" enable action
//! on a plain instrument track, or the whole external-instrument pairing
//! (`external_instrument`) on a track that already is one — then MIDI
//! CONTROL, the track's MIDI Learn bindings (`midi`).

use iced::widget::{column, row, text, Space};
use iced::Element;
use resonance_audio::types::TrackType;

use crate::message::{ExternalInstrumentMessage, Message, TrackMessage};
use crate::state::{ExternalInstrumentStatus, MixerInspectorGroup, TrackState};
use crate::theme;

pub(super) fn track_group(
    r: &crate::Resonance,
    track: &TrackState,
    collapsed: bool,
) -> Element<'static, Message> {
    let header = super::widgets::group_header("TRACK", MixerInspectorGroup::Track, collapsed);
    if collapsed {
        return header;
    }

    let mono = super::widgets::toggle_button(
        "Mono",
        track.mono,
        theme::ACCENT_SOFT,
        theme::ACCENT_DIM,
        Message::Track(TrackMessage::ToggleTrackMono(track.id)),
    );

    let mut col = column![header, Space::new().height(10)].spacing(0);

    if track.track_type == TrackType::Instrument {
        // Enabled exactly when the strip's bounce glyph is: the reducer
        // runs the same `classify_bounce` and reports the same reason, so
        // the inspector shows that reason instead of a dead button.
        let bounce = crate::update::track::classify_bounce(
            track,
            r.midi_clips.iter().map(|c| c.track_id),
        );
        let bounce_btn = super::widgets::action_button(
            "Bounce",
            bounce
                .is_ok()
                .then_some(Message::Track(TrackMessage::BounceInPlace(track.id))),
            false,
        );
        col = col.push(row![mono, Space::new().width(8), bounce_btn]);
        if let Err(reason) = bounce {
            col = col
                .push(Space::new().height(6))
                .push(text(reason).size(10).color(theme::TEXT_4));
        }
    } else {
        col = col.push(mono);
    }

    // External hardware section. Only instrument tracks can be paired
    // with a hardware synth (audio / vocal never grow it).
    if let Some(ext) = r.devices.external_instruments.get(&track.id) {
        col = col.push(Space::new().height(14));
        // A fresh pairing (nothing set yet) gets the dashed onboarding
        // card walking through the setup steps above the pickers (#459).
        if ext.status(track) == ExternalInstrumentStatus::Unconfigured {
            col = col
                .push(super::onboarding::onboarding_card())
                .push(Space::new().height(14));
        }
        col = col.push(super::external_instrument::external_instrument_group(
            r, track, ext,
        ));
    } else if track.track_type == TrackType::Instrument && track.sub_track.is_none() {
        // Dispatches `Enable`, which drops the user onto the onboarding
        // ("Unconfigured") card above. Not on a sub-track: it is one of
        // its parent plugin's output ports, with no MIDI of its own to
        // send out to hardware.
        col = col.push(Space::new().height(8)).push(super::widgets::action_button(
            "External hardware\u{2026}",
            Some(Message::ExternalInstrument(ExternalInstrumentMessage::Enable(
                track.id,
            ))),
            false,
        ));
    }

    col.push(Space::new().height(14))
        .push(super::midi::midi_section(r, track))
        .into()
}
