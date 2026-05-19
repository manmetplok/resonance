//! Bass generator parameter panel.

use iced::widget::{column, pick_list, slider, text, Space};
use iced::{Element, Length};

use resonance_audio::types::TrackId;
use resonance_music_theory::{BassMotifMode, BassMotifPhrase, BassStyle};

use crate::compose::messages::LaneInspectorMsg;
use crate::compose::ComposeMessage;
use crate::message::*;
use crate::theme;

use crate::view::compose::lane_inspector::label_with_info;

use super::{bass_base_note_options, NotePick};

pub(super) fn bass_controls<'a>(
    definition_id: u64,
    track_id: TrackId,
    params: &'a resonance_music_theory::BassParams,
) -> Element<'a, Message> {
    let style_picker = pick_list(BassStyle::ALL.to_vec(), Some(params.style), move |style| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetBassStyle(style),
        })
    })
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let base_note_picker = pick_list(
        bass_base_note_options(),
        Some(NotePick(params.base_note)),
        move |pick| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetBassBaseNote(pick.0),
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
            msg: LaneInspectorMsg::SetBassVelocity(v),
        })
    })
    .step(0.01)
    .width(Length::Fill);

    let mut col = column![
        label_with_info(
            "Style",
            "Bass voicing pattern.\n\u{2022} Root hold: one note per chord, full duration\n\u{2022} Root pulse: root on every beat\n\u{2022} Root + fifth: alternating root/fifth per beat\n\u{2022} Octave: root and root+12 alternating\n\u{2022} Walking: stepwise scale line into next chord (needs a scale)\n\u{2022} Motif: render the section’s shared motif in the bass register"
        ),
        style_picker,
        Space::new().height(4),
        label_with_info(
            "Base note",
            "MIDI floor for bass roots. Each chord’s root is moved to the nearest pitch at or above this note."
        ),
        base_note_picker,
        Space::new().height(4),
        label_with_info(
            format!("Velocity: {:.2}", params.velocity),
            "MIDI velocity (0–1) for emitted notes. Accented motif notes get a small +0.05 boost on top."
        ),
        vel_slider,
    ]
    .spacing(2);

    if params.style == BassStyle::Motif {
        let mode_picker = pick_list(
            BassMotifMode::ALL.to_vec(),
            Some(params.motif_mode),
            move |mode| {
                Message::Compose(ComposeMessage::LaneInspector {
                    definition_id,
                    track_id,
                    msg: LaneInspectorMsg::SetBassMotifMode(mode),
                })
            },
        )
        .text_size(12)
        .padding([4, 6])
        .width(Length::Fill);

        let phrase_picker = pick_list(
            BassMotifPhrase::ALL.to_vec(),
            Some(params.motif_phrase),
            move |phrase| {
                Message::Compose(ComposeMessage::LaneInspector {
                    definition_id,
                    track_id,
                    msg: LaneInspectorMsg::SetBassMotifPhrase(phrase),
                })
            },
        )
        .text_size(12)
        .padding([4, 6])
        .width(Length::Fill);

        col = col
            .push(Space::new().height(8))
            .push(label_with_info(
                "Motif mode",
                "How the bass renders the section motif.\n\u{2022} Same intervals: literal motif at the bass anchor\n\u{2022} Augmented: same intervals, each note 2× longer (slow line under the melody)\n\u{2022} Rhythm only: motif rhythm + accents, pitch is the chord root\n\u{2022} First note only: one note per chord on the chord root"
            ))
            .push(mode_picker)
            .push(Space::new().height(4))
            .push(label_with_info(
                "Phrase development",
                "How per-phrase Transforms are picked.\n\u{2022} Simple: Identity every phrase — predictable foundation\n\u{2022} Mirror melody: same Transform per phrase as the melody motif lane (locked together)\n\u{2022} Restricted: random Identity/Augment per phrase, independent of melody"
            ))
            .push(phrase_picker)
            .push(Space::new().height(4))
            .push(
                text("Motif knobs live in the Chords lane.")
                    .size(10)
                    .color(theme::TEXT_DIM),
            );
    }

    col.into()
}
