//! Right-rail inspector body for the Vocal lane generator. Mirrors the
//! prototype's songwriter flow: Lyrics → Lyric draft → Melody → Voice &
//! delivery → Generate. Warm (amber) accent matches the prototype, since
//! this is per-track (a track lane), not section-global.

use iced::widget::{button, column, container, row, text, text_editor, Space};
use iced::{alignment, Background, Border, Element, Length};

use resonance_audio::types::TrackId;
use resonance_music_theory::VocalParams;

use crate::compose::messages::LaneInspectorMsg;
use crate::compose::ComposeMessage;
use crate::message::*;
use crate::theme;

mod common;
mod draft_group;
mod generate_group;
mod lyrics_group;
mod melody_group;
mod voice_group;

use draft_group::draft_group;
use generate_group::generate_group;
use lyrics_group::lyrics_group;
use melody_group::melody_group;
use voice_group::voice_group;

/// Toggle row — warm-accent dot on the left when active, dim border when
/// off. Click anywhere on the row to toggle.
pub(in crate::view::compose::lane_inspector) fn toggle_row<'a>(
    label: impl Into<String>,
    on: bool,
    msg: LaneInspectorMsg,
    definition_id: u64,
    track_id: TrackId,
) -> Element<'a, Message> {
    let dot_color = if on { theme::WARM } else { theme::TEXT_4 };
    let dot = container(Space::new().width(Length::Fixed(6.0)).height(Length::Fixed(6.0))).style(move |_| {
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

/// The text of the FU-M11b warning: the section's vocal is rendered at
/// one tempo, but some placed bar plays at another.
pub(crate) fn tempo_warning_text(w: crate::update::compose::VocalTempoMismatch) -> String {
    format!(
        "Sung at {:.0} BPM (the first placement's tempo), but part of this \
         section plays at {:.0} BPM \u{2014} the vocal drifts off the grid there.",
        w.rendered_bpm, w.other_bpm
    )
}

/// Warm warning card (icon + bordered box, so it never relies on color
/// alone) shown above the vocal groups when the one render can't fit
/// every placed bar's tempo (FU-M11b).
fn tempo_warning_card<'a>(w: crate::update::compose::VocalTempoMismatch) -> Element<'a, Message> {
    let body = row![
        theme::icon(theme::fa::TRIANGLE_EXCLAMATION)
            .size(12)
            .color(theme::WARM),
        text(tempo_warning_text(w)).size(11.5).color(theme::TEXT_1),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Top);
    container(body)
        .padding([8, 10])
        .width(Length::Fill)
        .style(|_| container::Style {
            background: Some(Background::Color(iced::Color {
                a: 0.08,
                ..theme::WARM
            })),
            border: Border {
                color: theme::WARM_LINE,
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        })
        .into()
}

// ===========================================================================
// Main body
// ===========================================================================

pub(in crate::view::compose::lane_inspector) fn vocal_controls<'a>(
    definition_id: u64,
    track_id: TrackId,
    params: &VocalParams,
    seed: u64,
    bulk_content: Option<&'a text_editor::Content>,
    collapsed_panels: &std::collections::HashSet<crate::compose::RailPanelKey>,
    tempo_warning: Option<crate::update::compose::VocalTempoMismatch>,
) -> Element<'a, Message> {
    use crate::compose::RailPanelKey;

    // Each card collapses independently, keyed per track. The Generate
    // group is an action row, not content — always visible.
    let lyrics_group = lyrics_group(
        definition_id,
        track_id,
        params,
        collapsed_panels.contains(&RailPanelKey::VocalLyrics(track_id)),
    );
    let draft_group = draft_group(
        definition_id,
        track_id,
        params,
        bulk_content,
        collapsed_panels.contains(&RailPanelKey::VocalDraft(track_id)),
    );
    let melody_group = melody_group(
        definition_id,
        track_id,
        params,
        collapsed_panels.contains(&RailPanelKey::VocalMelody(track_id)),
    );
    let voice_group = voice_group(
        definition_id,
        track_id,
        params,
        collapsed_panels.contains(&RailPanelKey::VocalVoice(track_id)),
    );
    let generate_group = generate_group(definition_id, track_id, seed);

    let warning: Element<'a, Message> = match tempo_warning {
        Some(w) => column![tempo_warning_card(w), Space::new().height(18)].into(),
        None => Space::new().height(0).into(),
    };

    column![
        warning,
        lyrics_group,
        Space::new().height(18),
        draft_group,
        Space::new().height(18),
        melody_group,
        Space::new().height(18),
        voice_group,
        Space::new().height(18),
        generate_group,
    ]
    .spacing(0)
    .into()
}
