//! The "Melody predates chord change" badge
//! (vocal-melody-on-chord-change): a vocal lane whose melody a chord or
//! section-length edit kept shows it in the lane inspector (a card) and
//! the vocal-roll toolbar (a chip), with Re-fit and Regenerate buttons,
//! until the lane is re-fitted, regenerated or its notes are edited. An
//! icon and words, never colour alone.

use iced::widget::{button, column, container, row, text, Space};
use iced::{alignment, Background, Border, Element, Length};

use resonance_audio::types::TrackId;

use crate::compose::vocal_melody::VocalMelodyMsg;
use crate::compose::{ComposeMessage, LaneMelodyFlags};
use crate::message::Message;
use crate::theme;

/// The badge's label.
pub(crate) const LABEL: &str = "MELODY PREDATES CHORD CHANGE";

fn msg(m: VocalMelodyMsg) -> Message {
    Message::Compose(ComposeMessage::VocalMelody(m))
}

fn label_chip<'a>() -> Element<'a, Message> {
    let body = row![
        theme::icon(theme::fa::TRIANGLE_EXCLAMATION)
            .size(10)
            .color(theme::WARM),
        text(LABEL).size(10).font(theme::UI_FONT_SEMIBOLD).color(theme::WARM),
    ]
    .spacing(6)
    .align_y(alignment::Vertical::Center);
    container(body)
        .padding([2, 8])
        .style(|_| container::Style {
            background: Some(Background::Color(theme::WARM_DIM)),
            border: Border {
                color: theme::WARM_LINE,
                width: 1.0,
                radius: theme::RADIUS_PILL.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Re-fit and Regenerate for the lane.
fn actions<'a>(definition_id: u64, track_id: TrackId) -> Element<'a, Message> {
    let action = |label: &'static str, m: VocalMelodyMsg| {
        button(text(label).size(11))
            .on_press(msg(m))
            .padding([3, 10])
            .style(|_theme, status| theme::ghost_button_style(status))
    };
    row![
        action(
            "Re-fit",
            VocalMelodyMsg::Refit {
                definition_id,
                track_id
            }
        ),
        action(
            "Regenerate",
            VocalMelodyMsg::Regenerate {
                definition_id,
                track_id
            }
        ),
    ]
    .spacing(6)
    .into()
}

/// The lane inspector's card: chip, what it means, and the two actions;
/// `None` while the melody is current.
pub(crate) fn card<'a>(
    definition_id: u64,
    track_id: TrackId,
    flags: LaneMelodyFlags,
) -> Option<Element<'a, Message>> {
    if !flags.melody_predates_chords {
        return None;
    }
    let detail = text(
        "The chords changed under this melody and it was kept. Re-fit moves its pitches \
         onto the new chords (rhythm and lyrics stay); Regenerate writes a new melody.",
    )
    .size(11)
    .color(theme::TEXT_1);
    let body = column![label_chip(), detail, actions(definition_id, track_id)].spacing(8);
    Some(
        container(body)
            .padding([8, 10])
            .width(Length::Fill)
            .style(|_| container::Style {
                background: Some(Background::Color(theme::WARM_DIM)),
                border: Border {
                    color: theme::WARM_LINE,
                    width: 1.0,
                    radius: theme::RADIUS_MD.into(),
                },
                ..Default::default()
            })
            .into(),
    )
}

/// The vocal-roll toolbar's one-line form; `None` while current.
pub(crate) fn chip<'a>(
    definition_id: u64,
    track_id: TrackId,
    flags: LaneMelodyFlags,
) -> Option<Element<'a, Message>> {
    flags.melody_predates_chords.then(|| {
        row![
            label_chip(),
            Space::new().width(4),
            actions(definition_id, track_id)
        ]
        .align_y(alignment::Vertical::Center)
        .into()
    })
}
