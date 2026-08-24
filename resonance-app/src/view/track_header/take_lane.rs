//! Slim take-row header cell for the Arrange track-header column
//! (epic #15, doc #165, todo #413).
//!
//! The column iterates the same [`ArrangeRowLayout`](crate::view::
//! arrange_layout::ArrangeRowLayout) as the timeline canvas, so every
//! `ArrangeRowKind::TakeRow` the canvas draws gets a matching
//! `TAKE_ROW_HEIGHT` cell here: the take's name (`Take 3`) plus a state
//! chip saying what the comp does with it — `SOLO` when the take is the
//! group's active take, `N seg` when it contributes segments, nothing when
//! it is unused (and then the name dims, so an unused take reads as
//! subordinate without needing a second glance).
//!
//! The cell follows the column's row-pitch convention: the body is
//! `height - 1` px with a 1 px hairline below, so `cell + hairline` sum to
//! exactly the canvas row height and the column stays glued to its lanes
//! (same trick as `view_track_header` / `view_automation_lane_header`).

use iced::widget::{column, container, row, text, Space};
use iced::{alignment, Element, Length};

use crate::message::Message;
use crate::theme;

/// What the comp does with a take — the cell's trailing chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TakeRole {
    /// The group's active take: soloed over the whole slot.
    Active,
    /// Audible over `n` spans of the effective cover.
    Comped(usize),
    /// Not audible anywhere in the current comp.
    Unused,
}

/// Render the header cell for one take sub-row. `label` is the take's
/// display name; `height` is the row's height from the shared layout, so
/// the cell can never drift from the canvas row it mirrors.
pub(crate) fn view_take_lane_header(
    label: String,
    role: TakeRole,
    height: f32,
) -> Element<'static, Message> {
    let name = text(label)
        .size(10)
        .color(match role {
            TakeRole::Active => theme::WARM,
            TakeRole::Comped(_) => theme::TEXT_2,
            TakeRole::Unused => theme::TEXT_3,
        })
        .wrapping(iced::widget::text::Wrapping::None);

    let chip: Element<'static, Message> = match role {
        TakeRole::Active => chip_text("SOLO", theme::WARM),
        TakeRole::Comped(n) => chip_text(&format!("{n} seg"), theme::ACCENT_SOFT),
        TakeRole::Unused => Space::new().width(0).height(0).into(),
    };

    // Indented under the track name (24 px cell padding + 28 px glyph +
    // 12 px gap = 64 px), matching the automation lane cells so both
    // sub-row stacks share one left edge.
    let body = container(
        row![
            container(name).clip(true),
            Space::new().width(Length::Fill),
            chip,
        ]
        .align_y(alignment::Vertical::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_y(alignment::Vertical::Center)
    .padding(iced::Padding {
        top: 0.0,
        right: 12.0,
        bottom: 0.0,
        left: 64.0,
    })
    .style(|_theme| container::Style {
        // Recessed substrate matching the canvas's take-row band
        // (`theme::BG_2` there), so header and lane read as one row.
        background: Some(iced::Background::Color(theme::BG_2)),
        ..Default::default()
    });

    // `height - 1` + 1 px hairline = the canvas row pitch, exactly.
    let cell = container(body).width(Length::Fill).height(height - 1.0);
    let hairline = container(Space::new().width(Length::Fill))
        .height(1)
        .style(theme::separator_bg);

    column![cell, hairline].spacing(0).into()
}

fn chip_text(label: &str, color: iced::Color) -> Element<'static, Message> {
    text(label.to_string())
        .size(9)
        .font(theme::MONO_FONT)
        .color(color)
        .into()
}
