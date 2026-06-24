//! Freeze status banners for the Arrange track header (design doc #181).
//!
//! Two non-blocking banners surface a frozen track's exceptional states.
//! Both are rendered as a single full-width strip — leading status glyph +
//! message, trailing action buttons — tinted with an existing semantic
//! colour rather than a new hue:
//!
//! - **Stale / refreeze** ([`FreezeStatus::Stale`]): an amber ([`theme::WARM`])
//!   banner, "Frozen audio is out of date", shown when a frozen input changed
//!   so the cache no longer matches the live track (ba todo #576 flips the
//!   track to `Stale`). The **Refreeze** primary re-renders the cache in place
//!   ([`FreezeMessage::RefreezeTrack`]); the **Unfreeze to edit** secondary
//!   drops the stale cache and restores live editing
//!   ([`FreezeMessage::UnfreezeTrack`]).
//! - **Freeze failed** ([`FreezeStatus::Failed`]): a soft-pink ([`theme::BAD`])
//!   banner, "Freeze failed — <reason>" (e.g. not enough disk space). The
//!   track has already fallen back to live with no cache attached, so no work
//!   is lost. **Retry** re-runs the freeze ([`FreezeMessage::FreezeTrack`]);
//!   **Dismiss** clears the failed status, also via
//!   [`FreezeMessage::UnfreezeTrack`] (a clean return to the fully-live state).
//!
//! The builders are pure functions of `(status, track_id)` so the banners can
//! be golden-snapshotted in isolation (see `tests/freeze_banner_render.rs`).

use iced::widget::{button, container, row, text, Space};
use iced::{alignment, Color, Element, Length};

use crate::message::*;
use crate::state::FreezeStatus;
use crate::theme::{self, fa};
use resonance_audio::types::TrackId;

/// Build the freeze-status banner for `status`, if it warrants one. Returns
/// `Some` only for the [`FreezeStatus::Stale`] (amber refreeze prompt) and
/// [`FreezeStatus::Failed`] (soft-pink freeze-failed notice) states; every
/// other status — `Idle` / `Freezing` / `Frozen` — renders nothing.
pub fn freeze_banner<'a>(
    status: &FreezeStatus,
    track_id: TrackId,
) -> Option<Element<'a, Message>> {
    match status {
        FreezeStatus::Stale { .. } => Some(stale_banner(track_id)),
        FreezeStatus::Failed { message } => Some(failed_banner(track_id, message)),
        FreezeStatus::Idle | FreezeStatus::Freezing { .. } | FreezeStatus::Frozen { .. } => None,
    }
}

/// Amber (`WARM`) "frozen audio is out of date" refreeze prompt. Refreeze is
/// the primary (warm-filled) action; "Unfreeze to edit" is the secondary
/// (ghost) escape hatch.
pub fn stale_banner<'a>(track_id: TrackId) -> Element<'a, Message> {
    banner_shell(
        fa::SNOWFLAKE,
        theme::WARM,
        "Frozen audio is out of date",
        row![
            action_button(
                "Refreeze",
                Message::Freeze(FreezeMessage::RefreezeTrack(track_id)),
                true,
                theme::WARM,
            ),
            action_button(
                "Unfreeze to edit",
                Message::Freeze(FreezeMessage::UnfreezeTrack(track_id)),
                false,
                theme::WARM,
            ),
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center)
        .into(),
    )
}

/// Soft-pink (`BAD`) "freeze failed — <reason>" notice. The track already
/// fell back to live (no work lost); Retry re-runs the freeze, Dismiss clears
/// the failed status.
pub fn failed_banner<'a>(track_id: TrackId, reason: &str) -> Element<'a, Message> {
    let label = format!("Freeze failed — {reason}");
    banner_shell(
        fa::CIRCLE_INFO,
        theme::BAD,
        label,
        row![
            action_button(
                "Retry",
                Message::Freeze(FreezeMessage::FreezeTrack(track_id)),
                true,
                theme::BAD,
            ),
            action_button(
                "Dismiss",
                Message::Freeze(FreezeMessage::UnfreezeTrack(track_id)),
                false,
                theme::BAD,
            ),
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center)
        .into(),
    )
}

/// Shared banner chrome: a tinted, edge-outlined strip with a leading glyph +
/// message and the supplied trailing `actions`. The background is a low-alpha
/// wash of `tint` and the border a stronger edge of the same colour, so the
/// banner reads as a semantic state (`WARM` / `BAD`) rather than a new colour.
fn banner_shell<'a>(
    glyph: char,
    tint: Color,
    message: impl text::IntoFragment<'a>,
    actions: Element<'a, Message>,
) -> Element<'a, Message> {
    let wash = Color { a: 0.12, ..tint };
    let edge = Color { a: 0.40, ..tint };
    container(
        row![
            theme::icon(glyph).size(11).color(tint),
            Space::new().width(7),
            text(message)
                .size(11)
                .color(theme::TEXT_1)
                .wrapping(iced::widget::text::Wrapping::None),
            Space::new().width(Length::Fill),
            actions,
        ]
        .align_y(alignment::Vertical::Center),
    )
    .width(Length::Fill)
    .padding(iced::Padding {
        top: 5.0,
        right: 6.0,
        bottom: 5.0,
        left: 9.0,
    })
    .style(move |_theme| container::Style {
        background: Some(iced::Background::Color(wash)),
        border: iced::Border {
            color: edge,
            width: 1.0,
            radius: theme::RADIUS_MD.into(),
        },
        ..Default::default()
    })
    .into()
}

/// A small text action button for a banner. `primary` buttons are filled with
/// the banner `tint` (dark text); secondary buttons use the shared ghost
/// style so the primary action stays visually dominant.
fn action_button<'a>(
    label: &'a str,
    on_press: Message,
    primary: bool,
    tint: Color,
) -> Element<'a, Message> {
    button(
        text(label)
            .size(11)
            .font(theme::UI_FONT_MEDIUM)
            .wrapping(iced::widget::text::Wrapping::None),
    )
    .on_press(on_press)
    .padding(iced::Padding {
        top: 3.0,
        right: 9.0,
        bottom: 3.0,
        left: 9.0,
    })
    .style(move |_theme, status| {
        if primary {
            filled_button_style(tint, status)
        } else {
            theme::ghost_button_style(status)
        }
    })
    .into()
}

/// Filled button style tinted by `tint` with dark text — the primary action
/// inside a tinted banner (Refreeze / Retry). Mirrors `primary_button_style`'s
/// hover/pressed darkening but keyed to the semantic banner colour.
fn filled_button_style(
    tint: Color,
    status: iced::widget::button::Status,
) -> iced::widget::button::Style {
    use iced::widget::button::Status;
    let bg = match status {
        Status::Hovered => Color { a: 0.85, ..tint },
        Status::Pressed => Color {
            r: tint.r * 0.85,
            g: tint.g * 0.85,
            b: tint.b * 0.85,
            a: 1.0,
        },
        _ => tint,
    };
    iced::widget::button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: Color::from_rgb8(0x14, 0x0e, 0x0a),
        border: iced::Border {
            color: tint,
            width: 0.0,
            radius: theme::RADIUS_MD.into(),
        },
        ..Default::default()
    }
}
