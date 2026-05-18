//! Lyric draft preview — bulk multi-line editor + per-line preview rows.

use iced::widget::{button, column, container, row, text, text_editor, text_input, Space};
use iced::{alignment, Background, Border, Color, Element, Length};

use resonance_audio::types::TrackId;
use resonance_music_theory::VocalParams;

use crate::compose::messages::LaneInspectorMsg;
use crate::compose::ComposeMessage;
use crate::message::*;
use crate::theme;

use super::common::{dim_label, group_card, rail_dot_warm, warm_chip_inactive};

pub(super) fn draft_group<'a>(
    definition_id: u64,
    track_id: TrackId,
    params: &'a VocalParams,
    bulk_content: Option<&'a text_editor::Content>,
) -> Element<'a, Message> {
    let title_left = row![
        rail_dot_warm(),
        text("Lyric draft").size(11).font(theme::UI_FONT_SEMIBOLD).color(theme::WARM),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);
    let title_right = text(format!("{} · {} LINES", params.rhyme.as_str(), params.draft.len()))
        .size(9)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_4);
    let title = row![
        title_left,
        Space::with_width(Length::Fill),
        title_right
    ]
    .align_y(alignment::Vertical::Center);

    let bulk_block = bulk_lyrics_block(definition_id, track_id, params, bulk_content);

    let mut lines_col = column![].spacing(6);
    for line in &params.draft {
        lines_col = lines_col.push(lyric_line_row(definition_id, track_id, line));
    }

    let reroll = warm_chip_inactive("\u{21BB} Re-roll unlocked").on_press(Message::Compose(
        ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::RerollUnlockedLyrics,
        },
    ));
    let auto_syl = warm_chip_inactive("Auto-syllabify").on_press(Message::Compose(
        ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::AutoSyllabifyLyrics,
        },
    ));
    let rhyme_assist = warm_chip_inactive("Rhyme assist");
    let edit = warm_chip_inactive("Edit");

    let actions = row![reroll, auto_syl, rhyme_assist, edit].spacing(6);

    let body = column![
        title,
        Space::with_height(8),
        bulk_block,
        Space::with_height(10),
        dim_label("Per-line preview"),
        Space::with_height(4),
        lines_col,
        Space::with_height(8),
        actions,
    ]
    .spacing(0);

    group_card(body.into())
}

/// Multi-line text editor that lets the user type a whole section's
/// lyrics at once. Each non-empty line in the buffer maps to one
/// `LyricLine`; the per-line preview below stays in sync. The Content
/// is materialised in the update layer the moment a vocal lane is
/// selected, so by the time this view runs the `Some(_)` branch is
/// taken. The `None` branch renders a quiet placeholder for the brief
/// window before that materialisation completes.
fn bulk_lyrics_block<'a>(
    definition_id: u64,
    track_id: TrackId,
    params: &'a VocalParams,
    bulk_content: Option<&'a text_editor::Content>,
) -> Element<'a, Message> {
    let height = Length::Fixed(((params.lines.max(4) as f32) * 22.0).min(220.0));
    let editor: Element<'a, Message> = match bulk_content {
        Some(content) => text_editor(content)
            .placeholder("Type the section's lyrics — one line per lyric line…")
            .on_action(move |action| {
                Message::Compose(ComposeMessage::LaneInspector {
                    definition_id,
                    track_id,
                    msg: LaneInspectorMsg::VocalBulkLyricsAction(action),
                })
            })
            .padding(8)
            .height(height)
            .into(),
        None => container(
            text("Loading lyrics editor…")
                .size(11)
                .color(theme::TEXT_3),
        )
        .padding(8)
        .height(height)
        .width(Length::Fill)
        .into(),
    };

    column![dim_label("All lyrics"), Space::with_height(4), editor].spacing(0).into()
}

fn lyric_line_row<'a>(
    definition_id: u64,
    track_id: TrackId,
    line: &'a resonance_music_theory::LyricLine,
) -> Element<'a, Message> {
    let n = text(format!("{}.", line.n))
        .size(10)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_3);

    // Rhyme tag chip — color-coded by letter.
    let tag_color = match line.rhyme {
        'A' => theme::WARM,
        'B' => theme::ACCENT_SOFT,
        'C' => theme::GOOD,
        _ => theme::TEXT_2,
    };
    let tag = container(text(line.rhyme.to_string()).size(9).color(tag_color))
        .padding([1, 6])
        .style(move |_| container::Style {
            background: Some(Background::Color(Color { a: 0.16, ..tag_color })),
            border: Border {
                color: Color { a: 0.40, ..tag_color },
                width: 1.0,
                radius: 999.0.into(),
            },
            ..Default::default()
        });

    let syl = text(format!("{} syl", line.syllables))
        .size(9)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_4);

    let line_n_for_input = line.n;
    let lyric = text_input("type a line…", &line.text)
        .on_input(move |s| {
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetVocalLineText(line_n_for_input, s),
            })
        })
        .size(12)
        .padding([4, 6])
        .width(Length::Fill);

    let lock_color = if line.locked { theme::WARM } else { theme::TEXT_4 };
    let line_n = line.n;
    let lock_btn = button(
        text(if line.locked { "\u{25CF}" } else { "\u{25CB}" })
            .size(11)
            .color(lock_color),
    )
    .padding([2, 4])
    .style(|_, status| {
        let bg = match status {
            button::Status::Hovered => theme::BG_3,
            _ => Color::TRANSPARENT,
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: theme::TEXT_1,
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        }
    })
    .on_press(Message::Compose(ComposeMessage::LaneInspector {
        definition_id,
        track_id,
        msg: LaneInspectorMsg::ToggleVocalLockLine(line_n),
    }));

    row![n, tag, syl, lyric, lock_btn]
        .spacing(6)
        .align_y(alignment::Vertical::Center)
        .into()
}
