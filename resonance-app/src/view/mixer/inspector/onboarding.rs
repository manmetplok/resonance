//! Onboarding card and status badge for external-instrument inspector
//! sections. These are shown for fresh/unconfigured external-instrument
//! tracks (doc #169 / todo #459).

use iced::widget::{column, container, row, text, Space};
use iced::{alignment, Element, Length};

use crate::message::Message;
use crate::state::ExternalInstrumentStatus;
use crate::theme;

/// Inspector status badge for an external-instrument track — Unconfigured
/// (accent), Configuring (warm), Live (good), Offline (bad). Mirrors the
/// prototype's `.badge` pill styling (todo #459, doc #169).
pub(super) fn status_badge(status: ExternalInstrumentStatus) -> Element<'static, Message> {
    let (label, fg, bg, line) = match status {
        ExternalInstrumentStatus::Unconfigured => (
            "Unconfigured",
            theme::ACCENT_SOFT,
            theme::ACCENT_DIM,
            theme::ACCENT_LINE,
        ),
        ExternalInstrumentStatus::Configuring => {
            ("Configuring", theme::WARM, theme::WARM_DIM, theme::WARM_LINE)
        }
        ExternalInstrumentStatus::Live => {
            ("Live", theme::GOOD, theme::GOOD_DIM, theme::GOOD_LINE)
        }
        ExternalInstrumentStatus::Offline => {
            ("Offline", theme::BAD, theme::BAD_DIM, theme::BAD_LINE)
        }
    };
    container(
        text(label)
            .size(9)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(fg),
    )
    .padding([2, 7])
    .style(move |_theme| container::Style {
        background: Some(iced::Background::Color(bg)),
        border: iced::Border {
            color: line,
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    })
    .into()
}

/// Dashed onboarding card shown for a fresh (Unconfigured) external
/// instrument track — an intro line plus four numbered setup steps (MIDI
/// out → return → patch → latency), mirroring the prototype's `.empty`
/// guidance block (todo #459, doc #169).
pub(super) fn onboarding_card() -> Element<'static, Message> {
    let intro = text(
        "External instrument track. Pair a hardware synth's MIDI output with its \
         audio return so it plays and records in-line like a built-in instrument. \
         To set it up:",
    )
    .size(11)
    .color(theme::TEXT_2);

    let steps = column![
        onboarding_step(1, "Pick the synth's MIDI output device + channel below."),
        Space::new().height(9),
        onboarding_step(2, "Pick the audio return input the synth is wired into."),
        Space::new().height(9),
        onboarding_step(
            3,
            "Choose a patch (Bank + Program) — Resonance re-sends it on load & play.",
        ),
        Space::new().height(9),
        onboarding_step(
            4,
            "Dial in latency compensation so the return lines up with the grid.",
        ),
    ]
    .spacing(0);

    container(column![intro, Space::new().height(10), steps].spacing(0))
        .width(Length::Fill)
        .padding(12)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: iced::Border {
                color: theme::ACCENT_LINE,
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        })
        .into()
}

/// One numbered onboarding step: a lavender numbered chip beside its label.
fn onboarding_step(n: u8, label: &'static str) -> Element<'static, Message> {
    let chip = container(
        text(n.to_string())
            .size(9)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::ACCENT_SOFT),
    )
    .width(Length::Fixed(16.0))
    .height(Length::Fixed(16.0))
    .align_x(alignment::Horizontal::Center)
    .align_y(alignment::Vertical::Center)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::ACCENT_DIM)),
        border: iced::Border {
            color: theme::ACCENT_LINE,
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    });

    row![
        chip,
        Space::new().width(9),
        text(label).size(11).color(theme::TEXT_2).width(Length::Fill),
    ]
    .align_y(alignment::Vertical::Top)
    .into()
}
