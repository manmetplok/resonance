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

use resonance_audio::types::ReferenceAnalysisStage;

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
        PanelState::Analyzing => analyzing_body(&r.reference),
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
        Space::new().height(14),
        container(exclusion_badge()).center_x(Length::Fill),
    ]
    .spacing(0)
    .into()
}

/// The "not in exports" reassurance pill. A reference is a monitoring-only
/// surface, never bounced or stem-exported; this badge says so wherever the
/// panel needs to reassure the user (Empty state today, per design doc #198).
fn exclusion_badge() -> Element<'static, Message> {
    container(
        row![
            text(fa::EYE.to_string())
                .font(theme::ICON_FONT)
                .size(9)
                .color(theme::GOOD),
            text("Not included in exports")
                .size(10)
                .font(theme::UI_FONT_MEDIUM)
                .color(theme::TEXT_2),
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center),
    )
    .padding([4, 10])
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
// Analyzing — the 4-stage offline-analysis checklist + determinate progress
// bar + Cancel, driven by `ReferenceAnalysisProgress` events. Populated /
// Error remain placeholder bodies whose detail UI lands in later todos.
// ---------------------------------------------------------------------------

/// The four offline-analysis stages, in the order the engine reports them,
/// paired with the user-facing label shown in the checklist.
const ANALYSIS_STAGES: [(ReferenceAnalysisStage, &str); 4] = [
    (ReferenceAnalysisStage::Decoding, "Decoding audio"),
    (ReferenceAnalysisStage::MeasuringLufs, "Measuring loudness"),
    (ReferenceAnalysisStage::BuildingPeaks, "Building waveform"),
    (ReferenceAnalysisStage::ComputingOffset, "Matching loudness"),
];

/// Position of `stage` within [`ANALYSIS_STAGES`] (0-based). Stages strictly
/// before the current one are complete; the current one is in progress; later
/// ones are pending.
fn stage_index(stage: ReferenceAnalysisStage) -> usize {
    ANALYSIS_STAGES
        .iter()
        .position(|(s, _)| *s == stage)
        .unwrap_or(0)
}

fn analyzing_body(state: &ReferenceState) -> Element<'_, Message> {
    // The first analysing entry drives the panel — the Analyzing state is
    // the first-load experience, and `classify` only routes here when one
    // exists. Fall back gracefully if it has just flipped to Loaded.
    let Some(entry) = state
        .entries
        .iter()
        .find(|e| matches!(e.status, ReferenceStatus::Analyzing(_)))
    else {
        return placeholder_card("Analyzing reference\u{2026}", theme::TEXT_2);
    };
    let ReferenceStatus::Analyzing(stage) = entry.status else {
        return placeholder_card("Analyzing reference\u{2026}", theme::TEXT_2);
    };
    let current = stage_index(stage);

    let title = if entry.name.is_empty() {
        "Analyzing reference\u{2026}".to_string()
    } else {
        format!("Analyzing {}\u{2026}", entry.name)
    };

    let mut checklist = column![].spacing(10);
    for (i, (_, label)) in ANALYSIS_STAGES.iter().enumerate() {
        checklist = checklist.push(stage_row(label, i, current));
    }

    let cancel = button(text("Cancel").size(12).font(theme::UI_FONT_MEDIUM))
        .on_press(Message::Reference(
            crate::reference::ReferenceMessage::Remove(entry.id),
        ))
        .padding([7, 14])
        .style(|_theme, status| theme::small_button_style(status));

    container(
        column![
            text(title)
                .size(13)
                .font(theme::UI_FONT_MEDIUM)
                .color(theme::TEXT_1),
            Space::new().height(14),
            progress_bar(current),
            Space::new().height(16),
            checklist,
            Space::new().height(18),
            row![Space::new().width(Length::Fill), cancel],
        ]
        .spacing(0),
    )
    .width(Length::Fill)
    .padding([16, 16])
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

/// One checklist row: a status glyph (done / in-progress / pending) and the
/// stage label, coloured to match its state.
fn stage_row(label: &str, index: usize, current: usize) -> Element<'static, Message> {
    let (glyph, glyph_color, text_color) = if index < current {
        (fa::CIRCLE, theme::GOOD, theme::TEXT_2)
    } else if index == current {
        (fa::BULLSEYE, theme::ACCENT, theme::TEXT_1)
    } else {
        (fa::CIRCLE_HOLLOW, theme::TEXT_4, theme::TEXT_3)
    };

    row![
        text(glyph.to_string())
            .font(theme::ICON_FONT)
            .size(11)
            .color(glyph_color),
        text(label.to_string()).size(12).color(text_color),
    ]
    .spacing(10)
    .align_y(alignment::Vertical::Center)
    .into()
}

/// Determinate progress track filled to the current stage. The fill grows a
/// quarter per stage (Decoding → ¼ … ComputingOffset → 4/4), giving the user
/// a sense of forward motion through the four-step analysis.
fn progress_bar(current: usize) -> Element<'static, Message> {
    let done = (current + 1) as u16;
    let remaining = ANALYSIS_STAGES.len() as u16 - done;

    let fill = container(Space::new())
        .width(Length::FillPortion(done))
        .height(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::ACCENT)),
            border: iced::Border {
                radius: theme::RADIUS_XS.into(),
                ..Default::default()
            },
            ..Default::default()
        });

    let track = row![fill, Space::new().width(Length::FillPortion(remaining))];

    container(track)
        .width(Length::Fill)
        .height(Length::Fixed(5.0))
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_3)),
            border: iced::Border {
                radius: theme::RADIUS_XS.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
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
