//! The status lines between the transport and the main area
//! (`Resonance::view_status_area`): the persistent health / autosave
//! lines and the dismissable error banner. Moved out of `view/mod.rs`
//! (ARCH2-12: a `mod.rs` re-exports and dispatches).

use iced::widget::{button, container, row, text, Space};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::theme;

/// One persistent status line: a tag chip and the status text, on a
/// tinted band. No dismiss button — it clears when the condition does.
pub(super) fn status_line<'a>(
    tag: &'static str,
    status: impl text::IntoFragment<'a>,
    tone: iced::Color,
    wash: iced::Color,
) -> Element<'a, Message> {
    let chip = container(text(tag).size(11).color(tone))
        .padding([1, 6])
        .style(move |_theme| container::Style {
            border: iced::Border {
                color: tone,
                width: 1.0,
                radius: 3.0.into(),
            },
            ..Default::default()
        });
    container(
        row![chip, text(status).size(13).color(theme::TEXT)]
            .spacing(8)
            .align_y(alignment::Vertical::Center)
            .padding([6, 8]),
    )
    .width(Length::Fill)
    .style(move |_theme| container::Style {
        background: Some(iced::Background::Color(wash)),
        ..Default::default()
    })
    .into()
}

/// The transient, dismissable error banner. Previously a solid `BAD` fill
/// with white text — `BAD` is a light soft-pink, so *no* text colour reads
/// at 4.5:1 against it (pure white itself is only ≈2.75:1). Restyled to the
/// same tinted-wash + tag-chip language as the persistent [`status_line`]s
/// above it: a `BAD`-outlined "ERROR" chip on a `BAD_DIM` wash, `TEXT_1`
/// body text (code review UX-07).
pub(super) fn error_bar(err: &str) -> Element<'_, Message> {
    let chip = container(text("ERROR").size(11).color(theme::BAD))
        .padding([1, 6])
        .style(move |_theme| container::Style {
            border: iced::Border {
                color: theme::BAD,
                width: 1.0,
                radius: 3.0.into(),
            },
            ..Default::default()
        });
    let dismiss = button(text("\u{00d7}").size(14).color(theme::TEXT_2))
        .on_press(Message::Ui(UiMessage::DismissError))
        .style(|_theme, status| theme::small_button_style(status));
    container(
        row![
            chip,
            text(err).size(13).color(theme::TEXT_1),
            Space::new().width(Length::Fill),
            dismiss,
        ]
        .spacing(8)
        .align_y(alignment::Vertical::Center)
        .padding([6, 8]),
    )
    .width(Length::Fill)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BAD_DIM)),
        ..Default::default()
    })
    .into()
}
