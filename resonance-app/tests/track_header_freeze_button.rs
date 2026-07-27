//! Track-header Freeze toggle mini-button (design doc #181, ba todo #579).
//!
//! A snowflake Freeze toggle leads the track-header M/S/arm/monitor row.
//! It reuses the existing 22×22 mini-control footprint (`size = 12` →
//! `size + 10` cell) and button states; active (frozen) it takes the
//! **frost treatment** — the snowflake tints to `FROST_ICON` exactly as
//! `on-solo` / `on-mute` tint their glyph with their semantic colour.
//! Pressing it toggles Freeze ⇄ Unfreeze, emitting the
//! `FreezeTrack` / `UnfreezeTrack` messages (ba todo #574).
//!
//! Coverage:
//! - **Dispatch** (machine-independent): `freeze_toggle_message` picks
//!   `FreezeTrack` for a live track and `UnfreezeTrack` for a frozen one.
//! - **Presence** (widget tree): the snowflake glyph renders in the Arrange
//!   header for both an idle and a frozen track.
//! - **Golden** (`tests/snapshots`): idle and frozen Arrange headers, so any
//!   drift in the new button's idle / frost-active treatment trips the diff.
//!   On first run `matches_image()` writes the goldens under
//!   `tests/snapshots/`.

mod common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{FreezeMessage, Message};
use resonance_app::state::{FreezeStatus, ViewMode};
use resonance_app::view::controls::freeze_toggle_message;
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};
use resonance_common::{FreezeCacheRef, FreezeCacheStatus};

const WINDOW: (f32, f32) = (1440.0, 900.0);

/// The snowflake glyph the freeze button renders — the Arrange header's
/// only use of it, so finding it proves the button is in the widget tree.
const SNOWFLAKE: &str = "\u{f2dc}";

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

fn frozen_cache_ref() -> FreezeCacheRef {
    FreezeCacheRef::new(
        "freeze_1.wav".to_string(),
        48_000,
        32,
        0,
        FreezeCacheStatus::Frozen,
    )
}

/// Demo app pinned to the Arrange tab. When `freeze_first` is set, track 1
/// ("Pattern A") is marked frozen so its header takes the frost treatment.
fn build_arrange_app(freeze_first: bool) -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);
    if freeze_first {
        app.test_set_freeze_status(
            1,
            FreezeStatus::Frozen {
                cache_ref: frozen_cache_ref(),
            },
        );
    }
    app
}

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

// ---------------------------------------------------------------------
// Dispatch — which message the button emits (machine-independent).
// ---------------------------------------------------------------------

#[test]
fn live_track_button_freezes() {
    assert!(
        matches!(
            freeze_toggle_message(false, 7),
            Message::Freeze(FreezeMessage::FreezeTrack(7))
        ),
        "an idle (live) track's freeze button must emit FreezeTrack"
    );
}

#[test]
fn frozen_track_button_unfreezes() {
    assert!(
        matches!(
            freeze_toggle_message(true, 7),
            Message::Freeze(FreezeMessage::UnfreezeTrack(7))
        ),
        "a frozen track's freeze button must emit UnfreezeTrack"
    );
}

// ---------------------------------------------------------------------
// Presence — the snowflake renders in the header for both states.
// ---------------------------------------------------------------------

#[test]
fn snowflake_renders_in_idle_header() {
    let app = build_arrange_app(false);
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    ui.find(SNOWFLAKE)
        .expect("idle track header must render the freeze snowflake button");
}

#[test]
fn snowflake_renders_in_frozen_header() {
    let app = build_arrange_app(true);
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    ui.find(SNOWFLAKE)
        .expect("frozen track header must render the freeze snowflake button");
}

// ---------------------------------------------------------------------
// Golden — idle and frozen header rows.
// ---------------------------------------------------------------------

#[test]
fn freeze_button_idle_header_golden() {
    let app = build_arrange_app(false);
    snapshot_to(
        &app,
        "tests/snapshots/track_header_freeze_button_idle.png",
    );
}

#[test]
fn freeze_button_frozen_header_golden() {
    let app = build_arrange_app(true);
    snapshot_to(
        &app,
        "tests/snapshots/track_header_freeze_button_frozen.png",
    );
}
