//! Golden-image + unit coverage for the frozen-track render (design doc
//! #181, ba todo #580).
//!
//! Freezing a track renders its post-instrument/post-FX output to a cache
//! and plays that back instead of the live chain. The Arrange surface marks
//! that state as a *visual mode* derived from the frost tokens — never a new
//! palette hue:
//!
//! - **Header**: the substrate frosts (`theme::frost_over`), a `FROZEN` pill
//!   sits on the kind line, a "Notes & FX locked" chip marks the read-only
//!   inputs, and the record-arm + input-monitor controls dim to a locked
//!   style. Mute / solo / pan / volume stay live.
//! - **Lane**: the clip switches from the live lavender-MIDI language to the
//!   warm/audio waveform language overlaid with the frost wash, relabelled
//!   "frozen render".
//!
//! The snapshot pins the rendered Arrange view with one frozen track so any
//! drift in either surface trips the golden diff. On first run
//! `matches_image()` writes the golden under `tests/snapshots/`.

mod common;

use iced::{Color, Size};
use iced_test::simulator::Simulator;
use resonance_app::state::{FreezeStatus, ViewMode};
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};
use resonance_common::{FreezeCacheRef, FreezeCacheStatus};

const WINDOW: (f32, f32) = (1440.0, 900.0);

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

/// Demo app pinned to the Arrange tab with the first instrument track
/// (id 1 — "Pattern A") frozen, so both the frosted header cell and the
/// frozen-render lane clip are on screen.
fn build_frozen_arrange_app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new_for_test();
    demo::seed_demo_content(&mut app);
    app.test_set_freeze_status(
        1,
        FreezeStatus::Frozen {
            cache_ref: FreezeCacheRef::new(
                "freeze_1.wav".to_string(),
                48_000,
                32,
                0,
                FreezeCacheStatus::Frozen,
            ),
        },
    );
    app
}

#[test]
fn frozen_track_header_and_lane_render() {
    let app = build_frozen_arrange_app();
    let mut ui =
        Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    // Via the shared helper, not `matches_image` directly, so this test
    // self-skips under RESONANCE_SKIP_GOLDENS=1 like the other 103
    // goldens (ba todo #1064). A direct call here would hard-diff pixels
    // on a non-conformant renderer and wedge the whole scoped-fast gate
    // for every unrelated todo — the wedge #1064 existed to end.
    common::assert_golden(
        &snap,
        "tests/snapshots/frozen_track_header_and_lane_render.png",
    );
}

/// `frost_over` must source-over-composite `FROST_WASH` onto an opaque
/// base: the result is opaque and pulled toward the icy wash, but only by
/// the wash's 0.16 alpha (so the base still dominates). This is the single
/// blend the frosted header / lane substrate relies on, and it is pure
/// arithmetic so it is stable across machines (unlike the golden PNG).
#[test]
fn frost_over_composites_wash_onto_base() {
    let base = theme::BG_1;
    let out = theme::frost_over(base);

    // Fully opaque result.
    assert_eq!(out.a, 1.0, "frosted substrate must stay opaque");

    let a = theme::FROST_WASH.a;
    let expect = |b: f32, w: f32| b * (1.0 - a) + w * a;
    let eps = 1e-6;
    assert!((out.r - expect(base.r, theme::FROST_WASH.r)).abs() < eps);
    assert!((out.g - expect(base.g, theme::FROST_WASH.g)).abs() < eps);
    assert!((out.b - expect(base.b, theme::FROST_WASH.b)).abs() < eps);

    // The wash is icy (more blue than red), so frosting a near-neutral dark
    // base nudges blue up relative to red.
    assert!(
        out.b > base.b && out.b >= out.r,
        "frost should cool the substrate (more blue than red)"
    );

    // A zero-alpha wash would be a no-op; ours must actually move the colour.
    assert!(
        (out.r - base.r).abs() > eps || (out.b - base.b).abs() > eps,
        "frost_over must visibly shift the base colour"
    );
}

/// Frosting and then comparing against a hand-rolled opaque blend of a pure
/// black base isolates the maths from any palette value: black + wash·a == wash·a.
#[test]
fn frost_over_on_black_is_wash_times_alpha() {
    let out = theme::frost_over(Color::BLACK);
    let a = theme::FROST_WASH.a;
    let eps = 1e-6;
    assert!((out.r - theme::FROST_WASH.r * a).abs() < eps);
    assert!((out.g - theme::FROST_WASH.g * a).abs() < eps);
    assert!((out.b - theme::FROST_WASH.b * a).abs() < eps);
    assert_eq!(out.a, 1.0);
}
