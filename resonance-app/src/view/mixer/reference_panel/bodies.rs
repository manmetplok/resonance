//! The four body renderers routed by the panel container:
//! `empty_body`, `analyzing_body`, `populated_body`, `error_body`.
//! Also contains the reference list and inline error-card helpers used
//! by the populated body.

use iced::widget::{button, column, container, row, text, Space};
use iced::{alignment, Element, Length};

use resonance_audio::types::ReferenceAnalysisStage;

use crate::message::*;
use crate::reference::{ReferenceEntry, ReferenceMessage, ReferenceState, ReferenceStatus};
use crate::theme::{self, fa};
use crate::update::reference::REFERENCE_AUDIO_EXTENSIONS;

use super::{ab_controls as ab, loudness_canvas, widgets};

// ---------------------------------------------------------------------------
// Empty — dashed drop zone, format chips, "Add reference…", exclusion badge.
// ---------------------------------------------------------------------------

pub(super) fn empty_body() -> Element<'static, Message> {
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
        chips = chips.push(widgets::format_chip(ext));
    }
    let chips = container(chips).center_x(Length::Fill);

    // Primary CTA for the Empty state — a filled lavender action button,
    // not a toggle. (The chrome REF button is the genuine toggle.)
    let add_btn = button(
        text("Add reference\u{2026}")
            .size(12)
            .font(theme::UI_FONT_MEDIUM),
    )
    .on_press(Message::Reference(ReferenceMessage::PickFile))
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
        container(widgets::exclusion_badge()).center_x(Length::Fill),
    ]
    .spacing(0)
    .into()
}

// ---------------------------------------------------------------------------
// Analyzing — the 4-stage offline-analysis checklist + determinate progress
// bar + Cancel, driven by `ReferenceAnalysisProgress` events.
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

pub(super) fn analyzing_body(state: &ReferenceState) -> Element<'_, Message> {
    // The first analysing entry drives the panel — the Analyzing state is
    // the first-load experience, and `classify` only routes here when one
    // exists. Fall back gracefully if it has just flipped to Loaded.
    let Some(entry) = state
        .entries
        .iter()
        .find(|e| matches!(e.status, ReferenceStatus::Analyzing(_)))
    else {
        return widgets::placeholder_card("Analyzing reference\u{2026}", theme::TEXT_2);
    };
    let ReferenceStatus::Analyzing(stage) = entry.status else {
        return widgets::placeholder_card("Analyzing reference\u{2026}", theme::TEXT_2);
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
        .on_press(Message::Reference(ReferenceMessage::Remove(entry.id)))
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

// ---------------------------------------------------------------------------
// Populated — selectable reference list + A/B controls + loudness readout.
// ---------------------------------------------------------------------------

pub(super) fn populated_body(state: &ReferenceState) -> Element<'_, Message> {
    let mut col = column![reference_list(state)].spacing(14);

    // The A/B detail controls operate on the *active* reference, and only
    // once it has finished analysing (a waveform + loudness to show).
    if let Some(entry) = state
        .active_id
        .and_then(|id| state.entries.iter().find(|e| e.id == id))
        .filter(|e| matches!(e.status, ReferenceStatus::Loaded))
    {
        col = col.push(ab::ab_controls(state, entry));
    } else if state
        .entries
        .iter()
        .any(|e| matches!(e.status, ReferenceStatus::Loaded))
    {
        col = col.push(
            text("Select a reference above to compare")
                .size(11)
                .color(theme::TEXT_3),
        );
    }

    // The comparative loudness readout lives at the bottom of every
    // populated state (design doc #198), driven by the latest A/B meter
    // snapshot. The reference column reads "—" until a reference is active
    // and metered.
    if state
        .entries
        .iter()
        .any(|e| matches!(e.status, ReferenceStatus::Loaded))
    {
        col = col.push(loudness_canvas::loudness_readout(state));
    }

    col.into()
}

/// The loaded-reference list. Each loaded entry is a selectable row (name +
/// integrated loudness + a remove ×); the active one is lavender-lit.
/// Missing / errored entries keep their inline BAD card so they stay
/// actionable without hiding the references that did load.
fn reference_list(state: &ReferenceState) -> Element<'_, Message> {
    let mut list = column![].spacing(6);
    for entry in &state.entries {
        let row = match &entry.status {
            ReferenceStatus::Missing => error_card(entry, None),
            ReferenceStatus::Error(reason) => error_card(entry, Some(reason)),
            _ => reference_row(entry, state.active_id == Some(entry.id)),
        };
        list = list.push(row);
    }
    list.into()
}

/// One selectable row for a loaded reference: the clickable name + loudness
/// block (selects it active) sits beside a remove (×) button.
fn reference_row(entry: &ReferenceEntry, active: bool) -> Element<'_, Message> {
    let lufs = if entry.integrated_lufs.is_finite() {
        format!("{:.1} LUFS", entry.integrated_lufs)
    } else {
        "— LUFS".to_string()
    };

    let info = button(
        column![
            text(entry.name.clone())
                .size(12)
                .font(theme::UI_FONT_MEDIUM)
                .color(if active { theme::ACCENT_SOFT } else { theme::TEXT_1 }),
            Space::new().height(1),
            text(lufs).size(10).color(theme::TEXT_3),
        ]
        .spacing(0),
    )
    .width(Length::Fill)
    .padding([7, 10])
    .on_press(Message::Reference(ReferenceMessage::SetActive(entry.id)))
    .style(move |_theme, status| select_row_style(active, status));

    let remove = button(text("\u{00d7}").size(14).color(theme::TEXT_3))
        .on_press(Message::Reference(ReferenceMessage::Remove(entry.id)))
        .padding([1, 8])
        .style(|_theme, status| theme::small_button_style(status));

    row![info, remove]
        .spacing(4)
        .align_y(alignment::Vertical::Center)
        .into()
}

/// Selected-row chrome: a lavender wash + border when active, a hairline
/// card otherwise (hover-lit).
fn select_row_style(active: bool, status: button::Status) -> button::Style {
    let (bg, border) = if active {
        (theme::ACCENT_DIM, theme::ACCENT_LINE)
    } else {
        let bg = match status {
            button::Status::Hovered | button::Status::Pressed => theme::BG_3,
            _ => theme::BG_2,
        };
        (bg, theme::LINE_2)
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: theme::TEXT_1,
        border: iced::Border {
            color: border,
            width: 1.0,
            radius: theme::RADIUS_MD.into(),
        },
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Error bodies.
// ---------------------------------------------------------------------------

/// Full-panel error body for a load that failed before any entry existed
/// (so the notice lives in `last_error`, not on an entry). Reached only
/// while the slot is otherwise empty; a missing/errored entry alongside
/// loaded references renders inline via [`error_card`] instead.
pub(super) fn error_body(state: &ReferenceState) -> Element<'_, Message> {
    let reason = state
        .last_error
        .clone()
        .unwrap_or_else(|| "Reference failed to load".to_string());

    widgets::bad_card(
        column![
            widgets::error_heading("Couldn\u{2019}t load reference"),
            Space::new().height(6),
            text(reason).size(12).color(theme::TEXT_2),
            Space::new().height(14),
            // No entry to drop, so Dismiss just clears the notice.
            widgets::error_actions(Message::Reference(ReferenceMessage::DismissError)),
        ]
        .spacing(0),
    )
}

/// A BAD-tinted card for one missing or errored reference entry. `reason`
/// is `Some` for an [`ReferenceStatus::Error`] (the analysis failure text)
/// and `None` for an [`ReferenceStatus::Missing`] entry (file gone since
/// the project was saved). Dismiss drops just this entry; Choose another
/// re-opens the picker. Other entries are untouched.
fn error_card<'a>(entry: &'a ReferenceEntry, reason: Option<&'a str>) -> Element<'a, Message> {
    let name = if entry.name.is_empty() {
        std::path::Path::new(&entry.path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Reference")
            .to_string()
    } else {
        entry.name.clone()
    };

    let detail = reason
        .map(str::to_string)
        .unwrap_or_else(|| "File not found".to_string());

    let mut body = column![
        widgets::error_heading(&name),
        Space::new().height(4),
        text(detail).size(11).color(theme::TEXT_2),
    ]
    .spacing(0);

    // For a missing file, show the path so the user can tell which one.
    if reason.is_none() && !entry.path.is_empty() {
        body = body.push(Space::new().height(2));
        body = body.push(text(entry.path.clone()).size(10).color(theme::TEXT_3));
    }

    body = body.push(Space::new().height(14));
    body = body.push(widgets::error_actions(Message::Reference(
        ReferenceMessage::Remove(entry.id),
    )));

    widgets::bad_card(body)
}
