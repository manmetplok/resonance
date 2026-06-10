//! Voice & delivery group — timbre / voicebank / singer / vibrato /
//! tension / portamento / articulation / consonant emphasis.

use iced::widget::{column, row, slider, text, Space};
use iced::{Element, Length};

use resonance_audio::types::TrackId;
use resonance_music_theory::{
    VocalParams, VocalSinger, VocalSingerMeiji, VocalTimbre, VocalVoicebank,
};

use crate::compose::messages::LaneInspectorMsg;
use crate::compose::ComposeMessage;
use crate::message::*;
use crate::theme;

use super::common::{chip, dim_label, group_card, group_title};

pub(super) fn voice_group<'a>(
    definition_id: u64,
    track_id: TrackId,
    params: &'a VocalParams,
    collapsed: bool,
) -> Element<'a, Message> {
    let key = crate::compose::RailPanelKey::VocalVoice(track_id);
    let title = group_title("Voice & delivery", key, collapsed);
    if collapsed {
        return group_card(title);
    }

    let mut timbre_row = row![].spacing(4);
    for t in VocalTimbre::ALL.iter().copied() {
        timbre_row = timbre_row.push(chip(
            t.as_str(),
            params.timbre == t,
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetVocalTimbre(t),
            }),
        ));
    }

    // Voicebank chips — picks the trained DiffSinger model that
    // produces the singing audio. Each voicebank has its own character;
    // singer chips below are scoped to the chosen voicebank.
    let voicebank_chips: Vec<Element<'_, Message>> = VocalVoicebank::ALL
        .iter()
        .copied()
        .map(|v| {
            chip(
                v.as_str(),
                params.voicebank == v,
                Message::Compose(ComposeMessage::LaneInspector {
                    definition_id,
                    track_id,
                    msg: LaneInspectorMsg::SetVocalVoicebank(v),
                }),
            )
        })
        .collect();
    let voicebank_row =
        iced::widget::Row::with_children(voicebank_chips).spacing(4).wrap();

    // Singer chips — only meaningful for multi-speaker voicebanks.
    // TIGER ships 7 `tiger_*` speakers; Lilia is single-speaker so the
    // chip row is hidden when Lilia is selected.
    let singer_block: Option<Element<'_, Message>> = match params.voicebank {
        VocalVoicebank::Tiger => {
            let chips: Vec<Element<'_, Message>> = VocalSinger::ALL
                .iter()
                .copied()
                .map(|s| {
                    chip(
                        s.as_str(),
                        params.singer == s,
                        Message::Compose(ComposeMessage::LaneInspector {
                            definition_id,
                            track_id,
                            msg: LaneInspectorMsg::SetVocalSinger(s),
                        }),
                    )
                })
                .collect();
            let row = iced::widget::Row::with_children(chips).spacing(4).wrap();
            Some(
                column![dim_label("Singer"), Space::new().height(2), row]
                    .spacing(0)
                    .into(),
            )
        }
        VocalVoicebank::Lilia => None,
        VocalVoicebank::Meiji => {
            let chips: Vec<Element<'_, Message>> = VocalSingerMeiji::ALL
                .iter()
                .copied()
                .map(|s| {
                    chip(
                        s.as_str(),
                        params.singer_meiji == s,
                        Message::Compose(ComposeMessage::LaneInspector {
                            definition_id,
                            track_id,
                            msg: LaneInspectorMsg::SetVocalSingerMeiji(s),
                        }),
                    )
                })
                .collect();
            let row = iced::widget::Row::with_children(chips).spacing(4).wrap();
            Some(
                column![dim_label("Mode"), Space::new().height(2), row]
                    .spacing(0)
                    .into(),
            )
        }
    };

    let vibrato_label = dim_label(format!("Vibrato · {:.2}", params.vibrato));
    let vibrato_slider = slider(0.0..=1.0, params.vibrato, move |v| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetVocalVibrato(v),
        })
    })
    .step(0.01)
    .width(Length::Fill);

    let vibrato_rate_label = dim_label(format!("Vibrato rate · {:.1} Hz", params.vibrato_rate));
    let vibrato_rate_slider = slider(4.0..=7.0, params.vibrato_rate, move |v| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetVocalVibratoRate(v),
        })
    })
    .step(0.1)
    .width(Length::Fill);

    // Tension — only meaningful for voicebanks that accept the
    // `tension` ONNX input (Lilia today; Meiji once it's added).
    // The slider always renders for consistency; selecting TIGER just
    // ignores the value.
    let tension_label = dim_label(format!(
        "Tension · {:+.2} ({})",
        params.tension,
        if params.tension < -0.05 {
            "breathy"
        } else if params.tension > 0.05 {
            "belted"
        } else {
            "neutral"
        }
    ));
    let tension_slider = slider(-1.0..=1.0, params.tension, move |v| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetVocalTension(v),
        })
    })
    .step(0.05)
    .width(Length::Fill);

    // Per-syllable velocity → tension. 0 = constant tension; 1 =
    // strong beats fully tensed.
    let tension_vel_label = dim_label(format!(
        "↳ velocity \u{2192} tension · {:.2}",
        params.tension_velocity_amount
    ));
    let tension_vel_slider =
        slider(0.0..=1.0, params.tension_velocity_amount, move |v| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetVocalTensionVelocityAmount(v),
            })
        })
        .step(0.05)
        .width(Length::Fill);

    // Pitch contour → tension. 0 = constant; 1 = top of range belted.
    let tension_contour_label = dim_label(format!(
        "↳ contour \u{2192} tension · {:.2}",
        params.tension_contour_amount
    ));
    let tension_contour_slider =
        slider(0.0..=1.0, params.tension_contour_amount, move |v| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetVocalTensionContourAmount(v),
            })
        })
        .step(0.05)
        .width(Length::Fill);

    // Portamento — pitch glide between adjacent notes, in
    // milliseconds. 0 = hard step, 200 = strong scoop / slide.
    let portamento_label = dim_label(format!(
        "Portamento · {:.0} ms ({})",
        params.portamento_ms,
        if params.portamento_ms < 15.0 {
            "snappy"
        } else if params.portamento_ms > 80.0 {
            "scoopy"
        } else {
            "natural"
        }
    ));
    let portamento_slider = slider(0.0..=200.0, params.portamento_ms, move |v| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetVocalPortamentoMs(v),
        })
    })
    .step(5.0)
    .width(Length::Fill);

    let articulation_label = dim_label(format!("Articulation · {:.2}", params.articulation));
    let articulation_slider = slider(0.0..=1.0, params.articulation, move |v| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetVocalArticulation(v),
        })
    })
    .step(0.01)
    .width(Length::Fill);

    let consonant_label = dim_label(format!(
        "Consonant emphasis · {:.2}",
        params.consonant_emphasis
    ));
    let consonant_slider = slider(0.0..=1.0, params.consonant_emphasis, move |v| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetVocalConsonantEmphasis(v),
        })
    })
    .step(0.01)
    .width(Length::Fill);

    // Slider math is `trim = 0.98 - 0.48 * articulation`, so low =
    // notes fill the slot (legato) and high = notes are short with
    // gaps (staccato). Labels reflect that direction.
    let stacc_legato_hint = row![
        text("\u{00B7} legato")
            .size(9)
            .font(theme::SERIF_ITALIC_FONT)
            .color(theme::TEXT_4),
        Space::new().width(Length::Fill),
        text("staccato \u{00B7}")
            .size(9)
            .font(theme::SERIF_ITALIC_FONT)
            .color(theme::TEXT_4),
    ];

    let mut body = column![
        title,
        Space::new().height(8),
        dim_label("Timbre"),
        Space::new().height(2),
        timbre_row,
        Space::new().height(8),
        dim_label("Voicebank"),
        Space::new().height(2),
        voicebank_row,
    ]
    .spacing(0);
    if let Some(singer) = singer_block {
        body = body.push(Space::new().height(8)).push(singer);
    }
    let body = body
        .push(Space::new().height(8))
        .push(vibrato_label)
        .push(Space::new().height(2))
        .push(vibrato_slider)
        .push(Space::new().height(4))
        .push(vibrato_rate_label)
        .push(Space::new().height(2))
        .push(vibrato_rate_slider)
        .push(Space::new().height(6))
        .push(tension_label)
        .push(Space::new().height(2))
        .push(tension_slider)
        .push(Space::new().height(4))
        .push(tension_vel_label)
        .push(Space::new().height(2))
        .push(tension_vel_slider)
        .push(Space::new().height(4))
        .push(tension_contour_label)
        .push(Space::new().height(2))
        .push(tension_contour_slider)
        .push(Space::new().height(4))
        .push(portamento_label)
        .push(Space::new().height(2))
        .push(portamento_slider)
        .push(Space::new().height(6))
        .push(articulation_label)
        .push(Space::new().height(2))
        .push(articulation_slider)
        .push(Space::new().height(2))
        .push(stacc_legato_hint)
        .push(Space::new().height(6))
        .push(consonant_label)
        .push(Space::new().height(2))
        .push(consonant_slider);

    group_card(body.into())
}
