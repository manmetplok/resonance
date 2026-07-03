//! Right-rail drum-*arrangement* surfaces (doc #170, epic #38):
//!
//! 1. [`pattern_bank_card`] — the project-wide pattern bank as draggable
//!    sources. Each pattern is a grab-handle row that adds a new entry to
//!    the focused section's arrangement when dragged onto the strip/ribbon
//!    (or clicked, the always-available fallback since Iced has no native
//!    cross-widget drag payload). Meta shows the pattern's bar length +
//!    group count; a hint calls out that editing a pattern updates every
//!    section that chains it.
//! 2. [`entry_inspector_card`] — the inspector for the currently selected
//!    arrangement entry: pattern picker dropdown (swap `pattern_id`),
//!    Length-mode segmented toggle (`Repeat ×N` / `Fixed bars`), a
//!    repeat/length stepper with a live `2-bar pattern → 6 bars` readout,
//!    a Fill-on-last-repeat toggle + fill-pattern dropdown, and
//!    Duplicate / Remove.
//!
//! Every control emits an [`ArrangementMessage`] (epic #38 / todo #484);
//! the update handler mutates the arrangement and re-materializes the drum
//! clips, so edits reflect in the arrangement strip + tiling ribbon. All
//! colours / radii / fonts come from `theme.rs` tokens.

use iced::widget::{button, column, container, pick_list, row, text, Space};
use iced::{alignment, Color, Element, Length};

use crate::compose::drumroll::DrumPattern;
use crate::compose::messages::ArrangementMessage;
use crate::compose::{ComposeMessage, EntryLength, PatternEntry, SectionDefinitionState};
use crate::message::Message;
use crate::theme;

use super::common::{rail_card, rail_dot, u8_color};

/// Bounds for the repeat / fixed-bars stepper so a fat-fingered click can't
/// drive an entry to zero (which would silently drop it) or to an absurd
/// span.
const MIN_LEN: u32 = 1;
const MAX_LEN: u32 = 64;

/// Effective intrinsic bar length of a pattern id, guarded to `>= 1` so the
/// `RepeatN` readout never divides by zero. Mirrors [`DrumPattern::bar_span`].
fn bar_span_of(patterns: &[DrumPattern], pattern_id: u64) -> u32 {
    patterns
        .iter()
        .find(|p| p.id == pattern_id)
        .map(|p| p.bar_span())
        .unwrap_or(1)
}

fn pattern_color(patterns: &[DrumPattern], pattern_id: u64) -> Color {
    patterns
        .iter()
        .find(|p| p.id == pattern_id)
        .map(|p| u8_color(p.color))
        .unwrap_or(theme::TEXT_4)
}

fn pattern_name(patterns: &[DrumPattern], pattern_id: u64) -> String {
    patterns
        .iter()
        .find(|p| p.id == pattern_id)
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "—".to_string())
}

fn arr(msg: ArrangementMessage) -> Message {
    Message::Compose(ComposeMessage::Arrangement(msg))
}

// ===========================================================================
// Pattern bank card
// ===========================================================================

/// The project-wide pattern bank rendered as draggable "add to arrangement"
/// sources. `definition` is the focused section the drag/click targets.
pub(super) fn pattern_bank_card<'a>(
    definition: &'a SectionDefinitionState,
    patterns: &'a [DrumPattern],
) -> Element<'a, Message> {
    let title = row![
        rail_dot(theme::ACCENT_SOFT),
        text("Pattern bank").size(12).color(theme::TEXT_1),
        Space::new().width(Length::Fill),
        text(format!("{} patterns", patterns.len()))
            .size(9)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_4),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);

    let hint = text(
        "Drag a pattern into the arrangement to add it. Editing a pattern \
         updates every section that chains it.",
    )
    .size(10)
    .color(theme::TEXT_3);

    let mut items: Vec<Element<'a, Message>> = Vec::new();
    if patterns.is_empty() {
        items.push(
            text("No patterns in the bank yet.")
                .size(11)
                .color(theme::TEXT_DIM)
                .into(),
        );
    } else {
        for p in patterns {
            items.push(bank_row(definition.id, p));
        }
    }
    let list = column(items).spacing(6);

    rail_card(
        column![
            title,
            Space::new().height(8),
            hint,
            Space::new().height(10),
            list,
        ]
        .spacing(0)
        .into(),
    )
}

/// One draggable bank source: grab handle + colour dot + name, with a
/// `length · groups` meta line. Clicking (the drag fallback) appends the
/// pattern as a fresh entry.
fn bank_row<'a>(definition_id: u64, pattern: &'a DrumPattern) -> Element<'a, Message> {
    let color = u8_color(pattern.color);
    let handle = theme::icon(theme::fa::BARS).size(10).color(theme::TEXT_4);
    let name = text(pattern.name.clone()).size(12).color(theme::TEXT_1);
    let meta = text(format!(
        "{}-bar · {} groups",
        pattern.bar_span(),
        pattern.group_count()
    ))
    .size(9)
    .font(theme::MONO_FONT)
    .color(theme::TEXT_4);

    let left = row![
        handle,
        rail_dot(color),
        column![name, meta].spacing(1),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);

    let add = text("+ Add").size(10).color(theme::TEXT_3);

    let body = row![left, Space::new().width(Length::Fill), add]
        .spacing(8)
        .align_y(alignment::Vertical::Center)
        .width(Length::Fill);

    let accent = color;
    button(body)
        .padding([7, 10])
        .width(Length::Fill)
        .on_press(arr(ArrangementMessage::AddEntry {
            definition_id,
            pattern_id: pattern.id,
        }))
        .style(move |_theme, status| {
            let (bg, border) = match status {
                button::Status::Hovered => (theme::BG_3, accent),
                _ => (theme::BG_1, theme::LINE_2),
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
        })
        .into()
}

// ===========================================================================
// Entry inspector card
// ===========================================================================

/// Dropdown choice wrapping a pattern id + label. Equality is by id so the
/// pick_list highlights the selected pattern regardless of label churn.
#[derive(Clone)]
struct PatternChoice {
    id: u64,
    label: String,
}

impl PartialEq for PatternChoice {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for PatternChoice {}

impl std::fmt::Display for PatternChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

fn pattern_choices(patterns: &[DrumPattern]) -> Vec<PatternChoice> {
    patterns
        .iter()
        .map(|p| PatternChoice {
            id: p.id,
            label: p.name.clone(),
        })
        .collect()
}

/// Resolve which entry index the inspector shows: the explicit selection,
/// clamped into range, or the first entry as a fallback so the card is
/// never blank while the section has an arrangement.
pub(super) fn resolved_entry_index(
    definition: &SectionDefinitionState,
    selected: Option<usize>,
) -> Option<usize> {
    let len = definition.arrangement.len();
    if len == 0 {
        return None;
    }
    Some(selected.map(|i| i.min(len - 1)).unwrap_or(0))
}

/// Entry inspector for the selected arrangement entry (or an empty-state
/// prompt when the arrangement has no entries).
pub(super) fn entry_inspector_card<'a>(
    definition: &'a SectionDefinitionState,
    patterns: &'a [DrumPattern],
    selected: Option<usize>,
) -> Element<'a, Message> {
    let Some(index) = resolved_entry_index(definition, selected) else {
        return empty_entry_card();
    };
    let entry = &definition.arrangement[index];
    let def_id = definition.id;

    let title = row![
        rail_dot(pattern_color(patterns, entry.pattern_id)),
        text(format!("Entry {}", index + 1))
            .size(12)
            .color(theme::TEXT_1),
        Space::new().width(Length::Fill),
        text(pattern_name(patterns, entry.pattern_id))
            .size(10)
            .color(theme::TEXT_3),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);

    let pattern_field = field_block(
        "PATTERN",
        pattern_picker(def_id, index, entry, patterns),
    );
    let length_field = field_block(
        "LENGTH MODE",
        length_mode_toggle(def_id, index, entry, patterns),
    );
    let stepper_field = length_stepper(def_id, index, entry, patterns);
    let fill_field = field_block("FILL", fill_controls(def_id, index, entry, patterns));
    let actions = entry_actions(def_id, index);

    rail_card(
        column![
            title,
            Space::new().height(12),
            pattern_field,
            Space::new().height(12),
            length_field,
            Space::new().height(12),
            stepper_field,
            Space::new().height(12),
            fill_field,
            Space::new().height(14),
            actions,
        ]
        .spacing(0)
        .into(),
    )
}

fn empty_entry_card<'a>() -> Element<'a, Message> {
    rail_card(
        column![
            row![
                rail_dot(theme::TEXT_4),
                text("Entry").size(12).color(theme::TEXT_1),
            ]
            .spacing(8)
            .align_y(alignment::Vertical::Center),
            Space::new().height(8),
            text("Add a pattern from the bank to start the arrangement, then select an entry to edit it.")
                .size(10)
                .color(theme::TEXT_3),
        ]
        .spacing(0)
        .into(),
    )
}

/// `LABEL` kicker over a control.
fn field_block<'a>(label: &str, control: Element<'a, Message>) -> Element<'a, Message> {
    column![
        text(label.to_string()).size(10).color(theme::TEXT_3),
        Space::new().height(4),
        control,
    ]
    .spacing(0)
    .into()
}

fn pattern_picker<'a>(
    def_id: u64,
    index: usize,
    entry: &PatternEntry,
    patterns: &[DrumPattern],
) -> Element<'a, Message> {
    let options = pattern_choices(patterns);
    let selected = options.iter().find(|c| c.id == entry.pattern_id).cloned();
    pick_list(options, selected, move |choice: PatternChoice| {
        arr(ArrangementMessage::SetEntryPattern {
            definition_id: def_id,
            index,
            pattern_id: choice.id,
        })
    })
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill)
    .into()
}

/// Segmented `Repeat ×N | Fixed bars` toggle. Switching mode converts the
/// current value so the entry keeps roughly the same bar span.
fn length_mode_toggle<'a>(
    def_id: u64,
    index: usize,
    entry: &PatternEntry,
    patterns: &[DrumPattern],
) -> Element<'a, Message> {
    let span = bar_span_of(patterns, entry.pattern_id);
    let is_repeat = matches!(entry.length, EntryLength::RepeatN(_));

    // Repeat → keep n; Fixed → derive a repeat count that best matches the
    // fixed span. Guard both to the stepper bounds.
    let to_repeat = match entry.length {
        EntryLength::RepeatN(n) => n,
        EntryLength::Bars(b) => (b / span.max(1)).clamp(MIN_LEN, MAX_LEN),
    };
    // Fixed → keep bars; Repeat → expand to concrete bars.
    let to_fixed = match entry.length {
        EntryLength::Bars(b) => b,
        EntryLength::RepeatN(n) => (n.saturating_mul(span)).clamp(MIN_LEN, MAX_LEN),
    };

    let repeat_btn = segment_button(
        "Repeat ×N",
        is_repeat,
        arr(ArrangementMessage::SetEntryLength {
            definition_id: def_id,
            index,
            length: EntryLength::RepeatN(to_repeat),
        }),
    );
    let fixed_btn = segment_button(
        "Fixed bars",
        !is_repeat,
        arr(ArrangementMessage::SetEntryLength {
            definition_id: def_id,
            index,
            length: EntryLength::Bars(to_fixed),
        }),
    );

    row![repeat_btn, fixed_btn]
        .spacing(4)
        .width(Length::Fill)
        .into()
}

fn segment_button<'a>(label: &str, active: bool, msg: Message) -> Element<'a, Message> {
    button(
        text(label.to_string())
            .size(11)
            .color(if active { theme::TEXT_1 } else { theme::TEXT_3 }),
    )
    .padding([5, 8])
    .width(Length::Fill)
    .on_press(msg)
    .style(move |_theme, status| {
        let bg = if active {
            theme::ACCENT_DIM
        } else if matches!(status, button::Status::Hovered) {
            theme::BG_3
        } else {
            theme::BG_1
        };
        button::Style {
            background: Some(iced::Background::Color(bg)),
            text_color: theme::TEXT_1,
            border: iced::Border {
                color: if active { theme::ACCENT_LINE } else { theme::LINE_2 },
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        }
    })
    .into()
}

/// The repeat / fixed-bars stepper + live readout ("2-bar pattern →
/// 6 bars" for repeats, "Fixed · 4 bars" for a fixed span).
fn length_stepper<'a>(
    def_id: u64,
    index: usize,
    entry: &PatternEntry,
    patterns: &[DrumPattern],
) -> Element<'a, Message> {
    let span = bar_span_of(patterns, entry.pattern_id);
    let (value, is_repeat) = match entry.length {
        EntryLength::RepeatN(n) => (n, true),
        EntryLength::Bars(b) => (b, false),
    };
    let clamped = value.clamp(MIN_LEN, MAX_LEN);
    let make = move |v: u32| -> EntryLength {
        if is_repeat {
            EntryLength::RepeatN(v)
        } else {
            EntryLength::Bars(v)
        }
    };
    let dec = clamped.saturating_sub(1).max(MIN_LEN);
    let inc = (clamped + 1).min(MAX_LEN);

    let value_label = if is_repeat {
        format!("×{}", clamped)
    } else {
        format!("{} bars", clamped)
    };

    let stepper = row![
        stepper_button("−", clamped > MIN_LEN, arr(ArrangementMessage::SetEntryLength {
            definition_id: def_id,
            index,
            length: make(dec),
        })),
        container(
            text(value_label)
                .size(12)
                .font(theme::MONO_FONT)
                .color(theme::TEXT_1)
        )
        .width(Length::Fixed(74.0))
        .align_x(alignment::Horizontal::Center),
        stepper_button("+", clamped < MAX_LEN, arr(ArrangementMessage::SetEntryLength {
            definition_id: def_id,
            index,
            length: make(inc),
        })),
    ]
    .spacing(6)
    .align_y(alignment::Vertical::Center);

    let readout = if is_repeat {
        format!(
            "{span}-bar pattern → {} bars",
            clamped.saturating_mul(span)
        )
    } else {
        format!("Fixed · {clamped} bars")
    };

    column![
        field_block(if is_repeat { "REPEAT" } else { "BARS" }, stepper.into()),
        Space::new().height(5),
        text(readout).size(10).color(theme::WARM),
    ]
    .spacing(0)
    .into()
}

fn stepper_button<'a>(glyph: &str, enabled: bool, msg: Message) -> Element<'a, Message> {
    let mut btn = button(
        text(glyph.to_string())
            .size(13)
            .color(if enabled { theme::TEXT_1 } else { theme::TEXT_4 }),
    )
    .padding([2, 10])
    .style(|_theme, status| theme::small_button_style(status));
    if enabled {
        btn = btn.on_press(msg);
    }
    btn.into()
}

/// Fill-on-last-repeat toggle + fill-pattern dropdown. Enabling picks a
/// sensible default fill (first bank pattern that isn't the entry's own).
fn fill_controls<'a>(
    def_id: u64,
    index: usize,
    entry: &PatternEntry,
    patterns: &[DrumPattern],
) -> Element<'a, Message> {
    let fill_on = entry.fill.is_some();
    let default_fill = patterns
        .iter()
        .find(|p| p.id != entry.pattern_id)
        .or_else(|| patterns.first())
        .map(|p| p.id);

    let toggle_msg = if fill_on {
        arr(ArrangementMessage::SetEntryFill {
            definition_id: def_id,
            index,
            fill: None,
        })
    } else {
        arr(ArrangementMessage::SetEntryFill {
            definition_id: def_id,
            index,
            fill: default_fill,
        })
    };

    let toggle = toggle_row(
        "Fill on last repeat",
        fill_on,
        // Only offer the toggle when there's a pattern to fill with.
        default_fill.map(|_| toggle_msg),
    );

    let mut col = column![toggle].spacing(8);
    if let Some(fill_id) = entry.fill {
        let options = pattern_choices(patterns);
        let selected = options.iter().find(|c| c.id == fill_id).cloned();
        let picker = pick_list(options, selected, move |choice: PatternChoice| {
            arr(ArrangementMessage::SetEntryFill {
                definition_id: def_id,
                index,
                fill: Some(choice.id),
            })
        })
        .text_size(12)
        .padding([5, 8])
        .width(Length::Fill);
        col = col.push(picker);
    }
    col.into()
}

/// A dot + label toggle button matching the drum inspector's warm toggle
/// idiom. `msg` is `None` (disabled) when there's nothing to toggle.
fn toggle_row<'a>(label: &str, on: bool, msg: Option<Message>) -> Element<'a, Message> {
    let dot_color = if on { theme::WARM } else { theme::TEXT_4 };
    let dot = rail_dot(dot_color);
    let label_text = text(label.to_string())
        .size(11)
        .color(if on { theme::TEXT_1 } else { theme::TEXT_3 });
    let mut btn = button(
        row![dot, label_text]
            .spacing(8)
            .align_y(alignment::Vertical::Center)
            .width(Length::Fill),
    )
    .padding([6, 8])
    .width(Length::Fill)
    .style(move |_theme, status| {
        let bg = match status {
            button::Status::Hovered => theme::BG_3,
            _ => theme::BG_1,
        };
        button::Style {
            background: Some(iced::Background::Color(bg)),
            text_color: theme::TEXT_1,
            border: iced::Border {
                color: if on { theme::WARM_LINE } else { theme::LINE_2 },
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        }
    });
    if let Some(msg) = msg {
        btn = btn.on_press(msg);
    }
    btn.into()
}

/// Duplicate / Remove buttons for the selected entry.
fn entry_actions<'a>(def_id: u64, index: usize) -> Element<'a, Message> {
    let duplicate = button(text("Duplicate").size(11).color(theme::TEXT_2))
        .padding([6, 12])
        .width(Length::Fill)
        .on_press(arr(ArrangementMessage::DuplicateEntry {
            definition_id: def_id,
            index,
        }))
        .style(|_theme, status| theme::ghost_button_style(status));
    let remove = button(text("Remove").size(11).color(theme::WARM))
        .padding([6, 12])
        .width(Length::Fill)
        .on_press(arr(ArrangementMessage::RemoveEntry {
            definition_id: def_id,
            index,
        }))
        .style(|_theme, status| theme::ghost_button_style(status));

    row![duplicate, remove].spacing(8).width(Length::Fill).into()
}
