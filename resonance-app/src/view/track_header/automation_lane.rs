//! Slim automation-lane header cell for the Arrange track-header column
//! (doc #256, todo #1098).
//!
//! The column iterates the same [`ArrangeRowLayout`](crate::view::
//! arrange_layout::ArrangeRowLayout) as the timeline canvas, so every
//! `ArrangeRowKind::AutomationLane` row the canvas draws (todo #1097) gets
//! a matching 44 px header cell here: the lane's parameter label — device
//! params resolved to their definition names, falling back to the raw id —
//! styled subordinate to the track header (smaller, dimmer, indented under
//! the track name).
//!
//! The cell follows the column's row-pitch convention: the body is
//! `height - 1` px with a 1 px hairline below, so `cell + hairline` sum to
//! exactly the canvas row height and the column stays glued to its lanes
//! (same trick as `view_track_header` / `view_group_header`).

use iced::widget::{column, container, text, Space};
use iced::{alignment, Element, Length};

use crate::message::Message;
use crate::theme;

/// Render the header cell for one automation-lane sub-row. `label` is the
/// lane's resolved parameter name (see
/// [`crate::view::timeline::automation::target_label`]); `height` is the
/// row's height from the shared layout (44 px), so the cell can never
/// drift from the canvas row it mirrors.
pub(crate) fn view_automation_lane_header(
    label: String,
    height: f32,
) -> Element<'static, Message> {
    let name = text(label)
        .size(10)
        .color(theme::TEXT_2)
        .wrapping(iced::widget::text::Wrapping::None);

    // Indented under the track name (24 px cell padding + 28 px glyph +
    // 12 px gap = 64 px), so lane labels read as children of their track.
    let body = container(container(name).clip(true))
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
            // Recessed substrate matching the canvas's lane-row band
            // (`theme::BG_2` there), so header and lane read as one row.
            background: Some(iced::Background::Color(theme::BG_2)),
            ..Default::default()
        });

    // `height - 1` + 1 px hairline = the canvas row pitch, exactly.
    let cell = container(body)
        .width(Length::Fill)
        .height(height - 1.0);
    let hairline = container(Space::new().width(Length::Fill))
        .height(1)
        .style(theme::separator_bg);

    column![cell, hairline].spacing(0).into()
}
