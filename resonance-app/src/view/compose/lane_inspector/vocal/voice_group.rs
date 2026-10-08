//! Voice & delivery group — timbre / voicebank / speaker / the bank's own
//! timing and pitch models / vibrato / tension / portamento / articulation
//! / consonant emphasis / rest threshold / velocity dynamics.
//!
//! The Voicebank and Speaker pickers are driven by the voicebank registry
//! (vocal-mcp-control-and-quality.md §4): the options are the installed
//! English banks, cached in `UiViewCaches::voicebank_choices`, and the
//! speakers are the chosen bank's own. A lane on a built-in bank that is
//! not installed shows it as `<id> (not installed)`, from the cached
//! `UiViewCaches::missing_voicebank_choices`.

use iced::widget::{button, column, pick_list, row, slider, text, Space};
use iced::{Element, Length};

use resonance_audio::types::TrackId;
use resonance_music_theory::{VocalParams, VocalTimbre};

use crate::compose::messages::LaneInspectorMsg;
use crate::compose::ComposeMessage;
use crate::message::*;
use crate::theme;

use super::common::{chip, dim_label, group_card, group_title};
use super::toggle_row;
use crate::state::picks::{SpeakerChoice, VoicebankChoice};
use crate::state::ui_caches::{ChoiceList, VocalRailContext};

pub(super) fn voice_group<'a>(
    definition_id: u64,
    track_id: TrackId,
    params: &VocalParams,
    vocal: &VocalRailContext,
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

    let (voicebank_picker, speaker_block, models) =
        voicebank_pickers(definition_id, track_id, params, vocal);
    // With the bank's pitch model singing, the synthesized line's vibrato,
    // vibrato rate and portamento are not heard: their sliders are muted
    // and say why (they are heard again with the toggle off).
    let bank_sings_pitch = models.pitch && params.bank_pitch;
    let line_label = |s: String| -> Element<'a, Message> {
        if bank_sings_pitch {
            text(s).size(10).color(theme::TEXT_4).into()
        } else {
            dim_label(s)
        }
    };
    let line_style = move |t: &iced::Theme, s: slider::Status| {
        if bank_sings_pitch {
            theme::muted_slider_style(t, s)
        } else {
            slider::default(t, s)
        }
    };

    let vibrato_label = line_label(format!("Vibrato · {:.2}", params.vibrato));
    let vibrato_slider = slider(0.0..=1.0, params.vibrato, move |v| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetVocalVibrato(v),
        })
    })
    .step(0.01)
    .style(line_style)
    .width(Length::Fill);

    let vibrato_rate_label = line_label(format!("Vibrato rate · {:.1} Hz", params.vibrato_rate));
    let vibrato_rate_slider = slider(4.0..=7.0, params.vibrato_rate, move |v| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetVocalVibratoRate(v),
        })
    })
    .step(0.1)
    .style(line_style)
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
    let portamento_label = line_label(format!(
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
    .style(line_style)
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

    // Rest threshold: a gap of at least this many beats is silence, a
    // shorter one is sung legato (spec §8).
    let (rest_lo, rest_hi) = VocalParams::REST_THRESHOLD_BEATS;
    let rest_label = dim_label(format!(
        "Rest after gap · {:.3} beat",
        params.rest_threshold_beats
    ));
    let rest_slider = slider(rest_lo..=rest_hi, params.rest_threshold_beats, move |v| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetVocalRestThresholdBeats(v),
        })
    })
    .step(0.125)
    .width(Length::Fill);

    // Velocity → loudness: the bank's energy curve where it has one, and
    // a ±6 dB per-note gain on every bank (spec §9).
    let dynamics_label = dim_label(format!(
        "Velocity \u{2192} loudness · {:.2}",
        params.velocity_dynamics
    ));
    let dynamics_slider = slider(0.0..=1.0, params.velocity_dynamics, move |v| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetVocalVelocityDynamics(v),
        })
    })
    .step(0.05)
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
        voicebank_picker,
    ]
    .spacing(0);
    if let Some(speaker) = speaker_block {
        body = body.push(Space::new().height(8)).push(speaker);
    }
    // The bank's own duration / pitch models: each toggle shows only when
    // the bank ships that model (a bank without one sings with the rules
    // either way).
    if models.timing || models.pitch {
        body = body.push(Space::new().height(6));
    }
    if models.timing {
        body = body.push(toggle_row(
            "Timing from the voicebank",
            params.bank_timing,
            LaneInspectorMsg::ToggleVocalBankTiming,
            definition_id,
            track_id,
        ));
    }
    if models.pitch {
        body = body.push(toggle_row(
            "Pitch from the voicebank",
            params.bank_pitch,
            LaneInspectorMsg::ToggleVocalBankPitch,
            definition_id,
            track_id,
        ));
    }
    body = body.push(Space::new().height(8));
    if bank_sings_pitch {
        body = body.push(
            text("Vibrato and portamento: the voicebank sings its own")
                .size(10)
                .font(theme::SERIF_ITALIC_FONT)
                .color(theme::TEXT_3),
        );
        body = body.push(Space::new().height(4));
    }
    let body = body
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
        .push(consonant_slider)
        .push(Space::new().height(6))
        .push(rest_label)
        .push(Space::new().height(2))
        .push(rest_slider)
        .push(Space::new().height(6))
        .push(dynamics_label)
        .push(Space::new().height(2))
        .push(dynamics_slider);

    group_card(body.into())
}

/// The Voicebank picker (installed English banks; a lane on a built-in
/// bank that is not installed shows it as `<id> (not installed)`, any
/// other missing bank as the placeholder) and, for a bank with more than
/// one speaker, the Speaker picker. Every choice comes from the rail's
/// cached lists, so building the pickers allocates nothing.
fn voicebank_pickers<'a>(
    definition_id: u64,
    track_id: TrackId,
    params: &VocalParams,
    vocal: &VocalRailContext,
) -> (Element<'a, Message>, Option<Element<'a, Message>>, BankModels) {
    let lane_bank = params.voicebank.as_str();
    let selected = vocal
        .voicebanks
        .iter()
        .chain(vocal.missing_voicebanks.iter())
        .find(|c| &*c.id == lane_bank)
        .cloned();
    let speakers = selected.as_ref().map(|c| c.speakers.clone());
    let models = BankModels {
        timing: selected.as_ref().is_some_and(|c| c.has_timing_model),
        pitch: selected.as_ref().is_some_and(|c| c.has_pitch_model),
    };
    let bank_picker = pick_list(
        ChoiceList::Cached(vocal.voicebanks.clone()),
        selected,
        move |choice: VoicebankChoice| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetVocalVoicebank(resonance_music_theory::VoicebankId::new(
                    &*choice.id,
                )),
            })
        },
    )
    .placeholder(if vocal.voicebanks.is_empty() {
        "No voicebank installed"
    } else {
        "Voicebank not installed"
    })
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);
    // The voicebank library (installed banks, plok.org downloads).
    let library = button(text("Library\u{2026}").size(11).color(theme::TEXT))
        .on_press(Message::VoicebankLibrary(VoicebankLibraryMessage::Open {
            definition_id,
            track_id,
        }))
        .style(|_theme, status| theme::ghost_button_style(status))
        .padding([4, 8]);
    let bank_picker = row![bank_picker, library]
        .spacing(4)
        .align_y(iced::alignment::Vertical::Center);

    let Some(speakers) = speakers.filter(|s| s.len() > 1) else {
        return (bank_picker.into(), None, models);
    };
    let current = params
        .speaker
        .as_deref()
        .and_then(|id| speakers.iter().find(|s| s.id == id))
        .cloned();
    let speaker_picker = pick_list(
        ChoiceList::Cached(speakers),
        current,
        move |choice: SpeakerChoice| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetVocalSpeaker(choice.id),
            })
        },
    )
    .placeholder("Default")
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);
    let block = column![dim_label("Speaker"), Space::new().height(2), speaker_picker]
        .spacing(0)
        .into();
    (bank_picker.into(), Some(block), models)
}

/// Which of its own models the lane's bank ships.
#[derive(Debug, Clone, Copy)]
struct BankModels {
    timing: bool,
    pitch: bool,
}
