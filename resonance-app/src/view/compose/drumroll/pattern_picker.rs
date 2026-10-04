//! ARRANGEMENT strip above the drumroll lane (design doc #170).
//!
//! Evolves the old single-pattern picker strip into an ordered
//! *arrangement* editor. The strip keeps the lane's side-tag + content
//! split (`BG_1` side / `BG_2` content) and renders the section's ordered
//! [`Vec<PatternEntry>`](crate::compose::PatternEntry) as a row of **entry
//! chips**. Each chip carries a pattern colour dot + name, an inline `×N`
//! repeat stepper, the computed bar span (`2b ×3 = 6b`), an optional warm
//! `▸ FILL` badge, and a remove `×`. A trailing dashed-ghost `＋ Add
//! pattern` appends an entry, and a **coverage pill** reports whether the
//! entries fill the section exactly (green), fall short (warm `gap`), or
//! overflow (pink `overflow`).
//!
//! Under-fill and over-fill also surface a remediation **banner** below the
//! strip: a warm caret banner with a one-click **Fill to end** for gaps, and
//! a pink error banner with **Trim to fit** for overflow. Per the
//! accessibility rule these states read as colour **and** shape **and**
//! text, never colour alone.
//!
//! All colours, radii, and fonts come from `theme.rs`; the chip/banner/
//! segmented conventions mirror the rest of the Compose workspace. Chip
//! actions route through [`ArrangementMessage`] (todo #484); coverage +
//! span labels come from the pure resolver helpers (todo #483).

use iced::widget::{button, container, row, text, Space};
use iced::{alignment, Border, Color, Element, Length};

use crate::compose::messages::ArrangementMessage;
use crate::compose::{
    entry_span_label, entry_stepper_label, entry_stepper_value, step_entry_length,
    ArrangementCoverage, ComposeMessage, DrumPattern, PatternEntry, SectionDefinitionState,
};
use crate::message::Message;
use crate::theme;
use crate::Resonance;

use super::super::tracks::NAME_COLUMN_WIDTH;

/// Height of a remediation banner row.
const BANNER_HEIGHT: f32 = 28.0;

fn arrangement(msg: ArrangementMessage) -> Message {
    Message::Compose(ComposeMessage::Arrangement(msg))
}

/// Build the arrangement strip (+ any gap/overflow banner). `width` is the
/// fixed workspace width — the same value `chord_lane` and `tracks` use so
/// the strip aligns with the rest of the lane stack.
pub fn arrangement_strip<'a>(
    app: &'a Resonance,
    definition: &'a SectionDefinitionState,
    width: f32,
) -> Element<'a, Message> {
    // Side panel — "ARRANGEMENT · {section}" tag, matching the lane-side
    // header treatment used by the chord / vocal / drum lanes.
    let side = container(column_label("ARRANGEMENT", &definition.name))
        .width(Length::Fixed(NAME_COLUMN_WIDTH))
        .height(Length::Fill)
        .padding([0, 12])
        .align_y(alignment::Vertical::Center)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            ..Default::default()
        });

    // Ordered entry chips. A missing pattern (deleted from the bank) still
    // renders a neutral chip so the entry can be removed.
    let mut chips: Vec<Element<'a, Message>> = Vec::with_capacity(definition.arrangement.len() + 2);
    for (index, entry) in definition.arrangement.iter().enumerate() {
        let pattern = app.compose.find_pattern(entry.pattern_id);
        chips.push(entry_chip(definition.id, index, entry, pattern));
    }

    // Trailing dashed-ghost "＋ Add pattern". Appends the section's current
    // default pattern as a fresh single-repeat entry; disabled when the
    // bank is empty (nothing to add).
    let add_pattern_id = app.compose.pattern_for_definition(definition).map(|p| p.id);
    chips.push(add_button(definition.id, add_pattern_id));

    // Coverage pill. An empty arrangement means "the default pattern fills
    // the whole section", so it reads as a neutral default rather than a
    // gap needing remediation.
    let coverage = section_coverage(app, definition);
    chips.push(coverage_pill(coverage, definition.length_bars));

    let chip_row = iced::widget::Row::with_children(chips)
        .spacing(6)
        .align_y(alignment::Vertical::Center)
        .wrap();

    let body = container(chip_row)
        .padding([6, 12])
        .width(Length::Fill)
        .height(Length::Shrink)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        });

    let strip = container(
        row![side, body]
            .spacing(0)
            .align_y(alignment::Vertical::Top),
    )
    .width(Length::Fixed(width))
    .height(Length::Shrink);

    // Stack the strip with an optional remediation banner. Only one banner
    // shows at a time (an arrangement can't both gap and overflow).
    let mut rows: Vec<Element<'a, Message>> = vec![strip.into()];
    match coverage {
        Some(ArrangementCoverage::Gap { bars }) => {
            rows.push(gap_banner(definition.id, bars, width));
        }
        Some(ArrangementCoverage::Overflow { bars }) => {
            rows.push(overflow_banner(definition.id, bars, width));
        }
        _ => {}
    }

    iced::widget::Column::with_children(rows)
        .width(Length::Fixed(width))
        .into()
}

/// Coverage of the section's arrangement. `None` for an empty arrangement —
/// which is not a gap but "the default pattern covers everything", so it
/// carries no remediation banner and reads as a neutral pill.
fn section_coverage(
    app: &Resonance,
    definition: &SectionDefinitionState,
) -> Option<ArrangementCoverage> {
    if definition.arrangement.is_empty() {
        None
    } else {
        Some(app.compose.resolve_arrangement_for(definition).coverage)
    }
}

/// Two-line tag rendered in the lane's side column ("ARRANGEMENT" / section
/// name). Mirrors `lane_side::draw`'s aesthetic as a real Iced element.
fn column_label<'a>(kicker: &str, name: &str) -> Element<'a, Message> {
    iced::widget::column![
        text(kicker.to_string())
            .size(9)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
        text(name.to_string()).size(12).color(theme::TEXT_1),
    ]
    .spacing(0)
    .into()
}

/// One arrangement entry chip: colour dot + name, inline `×N` stepper, bar
/// span readout, optional `▸ FILL` badge, and a remove `×`.
fn entry_chip<'a>(
    definition_id: u64,
    index: usize,
    entry: &PatternEntry,
    pattern: Option<&DrumPattern>,
) -> Element<'a, Message> {
    let accent = pattern
        .map(|p| u8_color(p.color))
        .unwrap_or(theme::TEXT_3);
    let name = pattern.map(|p| p.name.clone()).unwrap_or_else(|| "?".into());
    let pattern_bars = pattern.map(|p| p.bar_span()).unwrap_or(1);

    let dot = container(Space::new().width(Length::Fixed(6.0)).height(Length::Fixed(6.0)))
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(accent)),
            border: Border {
                color: accent,
                width: 0.0,
                radius: 999.0.into(),
            },
            ..Default::default()
        });

    let label = text(name).size(11).color(theme::TEXT_1);

    // Inline ×N stepper. The `−` button is disabled at the value floor of 1
    // so a stepper never drives an entry to a zero-bar span.
    let value = entry_stepper_value(entry.length);
    let dec = stepper_button(
        "-",
        (value > 1).then(|| {
            arrangement(ArrangementMessage::SetEntryLength {
                definition_id,
                index,
                length: step_entry_length(entry.length, -1),
            })
        }),
    );
    let count = text(entry_stepper_label(entry.length))
        .size(10)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_1);
    let inc = stepper_button(
        "+",
        Some(arrangement(ArrangementMessage::SetEntryLength {
            definition_id,
            index,
            length: step_entry_length(entry.length, 1),
        })),
    );
    let stepper = row![dec, count, inc]
        .spacing(4)
        .align_y(alignment::Vertical::Center);

    let span = text(entry_span_label(entry.length, pattern_bars))
        .size(9)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_3);

    let mut inner = row![dot, label, stepper, span]
        .spacing(8)
        .align_y(alignment::Vertical::Center);

    // Warm "▸ FILL" badge when the entry caps its last bar with a fill.
    if entry.fill.is_some() {
        inner = inner.push(fill_badge());
    }

    // Remove "×".
    inner = inner.push(stepper_button(
        "×",
        Some(arrangement(ArrangementMessage::RemoveEntry {
            definition_id,
            index,
        })),
    ));

    container(inner)
        .padding([5, 8])
        .align_y(alignment::Vertical::Center)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(Color { a: 0.14, ..accent })),
            text_color: Some(theme::TEXT_1),
            border: Border {
                color: Color { a: 0.55, ..accent },
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Small square stepper / action button (`−`, `+`, `×`). Ghost styling; a
/// `None` action renders it disabled (used for the `−` floor).
fn stepper_button<'a>(glyph: &'static str, on_press: Option<Message>) -> Element<'a, Message> {
    let body = text(glyph).size(11).color(theme::TEXT_2);
    let mut btn = button(
        container(body)
            .width(Length::Fixed(14.0))
            .align_x(alignment::Horizontal::Center),
    )
    .padding([1, 2])
    .style(|_theme, status| theme::ghost_button_style(status));
    if let Some(msg) = on_press {
        btn = btn.on_press(msg);
    }
    btn.into()
}

/// Warm `▸ FILL` badge: caret icon + "FILL" in the warm accent.
fn fill_badge<'a>() -> Element<'a, Message> {
    let caret = theme::icon(theme::fa::CARET_RIGHT).size(8).color(theme::WARM);
    let label = text("FILL")
        .size(9)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(theme::WARM);
    container(
        row![caret, label]
            .spacing(3)
            .align_y(alignment::Vertical::Center),
    )
    .padding([2, 5])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(Color { a: 0.14, ..theme::WARM })),
        border: Border {
            color: theme::WARM_LINE,
            width: 1.0,
            radius: theme::RADIUS_XS.into(),
        },
        ..Default::default()
    })
    .into()
}

/// Trailing dashed-ghost "＋ Add pattern". Disabled (no `on_press`) when the
/// pattern bank is empty.
fn add_button<'a>(definition_id: u64, pattern_id: Option<u64>) -> Element<'a, Message> {
    let body = row![
        text("+").size(11).color(theme::TEXT_2),
        text("Add pattern").size(10).color(theme::TEXT_2),
    ]
    .spacing(5)
    .align_y(alignment::Vertical::Center);

    let mut btn = button(body).padding([5, 10]).style(|_theme, status| {
        let hovered = matches!(status, button::Status::Hovered);
        button::Style {
            background: Some(iced::Background::Color(if hovered {
                theme::BG_3
            } else {
                Color::TRANSPARENT
            })),
            text_color: theme::TEXT_2,
            border: Border {
                // Dashed-ghost cue via a dim, low-contrast outline (Iced has
                // no dashed border primitive; the muted colour + transparent
                // fill reads as the "ghost add" affordance used elsewhere).
                color: theme::TEXT_4,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        }
    });
    if let Some(pattern_id) = pattern_id {
        btn = btn.on_press(arrangement(ArrangementMessage::AddEntry {
            definition_id,
            pattern_id,
        }));
    }
    btn.into()
}

/// Coverage pill chip. Green when exact, warm when under-filled (`gap`),
/// pink when over-filled (`overflow`); each state pairs its colour with a
/// distinct leading glyph + the gap/overflow wording so the meaning never
/// rests on colour alone. `None` coverage (empty arrangement) reads as a
/// neutral "default fills section" pill.
fn coverage_pill<'a>(
    coverage: Option<ArrangementCoverage>,
    section_bars: u32,
) -> Element<'a, Message> {
    let (accent, glyph, label): (Color, Option<char>, String) = match coverage {
        None => (
            theme::TEXT_3,
            None,
            format!("default · {section_bars} bars"),
        ),
        Some(ArrangementCoverage::Exact) => (
            theme::GOOD,
            None,
            coverage.unwrap().chip_label(section_bars),
        ),
        Some(ArrangementCoverage::Gap { .. }) => (
            theme::WARM,
            Some(theme::fa::CARET_RIGHT),
            coverage.unwrap().chip_label(section_bars),
        ),
        Some(ArrangementCoverage::Overflow { .. }) => (
            theme::BAD,
            Some(theme::fa::CIRCLE_INFO),
            coverage.unwrap().chip_label(section_bars),
        ),
    };

    let mut inner = row![].spacing(5).align_y(alignment::Vertical::Center);
    if let Some(g) = glyph {
        inner = inner.push(theme::icon(g).size(9).color(accent));
    }
    inner = inner.push(
        text(label)
            .size(10)
            .font(theme::MONO_FONT)
            .color(accent),
    );

    container(inner)
        .padding([4, 10])
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(Color { a: 0.12, ..accent })),
            border: Border {
                color: Color { a: 0.5, ..accent },
                width: 1.0,
                radius: 999.0.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Non-destructive GAP banner: warm caret + text + one-click "Fill to end".
fn gap_banner<'a>(definition_id: u64, bars: u32, width: f32) -> Element<'a, Message> {
    let plural = if bars == 1 { "bar" } else { "bars" };
    let message = format!("{bars} {plural} uncovered · drum lane goes silent");
    remediation_banner(
        theme::WARM,
        theme::WARM_LINE,
        theme::fa::CARET_RIGHT,
        message,
        "Fill to end",
        arrangement(ArrangementMessage::FillToEnd { definition_id }),
        width,
    )
}

/// OVERFLOW error banner: pink marker + text + "Trim to fit".
fn overflow_banner<'a>(definition_id: u64, bars: u32, width: f32) -> Element<'a, Message> {
    let plural = if bars == 1 { "bar" } else { "bars" };
    let message = format!("{bars} {plural} overflow · clipped, won't play or bounce");
    remediation_banner(
        theme::BAD,
        Color { a: 0.34, ..theme::BAD },
        theme::fa::CIRCLE_INFO,
        message,
        "Trim to fit",
        arrangement(ArrangementMessage::TrimToFit { definition_id }),
        width,
    )
}

/// Shared banner body: a left accent marker + message on `BG_2`, with a
/// trailing remediation button. Colour + glyph shape + text together carry
/// the state (accessibility rule).
fn remediation_banner<'a>(
    accent: Color,
    edge: Color,
    glyph: char,
    message: String,
    action_label: &'static str,
    action: Message,
    width: f32,
) -> Element<'a, Message> {
    let marker = theme::icon(glyph).size(11).color(accent);
    let body = text(message).size(11).color(theme::TEXT_1);

    let action_btn = button(
        text(action_label)
            .size(10)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(accent),
    )
    .padding([3, 10])
    .on_press(action)
    .style(move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered);
        button::Style {
            background: Some(iced::Background::Color(Color {
                a: if hovered { 0.24 } else { 0.14 },
                ..accent
            })),
            text_color: accent,
            border: Border {
                color: edge,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        }
    });

    // Marker · message · remediation button, read left-to-right. The banner
    // background spans the full workspace width via the fixed-width
    // container below; the row itself stays shrink-left.
    let inner = row![marker, body, action_btn]
        .spacing(12)
        .align_y(alignment::Vertical::Center);

    container(inner)
        .width(Length::Fixed(width))
        .height(Length::Fixed(BANNER_HEIGHT))
        .padding([0, 12])
        .align_y(alignment::Vertical::Center)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: Border {
                color: edge,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}

fn u8_color(rgb: [u8; 3]) -> Color {
    Color::from_rgb(
        rgb[0] as f32 / 255.0,
        rgb[1] as f32 / 255.0,
        rgb[2] as f32 / 255.0,
    )
}
