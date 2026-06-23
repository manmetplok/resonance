//! Golden-image swatch for the frost treatment.  (design doc #181)
//!
//! Frozen tracks are rendered as a *visual mode* derived from existing
//! tokens — a translucent cool wash over `BG_2`, a desaturated icy edge,
//! and a tinted snowflake glyph — NOT as a new palette hue. The three
//! constants `FROST_WASH` / `FROST_EDGE` / `FROST_ICON` are the single
//! source of truth for that mode.
//!
//! This renders a small swatch of all three (wash painted over a `BG_2`
//! card, an edge-outlined cell, and a solid icon-tint cell) so any drift
//! in the treatment — alpha, hue, or which token it derives from — trips
//! the golden diff. On first run `matches_image()` writes the golden
//! under `tests/snapshots/`; subsequent runs diff against it.

use iced::widget::{column, container, row, text, Space};
use iced::{Color, Length, Size};
use iced_test::simulator::Simulator;
use resonance_app::theme;

/// A 1:1 cell with the given background painted over a `BG_2` card, so a
/// translucent wash blends against the real frozen-header substrate.
fn cell<'a>(bg: Color, label: &'a str, border: Color) -> iced::Element<'a, ()> {
    container(
        column![
            container(Space::new())
                .width(Length::Fixed(72.0))
                .height(Length::Fixed(48.0))
                .style(move |_: &iced::Theme| container::Style {
                    background: Some(iced::Background::Color(bg)),
                    border: iced::Border {
                        color: border,
                        width: 1.5,
                        radius: theme::RADIUS_SM.into(),
                    },
                    ..Default::default()
                }),
            text(label).size(11).color(theme::TEXT_2),
        ]
        .spacing(6),
    )
    .padding(8)
    .style(|_: &iced::Theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        ..Default::default()
    })
    .into()
}

/// Register the same fonts the production app loads so the swatch labels
/// render with Geist rather than a platform fallback — keeps the golden
/// stable across machines.
fn sim_settings() -> iced::Settings {
    let mut fonts: Vec<std::borrow::Cow<'static, [u8]>> = Vec::new();
    fonts.push(theme::ICON_FONT_BYTES.into());
    for face in theme::UI_FONT_FACES {
        fonts.push((*face).into());
    }
    iced::Settings {
        fonts,
        default_font: theme::UI_FONT,
        ..iced::Settings::default()
    }
}

fn swatch_view<'a>() -> iced::Element<'a, ()> {
    container(
        row![
            cell(theme::FROST_WASH, "WASH", theme::LINE_2),
            cell(theme::BG_2, "EDGE", theme::FROST_EDGE),
            cell(theme::FROST_ICON, "ICON", theme::LINE_2),
        ]
        .spacing(12),
    )
    .padding(16)
    .style(|_: &iced::Theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_1)),
        ..Default::default()
    })
    .into()
}

#[test]
fn frost_treatment_swatch() {
    let mut ui = Simulator::with_size(sim_settings(), Size::new(320.0, 120.0), swatch_view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image("tests/snapshots/frost_treatment_swatch.png")
            .expect("matches_image i/o"),
        "frost swatch diverged from golden — the freeze treatment \
         (FROST_WASH / FROST_EDGE / FROST_ICON) changed"
    );
}
