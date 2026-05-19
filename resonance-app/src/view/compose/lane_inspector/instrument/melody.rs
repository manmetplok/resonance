//! Melody generator parameter panel.

use iced::widget::{column, pick_list, slider, text, Space};
use iced::{Element, Length};

use resonance_audio::types::TrackId;
use resonance_music_theory::{ContourPreference, MelodyStyle};

use crate::compose::messages::LaneInspectorMsg;
use crate::compose::ComposeMessage;
use crate::message::*;
use crate::theme;

use crate::view::compose::lane_inspector::label_with_info;
use crate::view::compose::lane_inspector::vocal::toggle_row;

use super::{register_high_options, register_low_options, NotePick, NoteValuePick, PhraseLenPick};

pub(super) fn melody_controls<'a>(
    definition_id: u64,
    track_id: TrackId,
    params: &'a resonance_music_theory::MelodyParams,
) -> Element<'a, Message> {
    let style_picker = pick_list(
        MelodyStyle::ALL.to_vec(),
        Some(params.style),
        move |style| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetMelodyStyle(style),
            })
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let reg_lo_picker = pick_list(
        register_low_options(),
        Some(NotePick(params.register.0)),
        move |pick| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetMelodyRegisterLow(pick.0),
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
                msg: LaneInspectorMsg::SetMelodyRegisterHigh(pick.0),
            })
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    // Note value as a user-friendly pick list
    let note_values = vec![
        NoteValuePick(480, "Quarter"),
        NoteValuePick(240, "Eighth"),
        NoteValuePick(120, "Sixteenth"),
    ];
    let current_nv = note_values
        .iter()
        .find(|nv| nv.0 == params.note_value_ticks)
        .cloned()
        .unwrap_or(NoteValuePick(params.note_value_ticks, "Custom"));

    let nv_picker = pick_list(note_values, Some(current_nv), move |pick| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetMelodyNoteValue(pick.0),
        })
    })
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let rest_slider = slider(0.0..=1.0, params.rest_density, move |v| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetMelodyRestDensity(v),
        })
    })
    .step(0.01)
    .width(Length::Fill);

    let vel_slider = slider(0.0..=1.0, params.velocity, move |v| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetMelodyVelocity(v),
        })
    })
    .step(0.01)
    .width(Length::Fill);

    let mut col = column![
        label_with_info(
            "Style",
            "Melodic generator.\n\u{2022} Arp up / down / up-down: cycle through chord tones\n\u{2022} Motif: develop a short cell across phrases (uses the section motif knobs)"
        ),
        style_picker,
        Space::new().height(4),
        label_with_info(
            "Register low",
            "Lowest MIDI note this melody is allowed to play."
        ),
        reg_lo_picker,
        label_with_info(
            "Register high",
            "Highest MIDI note this melody is allowed to play."
        ),
        reg_hi_picker,
        Space::new().height(4),
    ]
    .spacing(2);

    // Arp-only controls
    if params.style != MelodyStyle::Motif {
        col = col
            .push(label_with_info(
                "Note value",
                "Length of one melody note (arp styles only). Quarter / Eighth / Sixteenth at the project tempo."
            ))
            .push(nv_picker);
    }

    col = col
        .push(Space::new().height(4))
        .push(label_with_info(
            format!("Rest density: {:.2}", params.rest_density),
            "Probability that any given slot is silent. 0 = no rests. Higher values produce sparser, more breathing melodies."
        ))
        .push(rest_slider)
        .push(label_with_info(
            format!("Velocity: {:.2}", params.velocity),
            "Base MIDI velocity (0–1). Motif accents add a small +0.05 boost on top."
        ))
        .push(vel_slider)
        .push(Space::new().height(4))
        .push(toggle_row(
            "Fill in vocal gaps",
            params.fill_vocal_gaps,
            LaneInspectorMsg::ToggleMelodyFillVocalGaps,
            definition_id,
            track_id,
        ));

    // Motif-specific controls — only those that are lane-local. The
    // motif's own knobs (complexity / motif length / leap chance) live
    // on the section so every Motif lane shares one identity.
    if params.style == MelodyStyle::Motif {
        let articulation_slider = slider(0.0..=1.0, params.articulation, move |v| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetMelodyArticulation(v),
            })
        })
        .step(0.01)
        .width(Length::Fill);

        let contour_picker = pick_list(
            ContourPreference::ALL.to_vec(),
            Some(params.contour),
            move |c| {
                Message::Compose(ComposeMessage::LaneInspector {
                    definition_id,
                    track_id,
                    msg: LaneInspectorMsg::SetMelodyContour(c),
                })
            },
        )
        .text_size(12)
        .padding([4, 6])
        .width(Length::Fill);

        let phrase_len_options = vec![
            PhraseLenPick(2),
            PhraseLenPick(4),
            PhraseLenPick(8),
        ];
        let phrase_len_picker = pick_list(
            phrase_len_options,
            Some(PhraseLenPick(params.phrase_len)),
            move |pick| {
                Message::Compose(ComposeMessage::LaneInspector {
                    definition_id,
                    track_id,
                    msg: LaneInspectorMsg::SetMelodyPhraseLen(pick.0),
                })
            },
        )
        .text_size(12)
        .padding([4, 6])
        .width(Length::Fill);

        col = col
            .push(Space::new().height(4))
            .push(label_with_info(
                format!("Articulation: {:.2}", params.articulation),
                "How short each note sounds relative to its rhythmic slot. 0 = legato (full slot), 1 = staccato (about 45% of the slot)."
            ))
            .push(articulation_slider)
            .push(Space::new().height(4))
            .push(label_with_info(
                "Contour",
                "Preferred melodic shape per phrase. Auto picks one per phrase from research-weighted distributions; the others pin every phrase to the chosen shape."
            ))
            .push(contour_picker)
            .push(label_with_info(
                "Phrase length",
                "How many chords belong to one phrase. Each phrase gets its own contour and Transform."
            ))
            .push(phrase_len_picker)
            .push(Space::new().height(4))
            .push(
                text("Motif knobs live in the Chords lane.")
                    .size(10)
                    .color(theme::TEXT_DIM),
            );
    }

    col.into()
}
