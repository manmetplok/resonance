//! The vocal rail's notices above its groups: the tempo-mismatch warning
//! (FU-M11b) and the "melody predates chord change" badge with its Re-fit
//! / Regenerate actions (vocal-melody-on-chord-change).

use iced::widget::{column, container, row, text, Space};
use iced::{alignment, Background, Border, Element, Length};

use resonance_audio::types::TrackId;

use crate::compose::LaneMelodyFlags;
use crate::message::*;
use crate::theme;

/// The tempo warning card and its gap, or an empty slot.
pub(super) fn tempo_warning<'a>(
    warning: Option<crate::update::compose::VocalTempoMismatch>,
) -> Element<'a, Message> {
    match warning {
        Some(w) => column![tempo_warning_card(w), Space::new().height(18)].into(),
        None => Space::new().height(0).into(),
    }
}

/// The out-of-date melody badge and its gap, or an empty slot.
pub(super) fn melody_notice<'a>(
    definition_id: u64,
    track_id: TrackId,
    flags: LaneMelodyFlags,
) -> Element<'a, Message> {
    match crate::view::compose::vocal_melody_badge::card(definition_id, track_id, flags) {
        Some(card) => column![card, Space::new().height(18)].into(),
        None => Space::new().height(0).into(),
    }
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
