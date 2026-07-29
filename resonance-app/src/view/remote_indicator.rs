//! Remote-control-active indicator (ba doc #265, todo #1159).
//!
//! The only UI in epic #200: a small status chip in the window chrome
//! shown while one or more control clients (the `resonance-mcp` MCP
//! server, or any control-socket client) are connected — so the user can
//! see at a glance that an external AI/tool is driving the app live.
//! Hidden entirely when no client is connected.
//!
//! View-performance rules (feedback_view_performance): a cheap
//! conditional widget, no Canvas. When hidden it collapses to a
//! zero-width [`Space`] so the chrome layout never shifts.

use iced::widget::text::LineHeight;
use iced::widget::{container, row, text, tooltip, Space};
use iced::{alignment, Element};

use crate::message::Message;
use crate::theme;

/// The remote-control chip for the window chrome, or a zero-width spacer
/// when `client_count == 0`.
///
/// Placed in the chrome row (spacing 0), so the zero-width hidden state
/// introduces no gap and thus no layout shift. The visible chip is a
/// filled green dot + "Remote" label, themed like other status chips
/// (GOOD accent). A tooltip reports the exact client count.
pub(crate) fn view(client_count: usize) -> Element<'static, Message> {
    if client_count == 0 {
        return Space::new().width(0).into();
    }

    let dot = text("\u{25cf}")
        .size(8)
        .color(theme::GOOD)
        .line_height(LineHeight::Relative(1.0));

    let label = text("Remote")
        .size(11)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(theme::GOOD)
        .line_height(LineHeight::Relative(1.0));

    let chip = container(
        row![dot, Space::new().width(6), label]
            .align_y(alignment::Vertical::Center)
            .padding(0),
    )
    .padding([4, 10])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::GOOD_DIM)),
        border: iced::Border {
            color: theme::GOOD_LINE,
            width: 1.0,
            radius: theme::RADIUS_PILL.into(),
        },
        ..Default::default()
    });

    let tip_text = if client_count == 1 {
        "1 remote control client connected".to_string()
    } else {
        format!("{client_count} remote control clients connected")
    };
    let tip = container(text(tip_text).size(11).color(theme::TEXT_1))
        .padding(8)
        .style(|_theme: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_3)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        });

    // A trailing gap rides inside the element (matching the chrome's
    // other conditional affordances) so the visible chip has breathing
    // room from the title cluster without changing the hidden layout.
    row![
        Space::new().width(12),
        tooltip(chip, tip, tooltip::Position::Bottom).gap(4),
    ]
    .align_y(alignment::Vertical::Center)
    .into()
}
