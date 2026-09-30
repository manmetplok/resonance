//! The command palette overlay (command-palette.md §7.1): the epic #58
//! prototype — a 640 px card anchored 96 px from the top, a search row,
//! grouped results with glyph, highlighted name, breadcrumb and keycaps, and
//! a hint footer.
//!
//! Pure rendering: every row was resolved by the reducer
//! (`palette::build`), so this only reads `r.ui.palette`.

use iced::widget::text::Span;
use iced::widget::{
    column, container, mouse_area, rich_text, row, scrollable, span, stack, text, text_input,
    Space,
};
use iced::{alignment, Color, Element, Length};

use crate::commands::Platform;
use crate::message::{Message, UiMessage};
use crate::palette::{self, PaletteMsg, PaletteRow, PaletteState};
use crate::theme::{self, KeycapTone};
use crate::update::palette::{msg, HEADER_HEIGHT, LIST_HEIGHT, ROW_HEIGHT};
use crate::Resonance;

/// Card width and its distance from the window top.
const CARD_WIDTH: f32 = 640.0;
const CARD_TOP: f32 = 96.0;

pub(crate) fn view_palette_overlay(r: &Resonance) -> Element<'_, Message> {
    let Some(state) = r.ui.palette.as_ref() else {
        return Space::new().into();
    };
    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.6))),
                ..Default::default()
            }),
    )
    .on_press(Message::Ui(UiMessage::ClosePalette));

    let card = container(
        column![
            search_row(state),
            hairline(),
            results(state, Platform::current()),
            footer(state),
        ]
        .spacing(0),
    )
    .width(CARD_WIDTH)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE,
            width: 1.0,
            radius: theme::RADIUS_XL.into(),
        },
        ..Default::default()
    });

    // Card-relative pointer reports tell a real move from a row appearing
    // or scrolling under a resting pointer (see `PaletteMsg::Hover`).
    let card = mouse_area(card).on_move(|at| msg(PaletteMsg::PointerMoved(at)));
    let placed = container(iced::widget::opaque(card))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Horizontal::Center)
        .align_y(alignment::Vertical::Top)
        .padding(iced::Padding {
            top: CARD_TOP,
            ..iced::Padding::ZERO
        });

    stack![backdrop, placed].into()
}

fn hairline<'a>() -> Element<'a, Message> {
    container(Space::new().width(Length::Fill).height(1))
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::LINE_2)),
            ..Default::default()
        })
        .into()
}

fn search_row(state: &PaletteState) -> Element<'_, Message> {
    let input = text_input("Search commands…", &state.query)
        .id(palette::query_input_id())
        .on_input(|q| msg(PaletteMsg::Query(q)))
        .on_submit(msg(PaletteMsg::Submit))
        .size(17)
        .padding(0)
        .style(|_theme, _status| text_input::Style {
            background: iced::Background::Color(Color::TRANSPARENT),
            border: iced::Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 0.0.into(),
            },
            icon: theme::TEXT_3,
            placeholder: theme::TEXT_3,
            value: theme::TEXT_1,
            selection: Color {
                a: 0.35,
                ..theme::ACCENT
            },
        });
    row![
        theme::icon(theme::fa::MAGNIFYING_GLASS)
            .size(15)
            .color(theme::TEXT_3),
        input,
        theme::keycap("Esc", KeycapTone::Neutral),
    ]
    .spacing(12)
    .align_y(alignment::Vertical::Center)
    .padding([16, 18])
    .into()
}

/// The result list, or the empty state.
fn results(state: &PaletteState, platform: Platform) -> Element<'_, Message> {
    if state.row_count() == 0 {
        return empty_state(state);
    }
    let mut list = column![].spacing(0).padding([0, 6]);
    let mut index = 0;
    let mut content_height = 0.0;
    for section in &state.sections {
        list = list.push(section_header(&section.title));
        content_height += HEADER_HEIGHT;
        for r in &section.rows {
            list = list.push(result_row(r, index, index == state.selected, platform));
            content_height += ROW_HEIGHT;
            index += 1;
        }
    }
    let height = (content_height + 6.0).min(LIST_HEIGHT);
    scrollable(list.push(Space::new().height(6)))
        .id(palette::list_id())
        .height(height)
        .on_scroll(|viewport| msg(PaletteMsg::Scrolled(viewport.absolute_offset().y)))
        .into()
}

fn section_header(title: &str) -> Element<'_, Message> {
    container(
        text(title.to_uppercase())
            .size(10)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
    )
    .height(HEADER_HEIGHT)
    .padding(iced::Padding {
        top: 12.0,
        left: 12.0,
        right: 12.0,
        bottom: 6.0,
    })
    .align_y(alignment::Vertical::Bottom)
    .into()
}

/// The name, with the fuzzy-matched runs in `ACCENT_SOFT`.
fn highlighted_name<'a>(row: &'a PaletteRow, base: Color) -> Element<'a, Message> {
    let chars: Vec<char> = row.name.chars().collect();
    let mut spans: Vec<Span<'a, (), iced::Font>> = Vec::new();
    let mut cursor = 0;
    let piece = |from: usize, to: usize| chars[from..to].iter().collect::<String>();
    for &(start, end) in &row.ranges {
        let (start, end) = (start.min(chars.len()), end.min(chars.len()));
        if start > cursor {
            spans.push(span(piece(cursor, start)).color(base));
        }
        if end > start {
            spans.push(
                span(piece(start, end))
                    .color(theme::ACCENT_SOFT)
                    .font(theme::UI_FONT_SEMIBOLD),
            );
        }
        cursor = end.max(cursor);
    }
    if cursor < chars.len() {
        spans.push(span(piece(cursor, chars.len())).color(base));
    }
    rich_text(spans).size(14).into()
}

fn result_row(row_: &PaletteRow, index: usize, active: bool, platform: Platform) -> Element<'_, Message> {
    let dim = row_.unavailable.is_some();
    let name_color = if dim { theme::TEXT_3 } else { theme::TEXT_1 };

    let glyph_tile = container(
        theme::icon(row_.glyph.unwrap_or(' '))
            .size(12)
            .color(if active && !dim { theme::BG_0 } else { theme::TEXT_2 }),
    )
    .width(26)
    .height(26)
    .align_x(alignment::Horizontal::Center)
    .align_y(alignment::Vertical::Center)
    .style(move |_theme| container::Style {
        background: Some(iced::Background::Color(if active && !dim {
            theme::ACCENT
        } else {
            theme::BG_3
        })),
        border: iced::Border {
            radius: theme::RADIUS_SM.into(),
            ..Default::default()
        },
        ..Default::default()
    });

    let label = column![
        highlighted_name(row_, name_color),
        text(&row_.breadcrumb).size(11).color(theme::TEXT_3),
    ]
    .spacing(1);

    let mut line = row![glyph_tile, label, Space::new().width(Length::Fill)]
        .spacing(14)
        .align_y(alignment::Vertical::Center);
    if let Some(reason) = row_.unavailable {
        line = line.push(text(reason).size(11).color(theme::TEXT_3));
    }
    if let Some(chord) = row_.chord {
        let caps = chord.keycaps(platform);
        let labels: Vec<&str> = caps.iter().map(String::as_str).collect();
        let tone = if active && !dim {
            KeycapTone::Active
        } else {
            KeycapTone::Neutral
        };
        line = line.push(theme::keycap_row(&labels, tone));
    }

    let body = container(line)
        .height(ROW_HEIGHT)
        .padding([0, 12])
        .align_y(alignment::Vertical::Center)
        .width(Length::Fill);
    let body = if active {
        body.style(theme::active_row_style)
    } else {
        body
    };
    mouse_area(body)
        .on_move(move |_| msg(PaletteMsg::Hover(index)))
        .on_press(msg(PaletteMsg::Click(index)))
        .into()
}

fn empty_state(state: &PaletteState) -> Element<'_, Message> {
    if palette::PaletteMode::of(&state.query).0 != palette::PaletteMode::Commands {
        return container(
            text(palette::empty_message(&state.query))
                .size(15)
                .color(theme::TEXT_2),
        )
        .width(Length::Fill)
        .padding([48, 20])
        .align_x(alignment::Horizontal::Center)
        .into();
    }
    container(
        column![
            row![
                text("No commands match ").size(15).color(theme::TEXT_2),
                text(format!("\u{201c}{}\u{201d}", state.query))
                    .size(15)
                    .font(theme::MONO_FONT)
                    .color(theme::TEXT_1),
            ],
            text("Try a shorter query, or search by what it does").size(12).color(theme::TEXT_3),
        ]
        .spacing(6)
        .align_x(alignment::Horizontal::Center),
    )
    .width(Length::Fill)
    .padding([48, 20])
    .align_x(alignment::Horizontal::Center)
    .into()
}

fn footer(state: &PaletteState) -> Element<'_, Message> {
    let hint = |caps: &[&str], label: &'static str| {
        row![
            theme::keycap_row(caps, KeycapTone::Neutral),
            text(label).size(11).color(theme::TEXT_3)
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center)
    };
    let up = theme::kbd::ARROW_UP.to_string();
    let down = theme::kbd::ARROW_DOWN.to_string();
    let enter = theme::kbd::ENTER.to_string();
    let right: Element<'_, Message> = match state.flash {
        Some(reason) => text(reason).size(11).color(theme::BAD).into(),
        None if state.query.is_empty() => text(": bar   @ jump   # track   + plugin")
            .size(11)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_3)
            .into(),
        None => {
            let n = state.row_count();
            text(if n == 1 {
                "1 result".to_string()
            } else {
                format!("{n} results")
            })
            .size(11)
            .color(theme::TEXT_3)
            .into()
        }
    };
    container(
        row![
            hint(&[up.as_str(), down.as_str()], "navigate"),
            hint(&[enter.as_str()], "run"),
            hint(&["Esc"], "close"),
            Space::new().width(Length::Fill),
            right,
        ]
        .spacing(18)
        .align_y(alignment::Vertical::Center),
    )
    .padding([9, 16])
    .width(Length::Fill)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_1)),
        border: iced::Border {
            radius: iced::border::Radius {
                top_left: 0.0,
                top_right: 0.0,
                bottom_right: theme::RADIUS_XL,
                bottom_left: theme::RADIUS_XL,
            },
            ..Default::default()
        },
        ..Default::default()
    })
    .into()
}
