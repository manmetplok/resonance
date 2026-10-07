//! The vocal lane's render-state badge (vocal-mcp-control-and-quality.md
//! §7, D22), shown in the lane inspector above the vocal groups and in
//! the vocal-roll toolbar. A lane whose audio is current shows nothing;
//! the other states name themselves in words next to an icon, so none
//! relies on colour alone.
//!
//! - **Stale** — warm chip with how many phrases changed: the old take
//!   still plays until a render replaces it.
//! - **Rendering** — neutral chip while a render runs.
//! - **Error** — `BAD` chip naming the failure's kind; the inspector card
//!   adds its message underneath (the toolbar has one line).

use iced::widget::{container, row, text};
use iced::{alignment, Background, Border, Element, Length};

use crate::compose::{VocalLaneRenderState, VocalLaneRenderStatus};
use crate::message::Message;
use crate::theme;

/// The chip's label: `STALE · 2 of 5 phrases changed`, `RENDERING`,
/// `RENDER FAILED · no_notes`; `None` for a lane with nothing to flag.
pub(crate) fn badge_label(status: &VocalLaneRenderStatus) -> Option<String> {
    match status.state {
        VocalLaneRenderState::Stale if status.stale_units > 0 && status.total_units > 0 => {
            Some(format!(
                "STALE \u{00B7} {} of {} phrase{} changed",
                status.stale_units,
                status.total_units,
                if status.total_units == 1 { "" } else { "s" }
            ))
        }
        VocalLaneRenderState::Stale => Some("STALE".to_owned()),
        VocalLaneRenderState::Rendering => Some("RENDERING".to_owned()),
        VocalLaneRenderState::Error => Some(match &status.error {
            Some((kind, _)) => format!("RENDER FAILED \u{00B7} {}", kind.as_str()),
            None => "RENDER FAILED".to_owned(),
        }),
        VocalLaneRenderState::Rendered | VocalLaneRenderState::NotRendered => None,
    }
}

/// Colours and glyph for a state: `(text/icon, wash, outline, glyph)`.
fn palette(state: VocalLaneRenderState) -> (iced::Color, iced::Color, iced::Color, char) {
    match state {
        VocalLaneRenderState::Error => (
            theme::BAD,
            theme::BAD_DIM,
            theme::BAD_LINE,
            theme::fa::TRIANGLE_EXCLAMATION,
        ),
        VocalLaneRenderState::Rendering => {
            (theme::TEXT_2, theme::BG_3, theme::LINE, theme::fa::CLOCK)
        }
        _ => (
            theme::WARM,
            theme::WARM_DIM,
            theme::WARM_LINE,
            theme::fa::ARROW_ROTATE_RIGHT,
        ),
    }
}

/// One-line pill for a toolbar; `None` when there is nothing to flag.
pub(crate) fn render_chip<'a>(status: &VocalLaneRenderStatus) -> Option<Element<'a, Message>> {
    let label = badge_label(status)?;
    let (fg, wash, line, glyph) = palette(status.state);
    let body = row![
        theme::icon(glyph).size(10).color(fg),
        text(label).size(10).font(theme::UI_FONT_SEMIBOLD).color(fg),
    ]
    .spacing(6)
    .align_y(alignment::Vertical::Center);
    Some(
        container(body)
            .padding([2, 8])
            .style(move |_| container::Style {
                background: Some(Background::Color(wash)),
                border: Border {
                    color: line,
                    width: 1.0,
                    radius: 999.0.into(),
                },
                ..Default::default()
            })
            .into(),
    )
}

/// The inspector's badge: the chip, and under it what the state means —
/// for an error, the failure's own message.
pub(crate) fn render_card<'a>(status: &VocalLaneRenderStatus) -> Option<Element<'a, Message>> {
    let chip = render_chip(status)?;
    let (_, wash, line, _) = palette(status.state);
    let detail = match (&status.state, &status.error) {
        (VocalLaneRenderState::Error, Some((_, message))) => message.clone(),
        (VocalLaneRenderState::Error, None) => "The last render failed.".to_owned(),
        (VocalLaneRenderState::Rendering, _) => {
            "The current audio plays until the new render lands.".to_owned()
        }
        _ => "Plays the take from before the last edit until a render replaces it."
            .to_owned(),
    };
    let detail = text(detail).size(11).color(theme::TEXT_1);
    let body = iced::widget::column![chip, detail].spacing(6);
    Some(
        container(body)
            .padding([8, 10])
            .width(Length::Fill)
            .style(move |_| container::Style {
                background: Some(Background::Color(wash)),
                border: Border {
                    color: line,
                    width: 1.0,
                    radius: theme::RADIUS_MD.into(),
                },
                ..Default::default()
            })
            .into(),
    )
}
