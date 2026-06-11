//! Chord-lane inspector body — matches the redesign's right rail:
//! a compact scale picker at the top, then "Chord generator" with the
//! style/length/beat/seventh-chords/start/end controls + a primary
//! lavender Generate action and an ↻ regenerate ghost button + seed
//! footer, then a "Section motif" block with source/complexity/preview.

use iced::widget::{column, row, text, Space};
use iced::{alignment, Element, Length};

use resonance_music_theory::{Degree, SchemaKind};

use super::display_pick::display_pick;
use crate::compose::messages::GeneratorKind;
use crate::compose::SectionDefinitionState;
use crate::message::*;
use crate::theme;

mod body;
mod motif;
mod preview_canvas;
mod scale;

pub(super) use body::chord_body;
pub(super) use scale::scale_block;

pub(super) const TABLE_IDS: &[&str] = &["pop", "modal", "jazz", "post-rock", "metal", "classical"];
pub(super) const TABLE_NAMES: &[&str] = &["Pop", "Modal", "Jazz", "Post-Rock", "Metal", "Classical"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TablePick {
    pub(super) id: String,
    pub(super) label: String,
}

impl std::fmt::Display for TablePick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

pub(super) fn table_picks() -> Vec<TablePick> {
    TABLE_IDS
        .iter()
        .zip(TABLE_NAMES.iter())
        .map(|(id, name)| TablePick {
            id: id.to_string(),
            label: name.to_string(),
        })
        .collect()
}

// GENERATOR mode dropdown (Style table / Schema) and SCHEMA dropdown —
// `Display` newtypes + process-wide cached option slices, so no Vec is
// allocated per repaint (see view-layer performance rules §11.1).
display_pick!(GeneratorKindPick, GeneratorKind, as_str, generator_kind_options);
display_pick!(SchemaPick, SchemaKind, name, schema_pick_options);

/// One entry in the ROTATION dropdown: the rotation amount plus a
/// preview of the rotated loop ("1 · V–vi–IV–I"). Loops longer than
/// four chords are elided ("0 · I–IV–vii°–iii…").
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RotationPick {
    pub(super) rotation: u8,
    label: String,
}

impl std::fmt::Display for RotationPick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// Rotation options for `schema` — one per position in its base loop.
/// All 13 schemas' option vectors are built once and cached; the view
/// hands `pick_list` a `&'static` slice (no per-frame allocation).
pub(super) fn rotation_picks(schema: SchemaKind) -> &'static [RotationPick] {
    static V: std::sync::OnceLock<Vec<Vec<RotationPick>>> = std::sync::OnceLock::new();
    let all = V.get_or_init(|| {
        SchemaKind::ALL
            .iter()
            .map(|k| {
                let base = k.base_degrees();
                (0..base.len())
                    .map(|r| RotationPick {
                        rotation: r as u8,
                        label: rotation_label(base, r),
                    })
                    .collect()
            })
            .collect()
    });
    let idx = SchemaKind::ALL
        .iter()
        .position(|k| *k == schema)
        .expect("SchemaKind::ALL is exhaustive");
    &all[idx]
}

/// "{rotation} · {first ≤4 degrees of the rotated loop}", elided with
/// `…` when the loop is longer than four chords.
fn rotation_label(base: &[Degree], rotation: usize) -> String {
    let n = base.len();
    let shown: Vec<String> = (0..n.min(4))
        .map(|i| base[(i + rotation) % n].to_string())
        .collect();
    let ellipsis = if n > 4 { "…" } else { "" };
    format!("{rotation} · {}{ellipsis}", shown.join("–"))
}

pub(super) fn current_table_id(def: &SectionDefinitionState) -> String {
    match &def.generator_spec {
        Some(resonance_music_theory::GeneratorSpec::MarkovProgression { table_id, .. }) => {
            table_id.clone()
        }
        // No spec yet, or a non-Markov spec (e.g. Schema): the table
        // picker shows the default it would switch to.
        _ => "pop".to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DegreePick(pub(super) Option<Degree>);

impl std::fmt::Display for DegreePick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            None => f.write_str("(any)"),
            Some(d) => write!(f, "{d}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MotifLenPick(pub(super) u8);

impl std::fmt::Display for MotifLenPick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0 == 0 {
            f.write_str("Auto")
        } else {
            write!(f, "{} notes", self.0)
        }
    }
}

// ---------------------------------------------------------------------------
// Section header — clickable collapse row (label left, shared caret
// right) with a bottom hairline. Clicking anywhere on it folds /
// unfolds the panel body under it.
// ---------------------------------------------------------------------------

pub(super) fn section_header<'a>(
    title: &'static str,
    key: crate::compose::RailPanelKey,
    collapsed: bool,
) -> Element<'a, Message> {
    let title_el: Element<'a, Message> = text(title)
        .size(13)
        .font(theme::UI_FONT_MEDIUM)
        .color(theme::TEXT_1)
        .into();
    let head =
        crate::view::compose::lane_inspector::rail_panel_header(title_el, None, key, collapsed);
    column![
        head,
        Space::new().height(4),
        Space::new().height(1).width(Length::Fill),
        crate::view::compose::lane_inspector::separator(),
    ]
    .spacing(0)
    .into()
}

/// Small uppercase field label, matching the design's letterspaced FIELD
/// captions.
pub(super) fn field_label<'a>(label: impl Into<String>) -> Element<'a, Message> {
    text(label.into())
        .size(10)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(theme::TEXT_3)
        .into()
}

/// Two-state toggle row — label on the left, pill toggle on the right.
pub(super) fn toggle_row<'a>(label: &'a str, on: bool, msg: Message) -> Element<'a, Message> {
    let track_color = if on { theme::ACCENT } else { theme::BG_3 };
    let knob_x = if on { 14.0 } else { 1.0 };

    let knob = iced::widget::container(Space::new().width(0))
        .width(12)
        .height(12)
        .style(|_theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(iced::Color::WHITE)),
            border: iced::Border {
                radius: 6.0.into(),
                ..Default::default()
            },
            ..Default::default()
        });

    let track = iced::widget::container(
        row![Space::new().width(knob_x), knob]
            .align_y(alignment::Vertical::Center),
    )
    .width(28)
    .height(16)
    .center_y(Length::Fill)
    .style(move |_theme| iced::widget::container::Style {
        background: Some(iced::Background::Color(track_color)),
        border: iced::Border {
            color: theme::LINE,
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    });

    let mouse = iced::widget::mouse_area(
        row![
            text(label).size(12).color(theme::TEXT_1),
            Space::new().width(Length::Fill),
            track,
        ]
        .align_y(alignment::Vertical::Center),
    )
    .on_press(msg);

    iced::widget::container(mouse)
        .width(Length::Fill)
        .padding([2, 0])
        .into()
}

pub(super) fn degree_picks_from(table_degrees: &[Degree]) -> Vec<DegreePick> {
    let mut picks = vec![DegreePick(None)];
    for d in table_degrees {
        picks.push(DegreePick(Some(*d)));
    }
    picks
}
