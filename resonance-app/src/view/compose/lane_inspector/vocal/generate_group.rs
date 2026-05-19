//! Generate group — primary "Generate melody + audio" action, the
//! re-render-audio path that preserves edits, lyrics/melody-only
//! fallbacks, plus the seed line for determinism inspection.

use iced::widget::{button, column, row, text, Space};
use iced::{alignment, Background, Border, Color, Element, Length};

use resonance_audio::types::TrackId;

use crate::compose::messages::LaneInspectorMsg;
use crate::compose::ComposeMessage;
use crate::message::*;
use crate::theme;

pub(super) fn generate_group<'a>(
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
        Space::new().height(6),
        rerender_audio,
        Space::new().height(4),
        edit_hint,
        Space::new().height(8),
        secondary_row,
        Space::new().height(8),
        seed_line,
    ]
    .spacing(0)
    .into()
}
