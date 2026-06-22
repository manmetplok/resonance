//! Mixer **Reference & A/B** right-rail (design doc #184/#198). A 360px
//! panel that auditions external mastered tracks against the mix. This
//! module lands the panel *container* and its state routing — the shell
//! header, the fully-built **Empty** drop-zone state, and stubbed bodies
//! for the **Analyzing / Populated / Error** states whose detail UI lands
//! in the follow-up todos (T9 / T10 / T12).
//!
//! The reference plays through the monitor path only and is excluded from
//! every bounce / stem export; the panel is purely a monitoring surface.

use iced::widget::{button, column, container, row, text, Space};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::reference::{ReferenceState, ReferenceStatus};
use crate::theme::{self, fa};
use crate::update::reference::REFERENCE_AUDIO_EXTENSIONS;

/// Which body the panel container routes to, derived from
/// [`ReferenceState`]. The per-entry Missing/Error and the populated A/B
/// controls are filled in by later todos; this scaffold only needs the
/// top-level routing to be correct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanelState {
    /// No references loaded and no pending load.
    Empty,
    /// At least one reference is mid-analysis.
    Analyzing,
    /// One or more references decoded and ready to audition.
    Populated,
    /// A load failed (no entry was created) — surfaced from `last_error`.
    Error,
}

fn classify(state: &ReferenceState) -> PanelState {
    // A load-failure notice wins: it carries no entry, so without this
    // branch a failed load on an otherwise-empty slot would fall through
    // to `Empty` and silently drop the error.
    if state.last_error.is_some() {
        return PanelState::Error;
    }
    if state
        .entries
        .iter()
        .any(|e| matches!(e.status, ReferenceStatus::Analyzing(_)))
    {
        return PanelState::Analyzing;
    }
    if state.entries.is_empty() && state.pending_loads.is_empty() {
        PanelState::Empty
    } else {
        PanelState::Populated
    }
}

pub(super) fn view(r: &crate::Resonance) -> Element<'_, Message> {
    let body: Element<'_, Message> = match classify(&r.reference) {
        PanelState::Empty => empty_body(),
        PanelState::Analyzing => analyzing_body(),
        PanelState::Populated => populated_body(&r.reference),
        PanelState::Error => error_body(&r.reference),
    };

    let content = column![header(), Space::new().height(18), body].spacing(0);

    container(content)
        .width(Length::Fixed(theme::REFERENCE_PANEL_WIDTH))
        .height(Length::Fill)
        .padding(theme::RAIL_PADDING)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            ..Default::default()
        })
        .into()
}

/// Panel title row: label + a close (×) button that re-toggles the rail.
fn header() -> Element<'static, Message> {
    let close = button(text("\u{00d7}").size(15).color(theme::TEXT_3))
        .on_press(Message::Ui(UiMessage::ToggleReferencePanel))
        .padding([1, 7])
        .style(|_theme, status| theme::small_button_style(status));

    row![
        column![
            text("REFERENCE & A/B")
                .size(10)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(theme::TEXT_3),
            Space::new().height(2),
            text("Monitor only — not in exports")
                .size(11)
                .color(theme::TEXT_2),
        ]
        .spacing(0),
        Space::new().width(Length::Fill),
        close,
    ]
    .align_y(alignment::Vertical::Center)
    .into()
}

// ---------------------------------------------------------------------------
// Empty — dashed drop zone, format chips, "Add reference…", exclusion badge.
// ---------------------------------------------------------------------------

fn empty_body() -> Element<'static, Message> {
    let drop_zone = container(
        column![
            text(fa::MUSIC.to_string())
                .font(theme::ICON_FONT)
                .size(22)
                .color(theme::TEXT_4),
            Space::new().height(12),
            text("Drop an audio file to compare")
                .size(13)
                .color(theme::TEXT_2),
            Space::new().height(4),
            text("or pick one below")
                .size(11)
                .color(theme::TEXT_3),
        ]
        .spacing(0)
        .align_x(alignment::Horizontal::Center),
    )
    .width(Length::Fill)
    .padding([34, 16])
    .center_x(Length::Fill)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: theme::RADIUS_MD.into(),
        },
        ..Default::default()
    });

    // Format chips, one per accepted container extension.
    let mut chips = row![].spacing(6);
    for ext in REFERENCE_AUDIO_EXTENSIONS {
        chips = chips.push(format_chip(ext));
    }
    let chips = container(chips).center_x(Length::Fill);

    // Primary CTA for the Empty state — a filled lavender action button,
    // not a toggle. (The chrome REF button is the genuine toggle.)
    let add_btn = button(
        text("Add reference\u{2026}")
            .size(12)
            .font(theme::UI_FONT_MEDIUM),
    )
    .on_press(Message::Reference(crate::reference::ReferenceMessage::PickFile))
    .width(Length::Fill)
    .padding([9, 12])
    .style(|_theme, status| theme::primary_button_style(status));

    column![
        drop_zone,
        Space::new().height(14),
        chips,
        Space::new().height(16),
        add_btn,
    ]
    .spacing(0)
    .into()
}

fn format_chip(label: &str) -> Element<'static, Message> {
    container(
        text(label.to_uppercase())
            .size(9)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
    )
    .padding([3, 8])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: theme::RADIUS_XS.into(),
        },
        ..Default::default()
    })
    .into()
}

// ---------------------------------------------------------------------------
// Analyzing / Populated / Error — placeholder bodies. The full stage
// checklist, A/B controls + waveform, and per-entry error cards land in
// the follow-up todos; these keep the routing observable in the meantime.
// ---------------------------------------------------------------------------

fn analyzing_body() -> Element<'static, Message> {
    placeholder_card("Analyzing reference\u{2026}", theme::TEXT_2)
}

fn populated_body(state: &ReferenceState) -> Element<'_, Message> {
    let mut col = column![].spacing(8);
    for entry in &state.entries {
        col = col.push(placeholder_card(&entry.name, theme::TEXT_1));
    }
    col.into()
}

fn error_body(state: &ReferenceState) -> Element<'_, Message> {
    let reason = state
        .last_error
        .clone()
        .unwrap_or_else(|| "Reference failed to load".to_string());

    let dismiss = button(text("Dismiss").size(12))
        .on_press(Message::Reference(
            crate::reference::ReferenceMessage::DismissError,
        ))
        .padding([7, 14])
        .style(|_theme, status| theme::small_button_style(status));

    container(
        column![
            text(reason).size(12).color(theme::BAD),
            Space::new().height(12),
            dismiss,
        ]
        .spacing(0),
    )
    .width(Length::Fill)
    .padding([16, 16])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::BAD,
            width: 1.0,
            radius: theme::RADIUS_MD.into(),
        },
        ..Default::default()
    })
    .into()
}

fn placeholder_card(label: &str, color: iced::Color) -> Element<'static, Message> {
    container(text(label.to_string()).size(12).color(color))
        .width(Length::Fill)
        .padding([12, 14])
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        })
        .into()
}
