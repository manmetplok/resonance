//! Lyrics group — the first section of the vocal inspector. Theme,
//! mood/POV, rhyme scheme, line/syllable counts, and the toggles for
//! matching syllables to the melody / avoiding clichés.

use iced::widget::{column, pick_list, row, text, text_input, Space};
use iced::{Element, Length};

use resonance_audio::types::TrackId;
use resonance_music_theory::{VocalParams, VocalRhymeScheme};

use crate::compose::messages::LaneInspectorMsg;
use crate::compose::ComposeMessage;
use crate::message::*;
use crate::theme;

use super::common::{
    chip, dim_label, group_card, group_title, line_count_options, mood_pick_options,
    pov_pick_options, syllable_max_options, syllable_min_options, MoodPick, PovPick,
};
use super::toggle_row;

pub(super) fn lyrics_group<'a>(
    definition_id: u64,
    track_id: TrackId,
    params: &'a VocalParams,
) -> Element<'a, Message> {
    let title = group_title("Lyrics");

    let theme_label = dim_label("Theme / prompt");
    let theme_input = text_input("e.g. fragile glass houses", &params.theme)
        .on_input(move |s| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetVocalTheme(s),
            })
        })
        .size(12)
        .padding([6, 8])
        .width(Length::Fill);
    let theme_meta = text(format!("{} / 240", params.theme.chars().count()))
        .size(9)
        .color(theme::TEXT_4)
        .font(theme::MONO_FONT);

    let mood_picker = pick_list(
        mood_pick_options(),
        Some(MoodPick(params.mood)),
        move |pick| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetVocalMood(pick.0),
            })
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let pov_picker = pick_list(
        pov_pick_options(),
        Some(PovPick(params.pov)),
        move |pick| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetVocalPov(pick.0),
            })
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let mood_col = column![dim_label("Mood"), Space::with_height(2), mood_picker].spacing(0);
    let pov_col = column![
        dim_label("Voice / POV"),
        Space::with_height(2),
        pov_picker
    ]
    .spacing(0);

    let mood_pov_row = row![mood_col, pov_col].spacing(10);

    // Rhyme-scheme chips
    let mut rhyme_row = row![].spacing(4);
    for scheme in VocalRhymeScheme::ALL.iter().copied() {
        rhyme_row = rhyme_row.push(chip(
            scheme.as_str(),
            params.rhyme == scheme,
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetVocalRhyme(scheme),
            }),
        ));
    }

    // Lines + syllables steppers (rendered as plain pick_lists of u8
    // ranges). Option vecs come from the cached statics so we don't
    // re-allocate them on every repaint.
    let lines_picker = pick_list(line_count_options(), Some(params.lines), move |n| {
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::SetVocalLines(n),
        })
    })
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let syl_min_picker = pick_list(
        syllable_min_options(),
        Some(params.syllables_min),
        move |n| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetVocalSyllablesMin(n),
            })
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);
    let syl_max_picker = pick_list(
        syllable_max_options(),
        Some(params.syllables_max),
        move |n| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetVocalSyllablesMax(n),
            })
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let lines_col = column![dim_label("Lines"), Space::with_height(2), lines_picker].spacing(0);
    let syl_col = column![
        dim_label(format!(
            "Syllables / line · {}\u{2013}{}",
            params.syllables_min, params.syllables_max
        )),
        Space::with_height(2),
        row![syl_min_picker, syl_max_picker].spacing(6),
    ]
    .spacing(0);

    let counts_row = row![lines_col, syl_col].spacing(10);

    let match_toggle = toggle_row(
        "Match syllables to melody",
        params.match_syllables_to_melody,
        LaneInspectorMsg::ToggleVocalMatchSyllables,
        definition_id,
        track_id,
    );
    let cliche_toggle = toggle_row(
        "Avoid clichés in this genre",
        params.avoid_cliches,
        LaneInspectorMsg::ToggleVocalAvoidCliches,
        definition_id,
        track_id,
    );

    let body = column![
        title,
        Space::with_height(8),
        theme_label,
        Space::with_height(2),
        theme_input,
        Space::with_height(2),
        row![Space::with_width(Length::Fill), theme_meta],
        Space::with_height(8),
        mood_pov_row,
        Space::with_height(8),
        dim_label("Rhyme scheme"),
        Space::with_height(2),
        rhyme_row,
        Space::with_height(8),
        counts_row,
        Space::with_height(8),
        match_toggle,
        Space::with_height(4),
        cliche_toggle,
    ]
    .spacing(0);

    group_card(body.into())
}
