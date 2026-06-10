//! Melody group — voice type, range, style/contour/syllable chips,
//! anchor/leap/phrase/breath sliders, and the toggles for scale & motif.

use iced::widget::{column, pick_list, row, slider, Space};
use iced::{Element, Length};

use resonance_audio::types::TrackId;
use resonance_music_theory::{SyllableMode, VocalContour, VocalParams, VocalStyle};

use crate::compose::messages::LaneInspectorMsg;
use crate::compose::ComposeMessage;
use crate::message::*;

use super::common::{
    chip, dim_label, group_card, group_title, midi_to_name, phrase_length_options,
    range_high_options, range_low_options, voice_type_pick_options, VoiceTypePick,
};
use super::toggle_row;

pub(super) fn melody_group<'a>(
    definition_id: u64,
    track_id: TrackId,
    params: &'a VocalParams,
    collapsed: bool,
) -> Element<'a, Message> {
    let key = crate::compose::RailPanelKey::VocalMelody(track_id);
    let title = group_title("Melody", key, collapsed);
    if collapsed {
        return group_card(title);
    }

    let voice_picker = pick_list(
        voice_type_pick_options(),
        Some(VoiceTypePick(params.voice)),
        move |pick| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetVocalVoiceType(pick.0),
            })
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let range_low = pick_list(range_low_options(), Some(params.range.0), move |n| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetVocalRangeLow(n),
        })
    })
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);
    let range_high = pick_list(range_high_options(), Some(params.range.1), move |n| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetVocalRangeHigh(n),
        })
    })
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let voice_col = column![
        dim_label("Voice type"),
        Space::new().height(2),
        voice_picker
    ]
    .spacing(0);
    let range_col = column![
        dim_label(format!(
            "Range · {}\u{2013}{}",
            midi_to_name(params.range.0),
            midi_to_name(params.range.1)
        )),
        Space::new().height(2),
        row![range_low, range_high].spacing(6),
    ]
    .spacing(0);
    let voice_range_row = row![voice_col, range_col].spacing(10);

    // Style chips — picks the per-syllable generator dispatched in
    // `derive_vocal`. Wraps to two lines automatically on narrow rails.
    let style_chips: Vec<Element<'_, Message>> = VocalStyle::ALL
        .iter()
        .copied()
        .map(|s| {
            chip(
                s.as_str(),
                params.style == s,
                Message::Compose(ComposeMessage::LaneInspector {
                    definition_id,
                    track_id,
                    msg: LaneInspectorMsg::SetVocalStyle(s),
                }),
            )
        })
        .collect();
    let style_row = iced::widget::Row::with_children(style_chips).spacing(4).wrap();

    // Contour chips
    let mut contour_row = row![].spacing(4);
    for c in VocalContour::ALL.iter().copied() {
        contour_row = contour_row.push(chip(
            c.as_str().to_uppercase(),
            params.contour == c,
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetVocalContour(c),
            }),
        ));
    }

    // Note → syllable mode chips
    let mut syllable_row = row![].spacing(4);
    for m in SyllableMode::ALL.iter().copied() {
        syllable_row = syllable_row.push(chip(
            m.as_str(),
            params.syllable_mode == m,
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetVocalSyllableMode(m),
            }),
        ));
    }

    // Chord-tone anchor slider
    let anchor_label = dim_label(format!(
        "Anchor on chord tones · {}%",
        (params.chord_tone_anchor * 100.0).round() as u32
    ));
    let anchor_slider = slider(0.0..=1.0, params.chord_tone_anchor, move |v| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetVocalChordToneAnchor(v),
        })
    })
    .step(0.01)
    .width(Length::Fill);

    // Leap range + phrase length
    let leap_label = dim_label(format!("Leap range · {:.2}", params.leap_range));
    let leap_slider = slider(0.0..=1.0, params.leap_range, move |v| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetVocalLeapRange(v),
        })
    })
    .step(0.01)
    .width(Length::Fill);
    let phrase_picker = pick_list(
        phrase_length_options(),
        Some(params.phrase_length_bars),
        move |n| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetVocalPhraseLength(n),
            })
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);
    let leap_col = column![leap_label, Space::new().height(2), leap_slider].spacing(0);
    let phrase_col = column![
        dim_label("Phrase length · bars"),
        Space::new().height(2),
        phrase_picker
    ]
    .spacing(0);
    let leap_phrase_row = row![leap_col, phrase_col].spacing(10);

    // Breath slider
    let breath_label = dim_label(format!("Breath between phrases · {:.2}", params.breath));
    let breath_slider = slider(0.0..=1.0, params.breath, move |v| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetVocalBreath(v),
        })
    })
    .step(0.01)
    .width(Length::Fill);

    let stay_in_scale_toggle = toggle_row(
        "Stay in section scale",
        params.stay_in_scale,
        LaneInspectorMsg::ToggleVocalStayInScale,
        definition_id,
        track_id,
    );
    let avoid_clash_toggle = toggle_row(
        "Avoid clashes with lead synth",
        params.avoid_clashes,
        LaneInspectorMsg::ToggleVocalAvoidClashes,
        definition_id,
        track_id,
    );
    let use_motif_toggle = toggle_row(
        "Use section motif for pitches",
        params.use_section_motif,
        LaneInspectorMsg::ToggleVocalUseSectionMotif,
        definition_id,
        track_id,
    );

    let body = column![
        title,
        Space::new().height(8),
        voice_range_row,
        Space::new().height(8),
        dim_label("Style"),
        Space::new().height(2),
        style_row,
        Space::new().height(8),
        dim_label("Phrase contour"),
        Space::new().height(2),
        contour_row,
        Space::new().height(8),
        dim_label("Note \u{2192} syllable"),
        Space::new().height(2),
        syllable_row,
        Space::new().height(8),
        anchor_label,
        Space::new().height(2),
        anchor_slider,
        Space::new().height(8),
        leap_phrase_row,
        Space::new().height(8),
        breath_label,
        Space::new().height(2),
        breath_slider,
        Space::new().height(8),
        stay_in_scale_toggle,
        Space::new().height(4),
        avoid_clash_toggle,
        Space::new().height(4),
        use_motif_toggle,
    ]
    .spacing(0);

    group_card(body.into())
}
