//! Golden-image + wiring coverage for the freeze status banners (design doc
//! #181, ba todo #583).
//!
//! A frozen track surfaces one of two non-blocking banners in its Arrange
//! header when it leaves the happy path:
//!
//! - **Stale / refreeze**: an amber (`WARM`) "Frozen audio is out of date"
//!   strip with a **Refreeze** primary (`RefreezeTrack`) and an **Unfreeze to
//!   edit** secondary (`UnfreezeTrack`).
//! - **Freeze failed**: a soft-pink (`BAD`) "Freeze failed — <reason>" strip
//!   with **Retry** (`FreezeTrack`) and **Dismiss** (`UnfreezeTrack`). The
//!   track has already fallen back to live, so no work is lost.
//!
//! The standalone goldens pin each banner's tint / glyph / copy / actions so
//! any drift trips the diff (first run writes the golden under
//! `tests/snapshots/`). The behavioural tests then drive the exact messages
//! the banner buttons dispatch through the real update handlers.

use crate::common;

use iced::widget::container;
use iced::{Length, Size};
use iced_test::simulator::Simulator;
use resonance_app::message::{FreezeMessage, Message};
use resonance_app::state::FreezeStatus;
use resonance_app::theme;
use resonance_app::view::freeze_banner::{failed_banner, freeze_banner, stale_banner};
use resonance_app::Resonance;
use resonance_audio::types::TrackType;
use resonance_common::{FreezeCacheRef, FreezeCacheStatus};

const TRACK: u64 = 7;

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

/// Wrap a banner on a `BG_1` card at a realistic track-header width so the
/// golden frames the strip exactly as the Arrange header overlays it.
fn staged(banner: iced::Element<'static, Message>) -> iced::Element<'static, Message> {
    container(container(banner).width(Length::Fixed(360.0)))
        .padding(14)
        .style(|_: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            ..Default::default()
        })
        .into()
}

fn stale_cache() -> FreezeCacheRef {
    FreezeCacheRef::new(
        "freeze_7.wav".to_string(),
        48_000,
        32,
        0,
        FreezeCacheStatus::Stale,
    )
}

// ---------------------------------------------------------------------
// Golden snapshots
// ---------------------------------------------------------------------

#[test]
fn stale_refreeze_banner_render() {
    let view = staged(stale_banner(TRACK));
    let mut ui = Simulator::with_size(sim_settings(), Size::new(400.0, 56.0), view);
    ui.find("Frozen audio is out of date")
        .expect("stale banner shows the out-of-date message");
    ui.find("Refreeze").expect("stale banner has a Refreeze action");
    ui.find("Unfreeze to edit")
        .expect("stale banner has an Unfreeze-to-edit action");
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, "tests/snapshots/stale_refreeze_banner.png");
}

#[test]
fn freeze_failed_banner_render() {
    let view = staged(failed_banner(TRACK, "not enough disk space"));
    let mut ui = Simulator::with_size(sim_settings(), Size::new(400.0, 56.0), view);
    ui.find("Freeze failed — not enough disk space")
        .expect("failed banner shows the reason");
    ui.find("Retry").expect("failed banner has a Retry action");
    ui.find("Dismiss").expect("failed banner has a Dismiss action");
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, "tests/snapshots/freeze_failed_banner.png");
}

// ---------------------------------------------------------------------
// Dispatcher gating
// ---------------------------------------------------------------------

#[test]
fn banner_only_for_stale_and_failed() {
    assert!(
        freeze_banner(&FreezeStatus::Idle, TRACK).is_none(),
        "idle tracks show no banner"
    );
    assert!(
        freeze_banner(&FreezeStatus::Freezing { fraction: 0.3 }, TRACK).is_none(),
        "an in-flight freeze shows the progress modal, not a banner"
    );
    assert!(
        freeze_banner(
            &FreezeStatus::Frozen {
                cache_ref: stale_cache()
            },
            TRACK
        )
        .is_none(),
        "a healthy frozen track shows no banner"
    );
    assert!(
        freeze_banner(
            &FreezeStatus::Stale {
                cache_ref: stale_cache()
            },
            TRACK
        )
        .is_some(),
        "a stale track shows the refreeze banner"
    );
    assert!(
        freeze_banner(
            &FreezeStatus::Failed {
                message: "disk full".to_string()
            },
            TRACK
        )
        .is_some(),
        "a failed freeze shows the freeze-failed banner"
    );
}

// ---------------------------------------------------------------------
// Banner action wiring (the messages the buttons dispatch)
// ---------------------------------------------------------------------

/// "Unfreeze to edit" on a stale track drops the cache and returns to live
/// editing (`Idle`).
#[test]
fn unfreeze_stale_returns_to_idle() {
    let (mut app, _task) = Resonance::new_for_test();
    let rx = app.test_capture_engine();
    let dir = tempfile::tempdir().expect("temp project dir");
    app.test_set_project_path(dir.path().join("project.rproj"));
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_set_freeze_status(
        TRACK,
        FreezeStatus::Stale {
            cache_ref: stale_cache(),
        },
    );

    app.test_dispatch(Message::Freeze(FreezeMessage::UnfreezeTrack(TRACK)));
    while rx.try_recv().is_ok() {}

    assert_eq!(app.test_freeze_status(TRACK), FreezeStatus::Idle);
}

/// "Dismiss" on a failed freeze clears the failed status (the track already
/// fell back to live, so it just acknowledges the notice). Wired through
/// `UnfreezeTrack`.
#[test]
fn dismiss_failed_clears_status() {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_set_freeze_status(
        TRACK,
        FreezeStatus::Failed {
            message: "not enough disk space".to_string(),
        },
    );

    app.test_dispatch(Message::Freeze(FreezeMessage::UnfreezeTrack(TRACK)));

    assert_eq!(app.test_freeze_status(TRACK), FreezeStatus::Idle);
}

/// "Refreeze" on a stale track kicks off a fresh render in place, moving the
/// track to `Freezing`.
#[test]
fn refreeze_stale_starts_render() {
    let (mut app, _task) = Resonance::new_for_test();
    let _rx = app.test_capture_engine();
    let dir = tempfile::tempdir().expect("temp project dir");
    app.test_set_project_path(dir.path().join("project.rproj"));
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_set_freeze_status(
        TRACK,
        FreezeStatus::Stale {
            cache_ref: stale_cache(),
        },
    );

    app.test_dispatch(Message::Freeze(FreezeMessage::RefreezeTrack(TRACK)));

    assert_eq!(
        app.test_freeze_status(TRACK),
        FreezeStatus::Freezing { fraction: 0.0 },
        "refreeze should start a new offline render"
    );
}
