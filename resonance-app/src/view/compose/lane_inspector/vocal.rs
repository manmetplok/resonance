//! Right-rail inspector body for the Vocal lane generator. Mirrors the
//! prototype's songwriter flow: Lyrics → Lyric draft → Melody → Voice &
//! delivery → Generate. Warm (amber) accent matches the prototype, since
//! this is per-track (a track lane), not section-global.

use std::sync::OnceLock;

use iced::widget::{
    button, column, container, pick_list, row, slider, text, text_editor, text_input, Space,
};
use iced::{alignment, Background, Border, Color, Element, Length};

use resonance_audio::types::TrackId;
use resonance_music_theory::{
    SyllableMode, VocalContour, VocalMood, VocalParams, VocalPov, VocalRhymeScheme, VocalSinger,
    VocalSingerMeiji, VocalStyle, VocalTimbre, VocalVoicebank, VoiceType,
};

use crate::compose::messages::LaneInspectorMsg;
use crate::compose::ComposeMessage;
use crate::message::*;
use crate::theme;

// ===========================================================================
// Cached pick_list option vectors.
// ===========================================================================
//
// `pick_list` takes the option list by value (a `Cow`-style slice). Rebuilding
// these every frame allocates a fresh `Vec<u8>` per dropdown; the inspector
// shows six of them, so that's six vec allocs per repaint while the user
// twiddles a slider. These statics are populated on first access and reused
// thereafter. See view-layer performance memory for the codebase rule.

fn line_count_options() -> &'static [u8] {
    static V: OnceLock<Vec<u8>> = OnceLock::new();
    V.get_or_init(|| (1..=8).collect())
}
fn syllable_min_options() -> &'static [u8] {
    static V: OnceLock<Vec<u8>> = OnceLock::new();
    V.get_or_init(|| (3..=20).collect())
}
fn syllable_max_options() -> &'static [u8] {
    static V: OnceLock<Vec<u8>> = OnceLock::new();
    V.get_or_init(|| (3..=24).collect())
}
fn range_low_options() -> &'static [u8] {
    static V: OnceLock<Vec<u8>> = OnceLock::new();
    V.get_or_init(|| (36..=72).collect())
}
fn range_high_options() -> &'static [u8] {
    static V: OnceLock<Vec<u8>> = OnceLock::new();
    V.get_or_init(|| (48..=96).collect())
}
fn phrase_length_options() -> &'static [u8] {
    static V: OnceLock<Vec<u8>> = OnceLock::new();
    V.get_or_init(|| (1..=8).collect())
}
fn mood_pick_options() -> &'static [MoodPick] {
    static V: OnceLock<Vec<MoodPick>> = OnceLock::new();
    V.get_or_init(|| VocalMood::ALL.iter().map(|m| MoodPick(*m)).collect())
}
fn pov_pick_options() -> &'static [PovPick] {
    static V: OnceLock<Vec<PovPick>> = OnceLock::new();
    V.get_or_init(|| VocalPov::ALL.iter().map(|p| PovPick(*p)).collect())
}
fn voice_type_pick_options() -> &'static [VoiceTypePick] {
    static V: OnceLock<Vec<VoiceTypePick>> = OnceLock::new();
    V.get_or_init(|| VoiceType::ALL.iter().map(|v| VoiceTypePick(*v)).collect())
}

// ===========================================================================
// Display newtypes — needed so pick_lists can show pretty labels without
// disturbing the underlying enums.
// ===========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MoodPick(VocalMood);
impl std::fmt::Display for MoodPick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PovPick(VocalPov);
impl std::fmt::Display for PovPick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VoiceTypePick(VoiceType);
impl std::fmt::Display for VoiceTypePick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

// ===========================================================================
// Style helpers
// ===========================================================================

fn rail_dot_warm<'a>() -> Element<'a, Message> {
    container(Space::new(Length::Fixed(6.0), Length::Fixed(6.0)))
        .style(|_| container::Style {
            background: Some(Background::Color(theme::WARM)),
            border: Border {
                color: theme::WARM,
                width: 0.0,
                radius: 3.0.into(),
            },
            ..Default::default()
        })
        .into()
}

fn group_title<'a>(label: &'static str) -> Element<'a, Message> {
    row![
        rail_dot_warm(),
        text(label).size(11).font(theme::UI_FONT_SEMIBOLD).color(theme::WARM),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center)
    .into()
}

fn dim_label<'a>(s: impl Into<String>) -> Element<'a, Message> {
    text(s.into()).size(10).color(theme::TEXT_3).into()
}

fn warm_chip_active<'a>(label: impl Into<String>) -> button::Button<'a, Message> {
    button(
        text(label.into())
            .size(10)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::WARM),
    )
    .padding([4, 8])
    .style(|_, status| {
        let bg = match status {
            button::Status::Hovered => Color { a: 0.24, ..theme::WARM },
            _ => Color { a: 0.18, ..theme::WARM },
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: theme::WARM,
            border: Border {
                color: theme::WARM,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        }
    })
}

fn warm_chip_inactive<'a>(label: impl Into<String>) -> button::Button<'a, Message> {
    button(text(label.into()).size(10).color(theme::TEXT_2))
        .padding([4, 8])
        .style(|_, status| {
            let bg = match status {
                button::Status::Hovered => theme::BG_3,
                _ => theme::BG_2,
            };
            button::Style {
                background: Some(Background::Color(bg)),
                text_color: theme::TEXT_2,
                border: Border {
                    color: theme::LINE_2,
                    width: 1.0,
                    radius: theme::RADIUS_SM.into(),
                },
                ..Default::default()
            }
        })
}

fn chip<'a>(
    label: impl Into<String>,
    active: bool,
    on_press: Message,
) -> Element<'a, Message> {
    if active {
        warm_chip_active(label).on_press(on_press).into()
    } else {
        warm_chip_inactive(label).on_press(on_press).into()
    }
}

fn group_card<'a>(content: Element<'a, Message>) -> Element<'a, Message> {
    container(content)
        .padding(12)
        .width(Length::Fill)
        .style(|_| container::Style {
            background: Some(Background::Color(theme::BG_2)),
            border: Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_XL.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Toggle row — warm-accent dot on the left when active, dim border when
/// off. Click anywhere on the row to toggle.
pub(super) fn toggle_row<'a>(
    label: impl Into<String>,
    on: bool,
    msg: LaneInspectorMsg,
    definition_id: u64,
    track_id: TrackId,
) -> Element<'a, Message> {
    let dot_color = if on { theme::WARM } else { theme::TEXT_4 };
    let dot = container(Space::new(Length::Fixed(6.0), Length::Fixed(6.0))).style(move |_| {
        container::Style {
            background: Some(Background::Color(dot_color)),
            border: Border {
                color: dot_color,
                width: 0.0,
                radius: 3.0.into(),
            },
            ..Default::default()
        }
    });
    let label_text = text(label.into())
        .size(11)
        .color(if on { theme::TEXT_1 } else { theme::TEXT_3 });

    button(
        row![dot, label_text]
            .spacing(8)
            .align_y(alignment::Vertical::Center)
            .width(Length::Fill),
    )
    .padding([6, 8])
    .width(Length::Fill)
    .on_press(Message::Compose(ComposeMessage::LaneInspector {
        definition_id,
        track_id,
        msg,
    }))
    .style(move |_, status| {
        let bg = match status {
            button::Status::Hovered => theme::BG_3,
            _ => theme::BG_2,
        };
        let border = if on { theme::WARM_LINE } else { theme::LINE_2 };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: theme::TEXT_1,
            border: Border {
                color: border,
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        }
    })
    .into()
}

// ===========================================================================
// Main body
// ===========================================================================

pub(super) fn vocal_controls<'a>(
    definition_id: u64,
    track_id: TrackId,
    params: &'a VocalParams,
    seed: u64,
    bulk_content: Option<&'a text_editor::Content>,
) -> Element<'a, Message> {
    let lyrics_group = lyrics_group(definition_id, track_id, params);
    let draft_group = draft_group(definition_id, track_id, params, bulk_content);
    let melody_group = melody_group(definition_id, track_id, params);
    let voice_group = voice_group(definition_id, track_id, params);
    let generate_group = generate_group(definition_id, track_id, seed);

    column![
        lyrics_group,
        Space::with_height(10),
        draft_group,
        Space::with_height(10),
        melody_group,
        Space::with_height(10),
        voice_group,
        Space::with_height(10),
        generate_group,
    ]
    .spacing(0)
    .into()
}

// ---------------------------------------------------------------------------
// 1. Lyrics
// ---------------------------------------------------------------------------

fn lyrics_group<'a>(
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

// ---------------------------------------------------------------------------
// 2. Lyric draft preview
// ---------------------------------------------------------------------------

fn draft_group<'a>(
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

// ---------------------------------------------------------------------------
// 3. Melody
// ---------------------------------------------------------------------------

fn melody_group<'a>(
    definition_id: u64,
    track_id: TrackId,
    params: &'a VocalParams,
) -> Element<'a, Message> {
    let title = group_title("Melody");

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
        Space::with_height(2),
        voice_picker
    ]
    .spacing(0);
    let range_col = column![
        dim_label(format!(
            "Range · {}\u{2013}{}",
            midi_to_name(params.range.0),
            midi_to_name(params.range.1)
        )),
        Space::with_height(2),
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
    let leap_col = column![leap_label, Space::with_height(2), leap_slider].spacing(0);
    let phrase_col = column![
        dim_label("Phrase length · bars"),
        Space::with_height(2),
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
        Space::with_height(8),
        voice_range_row,
        Space::with_height(8),
        dim_label("Style"),
        Space::with_height(2),
        style_row,
        Space::with_height(8),
        dim_label("Phrase contour"),
        Space::with_height(2),
        contour_row,
        Space::with_height(8),
        dim_label("Note \u{2192} syllable"),
        Space::with_height(2),
        syllable_row,
        Space::with_height(8),
        anchor_label,
        Space::with_height(2),
        anchor_slider,
        Space::with_height(8),
        leap_phrase_row,
        Space::with_height(8),
        breath_label,
        Space::with_height(2),
        breath_slider,
        Space::with_height(8),
        stay_in_scale_toggle,
        Space::with_height(4),
        avoid_clash_toggle,
        Space::with_height(4),
        use_motif_toggle,
    ]
    .spacing(0);

    group_card(body.into())
}

// ---------------------------------------------------------------------------
// 4. Voice & delivery
// ---------------------------------------------------------------------------

fn voice_group<'a>(
    definition_id: u64,
    track_id: TrackId,
    params: &'a VocalParams,
) -> Element<'a, Message> {
    let title = group_title("Voice & delivery");

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
                column![dim_label("Singer"), Space::with_height(2), row]
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
                column![dim_label("Mode"), Space::with_height(2), row]
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
        Space::with_width(Length::Fill),
        text("staccato \u{00B7}")
            .size(9)
            .font(theme::SERIF_ITALIC_FONT)
            .color(theme::TEXT_4),
    ];

    let mut body = column![
        title,
        Space::with_height(8),
        dim_label("Timbre"),
        Space::with_height(2),
        timbre_row,
        Space::with_height(8),
        dim_label("Voicebank"),
        Space::with_height(2),
        voicebank_row,
    ]
    .spacing(0);
    if let Some(singer) = singer_block {
        body = body.push(Space::with_height(8)).push(singer);
    }
    let body = body
        .push(Space::with_height(8))
        .push(vibrato_label)
        .push(Space::with_height(2))
        .push(vibrato_slider)
        .push(Space::with_height(4))
        .push(vibrato_rate_label)
        .push(Space::with_height(2))
        .push(vibrato_rate_slider)
        .push(Space::with_height(6))
        .push(tension_label)
        .push(Space::with_height(2))
        .push(tension_slider)
        .push(Space::with_height(4))
        .push(tension_vel_label)
        .push(Space::with_height(2))
        .push(tension_vel_slider)
        .push(Space::with_height(4))
        .push(tension_contour_label)
        .push(Space::with_height(2))
        .push(tension_contour_slider)
        .push(Space::with_height(4))
        .push(portamento_label)
        .push(Space::with_height(2))
        .push(portamento_slider)
        .push(Space::with_height(6))
        .push(articulation_label)
        .push(Space::with_height(2))
        .push(articulation_slider)
        .push(Space::with_height(2))
        .push(stacc_legato_hint)
        .push(Space::with_height(6))
        .push(consonant_label)
        .push(Space::with_height(2))
        .push(consonant_slider);

    group_card(body.into())
}

// ---------------------------------------------------------------------------
// 5. Generate
// ---------------------------------------------------------------------------

fn generate_group<'a>(
    definition_id: u64,
    track_id: TrackId,
    seed: u64,
) -> Element<'a, Message> {
    // Primary action — full regenerate. Rolls fresh lyrics, re-derives
    // the melody from the section's chords, then renders audio. This
    // overwrites any hand-edits the user made in the vocal roll.
    let primary = button(
        text("Generate melody + audio")
            .size(12)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::BG_0),
    )
    .padding([8, 14])
    .width(Length::Fill)
    .style(|_, status| {
        let bg = match status {
            button::Status::Hovered => Color { a: 0.92, ..theme::WARM },
            _ => theme::WARM,
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: theme::BG_0,
            border: Border {
                color: theme::WARM,
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        }
    })
    .on_press(Message::Compose(ComposeMessage::LaneInspector {
        definition_id,
        track_id,
        msg: LaneInspectorMsg::GenerateVocalAll,
    }));

    // Audio re-render — preserves the user's edited notes/lyrics and
    // just runs the existing MIDI clip through the SVS pipeline again
    // (so changed delivery params like vibrato, portamento, timbre,
    // and any hand-drawn note edits become audible).
    let rerender_audio = button(
        text("Re-render audio")
            .size(11)
            .color(theme::WARM)
            .align_x(alignment::Horizontal::Center),
    )
    .padding([7, 10])
    .width(Length::Fill)
    .style(|_, status| {
        let bg = match status {
            button::Status::Hovered => Color { a: 0.18, ..theme::WARM },
            _ => Color { a: 0.10, ..theme::WARM },
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: theme::WARM,
            border: Border {
                color: Color { a: 0.55, ..theme::WARM },
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        }
    })
    .on_press(Message::Compose(ComposeMessage::LaneInspector {
        definition_id,
        track_id,
        msg: LaneInspectorMsg::RerenderVocalAudio,
    }));

    let lyrics_only = button(
        text("Lyrics only")
            .size(11)
            .color(theme::TEXT_2)
            .align_x(alignment::Horizontal::Center),
    )
    .padding([7, 10])
    .width(Length::Fill)
    .style(|_, status| {
        let bg = match status {
            button::Status::Hovered => theme::BG_3,
            _ => theme::BG_2,
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: theme::TEXT_2,
            border: Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        }
    })
    .on_press(Message::Compose(ComposeMessage::LaneInspector {
        definition_id,
        track_id,
        msg: LaneInspectorMsg::GenerateVocalLyricsOnly,
    }));

    let melody_only = button(
        text("Melody only")
            .size(11)
            .color(theme::TEXT_2)
            .align_x(alignment::Horizontal::Center),
    )
    .padding([7, 10])
    .width(Length::Fill)
    .style(|_, status| {
        let bg = match status {
            button::Status::Hovered => theme::BG_3,
            _ => theme::BG_2,
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: theme::TEXT_2,
            border: Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        }
    })
    .on_press(Message::Compose(ComposeMessage::LaneInspector {
        definition_id,
        track_id,
        msg: LaneInspectorMsg::GenerateVocalMelodyOnly,
    }));

    // Show the lane's actual config seed so the user can verify
    // determinism. The seed bumps on every regenerate, so the displayed
    // hex changes after each press.
    let seed_line = text(format!("seed · 0x{:X}", seed))
        .size(10)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_4);

    // Helper caption distinguishing the two render paths. The primary
    // bar overwrites any user edits in the vocal roll; the audio
    // re-render keeps them and just re-runs the SVS pipeline.
    let edit_hint = text(
        "Re-render keeps your edited notes \u{2014} regenerate replaces them.",
    )
    .size(10)
    .color(theme::TEXT_3);

    // Layout: primary action top, audio re-render directly under (the
    // pair that picks between "fresh take" and "audition my edits"),
    // then the lyrics/melody-only fallbacks, finally the seed line.
    let secondary_row = row![lyrics_only, melody_only].spacing(6);

    column![
        primary,
        Space::with_height(6),
        rerender_audio,
        Space::with_height(4),
        edit_hint,
        Space::with_height(8),
        secondary_row,
        Space::with_height(8),
        seed_line,
    ]
    .spacing(0)
    .into()
}

// ---------------------------------------------------------------------------
// Misc helpers
// ---------------------------------------------------------------------------

fn midi_to_name(n: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C\u{266f}", "D", "D\u{266f}", "E", "F", "F\u{266f}", "G", "G\u{266f}", "A",
        "A\u{266f}", "B",
    ];
    let octave = (n as i16 / 12) - 1;
    format!("{}{}", NAMES[(n % 12) as usize], octave)
}
