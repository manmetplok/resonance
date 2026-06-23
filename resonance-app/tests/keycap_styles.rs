//! Golden-image snapshot for the keyboard-shortcut presentation helpers
//! (epic #58, todo #632): `theme::keycap_row` rendered in each
//! `KeycapTone`, plus the active-row wash, the "edited" warm pill, and the
//! conflict ring. These are the theme foundations the command palette and
//! the Preferences › Keyboard panel build on, so a single golden locks in
//! how a sample binding list reads against the theme.
//!
//! On first run `matches_image()` writes the golden under
//! `tests/snapshots/`; subsequent runs diff against the committed PNG.

use iced::widget::{column, container, row, text, Space};
use iced::{alignment, Element, Length, Size};
use iced_test::simulator::Simulator;
use resonance_app::theme::{self, kbd, KeycapTone};

/// A compact card just big enough to frame the four sample rows.
const WINDOW: (f32, f32) = (440.0, 320.0);

/// Mirror the production font registration so the headless renderer shapes
/// the mono keycaps (and the modifier glyphs) the same way the app does.
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

/// A single command-name label.
fn name<'a>(label: &'a str) -> Element<'a, ()> {
    text(label).size(13).color(theme::TEXT_1).into()
}

/// Build a sample binding list exercising every helper added in todo #632.
fn sample_view<'a>() -> Element<'a, ()> {
    let cmd = kbd::CMD.to_string();
    let shift = kbd::SHIFT.to_string();
    let (cmd, shift) = (cmd.as_str(), shift.as_str());

    // Neutral resting row — plain keycaps.
    let neutral = row![
        name("Quantize"),
        Space::new().width(Length::Fill),
        theme::keycap_row::<()>(&[cmd, "Q"], KeycapTone::Neutral),
    ]
    .align_y(alignment::Vertical::Center);

    // Active (selected) row — lavender wash + accent keycaps.
    let active = container(
        row![
            name("Toggle Mixer"),
            Space::new().width(Length::Fill),
            theme::keycap_row::<()>(&[cmd, "2"], KeycapTone::Active),
        ]
        .align_y(alignment::Vertical::Center),
    )
    .padding([8.0, 12.0])
    .style(theme::active_row_style);

    // Edited row — warm "edited" pill next to a customised chord.
    let edited = row![
        name("Save Project"),
        container(text("edited").size(10).font(theme::MONO_FONT))
            .padding([1.0, 6.0])
            .style(theme::edited_pill_style),
        Space::new().width(Length::Fill),
        theme::keycap_row::<()>(&[cmd, shift, "S"], KeycapTone::Neutral),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);

    // Conflict row — BAD ring + BAD keycaps.
    let conflict = container(
        row![
            name("New Track"),
            Space::new().width(Length::Fill),
            theme::keycap_row::<()>(&[cmd, "T"], KeycapTone::Conflict),
        ]
        .align_y(alignment::Vertical::Center),
    )
    .padding([8.0, 12.0])
    .style(theme::conflict_ring_style);

    container(
        column![neutral, active, edited, conflict]
            .spacing(16)
            .width(Length::Fill),
    )
    .padding(20)
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::base_bg)
    .into()
}

#[test]
fn keycap_styles_sample_matches_golden() {
    let mut ui = Simulator::with_size(
        sim_settings(),
        Size::new(WINDOW.0, WINDOW.1),
        sample_view(),
    );
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image("tests/snapshots/keycap_styles_sample.png")
            .expect("matches_image i/o"),
        "keycap sample diverged from golden: tests/snapshots/keycap_styles_sample.png"
    );
}
