//! Golden-image snapshots for the **floating selection bar**
//! (epic #36, doc #200, todo #684).
//!
//! When two or more track headers are selected in the Arrange view,
//! a small pill floats above the bottom edge reading
//! "N tracks selected · … — Group ⌘G". The bar is deliberately
//! *non-modal*: it fills the surface so it can centre itself, but only
//! the pill is `opaque`, so every click outside it falls through to
//! the arrange view underneath.
//!
//! This component is snapshotted standalone via the
//! `test_selection_bar_view` accessor, testing two states:
//!
//! 1. **2 tracks selected** — the minimum threshold for showing the bar.
//! 2. **N tracks selected** — a larger selection (5 tracks).

mod common;

use iced::widget::container;
use iced::{Length, Size};
use iced_test::simulator::Simulator;
use resonance_app::{theme, Resonance};

/// Snapshot canvas — wide enough to fit the pill content at its
/// real pitch, with vertical slack for the bottom placement.
const CANVAS: (f32, f32) = (800.0, 120.0);

/// Build the iced simulator `Settings` with the same font registrations
/// the production app uses.
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

fn snapshot_to(app: &Resonance, count: usize, path: &str) {
    // Wrap the bar in a container that provides the Arrange-view-like
    // surface it expects (it uses `Length::Fill` to centre itself).
    let element = container(app.test_selection_bar_view(count))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::base_bg);

    let mut ui = Simulator::with_size(
        sim_settings(),
        Size::new(CANVAS.0, CANVAS.1),
        element,
    );
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

/// 2 tracks selected — the minimum threshold for showing the bar.
/// Pill should read "2 tracks selected · … — Group ⌘G".
#[test]
fn selection_bar_two_tracks() {
    let (app, _task) = Resonance::new_for_test();
    snapshot_to(&app, 2, "tests/snapshots/selection_bar_two_tracks.png");
}

/// 5 tracks selected — tests the N-track plural state.
/// Pill should read "5 tracks selected · … — Group ⌘G".
#[test]
fn selection_bar_many_tracks() {
    let (app, _task) = Resonance::new_for_test();
    snapshot_to(&app, 5, "tests/snapshots/selection_bar_many_tracks.png");
}
