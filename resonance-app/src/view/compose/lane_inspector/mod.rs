//! Unified right-hand inspector panel for the Compose view.
//!
//! One section, many lanes, each lane has an optional generator, the chord
//! lane is shared harmonic context. Selecting a lane updates this panel.

use iced::widget::{column, container, pick_list, row, scrollable, text, tooltip, Space};
use iced::{alignment, Element, Length};

use resonance_music_theory::TableRegistry;

use crate::compose::{ComposeMessage, DrumrollViewState, SectionDefinitionState, SelectedLane};
use crate::message::*;
use crate::state::{InstrumentType, TrackState};
use crate::theme;

/// Resolve the active selection into a display-ready (label, name) pair so
/// the EDITING context header can render `EDITING SECTION · Intro` or
/// `EDITING TRACK · Drums` regardless of which lane is focused.
fn editing_context<'a>(
    selected: &'a SelectedLane,
    definition: &'a SectionDefinitionState,
    tracks: &'a [TrackState],
) -> (&'static str, String, bool) {
    match selected {
        SelectedLane::Chords => ("SECTION", definition.name.clone(), true),
        SelectedLane::Instrument(id) | SelectedLane::Drums(id) => {
            let name = tracks
                .iter()
                .find(|t| t.id == *id)
                .map(|t| t.name.clone())
                .unwrap_or_else(|| "Track".to_string());
            ("TRACK", name, false)
        }
    }
}

/// EDITING context header card — the prominent banner that tells the user
/// whether they're editing the section's harmonic skeleton (chords / motif)
/// or a single track's generator. Lavender for section, warm amber for
/// track. Matches the "EDITING SECTION · {name}" treatment in the bundled
/// design and reinforces the GLOBAL / PER-TRACK scope chip on the right.
fn editing_header<'a>(
    selected: &'a SelectedLane,
    definition: &'a SectionDefinitionState,
    tracks: &'a [TrackState],
) -> Element<'a, Message> {
    let (scope_label, name, is_section) = editing_context(selected, definition, tracks);
    let accent = if is_section {
        theme::ACCENT_SOFT
    } else {
        theme::WARM
    };

    let editing_label = text(format!("EDITING {}", scope_label))
        .size(9)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(accent);

    let scope_chip_text = text(if is_section { "GLOBAL" } else { "PER-TRACK" })
        .size(9)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(accent);
    let scope_chip = if is_section {
        container(scope_chip_text)
            .padding([2, 8])
            .style(theme::editing_pill_style)
    } else {
        container(scope_chip_text)
            .padding([2, 8])
            .style(theme::editing_pill_warm_style)
    };

    let top_row = row![
        editing_label,
        Space::with_width(Length::Fill),
        scope_chip,
    ]
    .align_y(alignment::Vertical::Center);

    let name_text = text(name)
        .size(22)
        .font(theme::SERIF_ITALIC_FONT)
        .color(theme::TEXT_1)
        .wrapping(iced::widget::text::Wrapping::None);

    let body_row = row![name_text]
        .align_y(alignment::Vertical::Center)
        .spacing(8);

    let content = column![top_row, Space::with_height(4), body_row]
        .spacing(0)
        .width(Length::Fill);

    let card = container(content).padding([10, 12]).width(Length::Fill);
    if is_section {
        card.style(theme::editing_header_card_style).into()
    } else {
        card.style(theme::editing_header_card_warm_style).into()
    }
}

mod chord;
mod drums;
mod instrument;
mod vocal;

/// Compose right-rail width. Aliased to the design-system constant so
/// every right-hand panel widens together if the token shifts.
pub const PANEL_WIDTH: f32 = theme::COMPOSE_RAIL_WIDTH as f32;

// ===========================================================================
// Top-level inspector
// ===========================================================================

pub fn view<'a>(
    definition: &'a SectionDefinitionState,
    selected_lane: &'a SelectedLane,
    tracks: &'a [TrackState],
    drumroll_state: &'a DrumrollViewState,
    clip_id_for_drum: Option<u64>,
    table_registry: &'a TableRegistry,
) -> Element<'a, Message> {
    // EDITING context header — large, unmistakable. Tells the user whether
    // they're editing the section (lavender) or a track (warm amber) and
    // shows the active lane's name in italic serif.
    let editing_card = editing_header(selected_lane, definition, tracks);

    // Scale block — always at top, section-global.
    let scale = chord::scale_block(definition);

    // Lane switcher
    let lane_switcher = lane_switcher_row(selected_lane, tracks);

    // Body — varies by selected lane
    let body: Element<'a, Message> = match selected_lane {
        SelectedLane::Chords => chord::chord_body(definition, table_registry),
        SelectedLane::Instrument(track_id) => {
            let track = tracks.iter().find(|t| t.id == *track_id);
            match track {
                Some(t) => instrument::instrument_body(definition, t),
                None => text("Track not found")
                    .size(12)
                    .color(theme::TEXT_DIM)
                    .into(),
            }
        }
        SelectedLane::Drums(track_id) => {
            let track = tracks.iter().find(|t| t.id == *track_id);
            match track {
                Some(t) => drums::drum_body(definition, t, drumroll_state, clip_id_for_drum),
                None => text("Track not found")
                    .size(12)
                    .color(theme::TEXT_DIM)
                    .into(),
            }
        }
    };

    let inner = column![
        editing_card,
        Space::with_height(14),
        scale,
        Space::with_height(20),
        lane_switcher,
        Space::with_height(20),
        body,
    ]
    .spacing(0)
    .padding(18);

    // The vocal generator panel runs ~5 stacked group cards, easily
    // taller than the viewport. Wrapping the rail in a vertical
    // scrollable keeps every control reachable on smaller windows.
    let scrollable_body = scrollable(inner)
        .direction(scrollable::Direction::Vertical(
            scrollable::Scrollbar::default(),
        ))
        .width(Length::Fill)
        .height(Length::Fill);

    container(scrollable_body)
        .width(Length::Fixed(PANEL_WIDTH))
        .height(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 0.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}

// ===========================================================================
// Lane switcher
// ===========================================================================

/// Lane names for the dropdown.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LanePick {
    lane: SelectedLane,
    label: String,
}

impl std::fmt::Display for LanePick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

fn lane_switcher_row<'a>(
    selected: &'a SelectedLane,
    tracks: &'a [TrackState],
) -> Element<'a, Message> {
    let mut options = vec![LanePick {
        lane: SelectedLane::Chords,
        label: "Chords".to_string(),
    }];

    for t in tracks.iter().filter(|t| t.sub_track.is_none()) {
        let lane = if t.instrument_type == InstrumentType::Drum {
            SelectedLane::Drums(t.id)
        } else {
            SelectedLane::Instrument(t.id)
        };
        options.push(LanePick {
            lane,
            label: t.name.clone(),
        });
    }

    let current = options.iter().find(|o| o.lane == *selected).cloned();

    let picker = pick_list(options, current, |pick| {
        Message::Compose(ComposeMessage::SelectLane(pick.lane))
    })
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);

    column![
        text("LANE")
            .size(10)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
        Space::with_height(4),
        picker,
    ]
    .spacing(0)
    .into()
}

// ===========================================================================
// Shared helpers
// ===========================================================================

/// Small info-icon (Font Awesome circle-info) that shows `info` on hover.
/// Use via [`label_with_info`] to pair a control label with its explanation.
pub(super) fn info_icon<'a>(info: &'static str) -> Element<'a, Message> {
    let icon = container(theme::icon(theme::fa::CIRCLE_INFO).size(10).color(theme::TEXT_DIM))
        .padding([0, 2]);
    let tip = container(text(info).size(11).color(theme::TEXT))
        .max_width(220.0)
        .padding(8)
        .style(|_theme: &iced::Theme| container::Style {
            text_color: Some(theme::TEXT),
            background: Some(iced::Background::Color(theme::PANEL_DARK)),
            border: iced::Border {
                color: theme::SEPARATOR,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..Default::default()
        });
    tooltip(icon, tip, tooltip::Position::Top).gap(4).into()
}

/// Standard small dim label paired with a hoverable info icon. Drop-in
/// replacement for `text(label).size(11).color(theme::TEXT_DIM)` when
/// the option deserves a one-sentence explanation.
pub(super) fn label_with_info<'a>(label: impl Into<String>, info: &'static str) -> Element<'a, Message> {
    let label_text = text(label.into()).size(11).color(theme::TEXT_DIM);
    row![label_text, Space::with_width(4), info_icon(info)]
        .align_y(alignment::Vertical::Center)
        .into()
}

pub(super) fn separator<'a>() -> Element<'a, Message> {
    container(Space::with_height(1))
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::SEPARATOR)),
            ..Default::default()
        })
        .into()
}
