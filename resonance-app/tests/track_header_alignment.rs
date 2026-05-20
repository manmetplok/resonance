//! Golden-image snapshots for the Arrange view's track-header column.
//!
//! The track-header column has to mirror the timeline canvas's vertical
//! layout row-for-row so each header stays glued to its lane during
//! vertical scrolling. Three regressions are locked in here:
//!
//! 1. **scroll = 0** — the section-band placeholder pushes the first
//!    header down to align with its lane (without it every header
//!    drifts up by `SECTION_BAND_HEIGHT`).
//! 2. **scroll = 50** — fractional scroll inside the first row. This is
//!    the case that exposed the snap-to-row bug where the column would
//!    only translate in multiples of `TRACK_HEIGHT`.
//! 3. **scroll = 140** — past row 1 plus a 44 px fractional offset, so
//!    multi-row skipping plus fractional translation both have to
//!    cooperate.
//!
//! Manual virtualization regressions also live here. With 100 tracks
//! seeded into the registry, `track_header_virtualization_*` use
//! `iced_test::Simulator::find` to verify that tracks outside the
//! visible viewport are dropped from the widget tree entirely (not
//! just clipped offscreen). The matching snapshot
//! `track_header_virtualizes_100_tracks_scroll_0` locks in the visual
//! result.
//!
//! Window size is the app's 1440×900 minimum (per `ux-guidelines.md`).
//! On first run `matches_image()` writes the goldens under
//! `tests/snapshots/`; subsequent runs diff against the committed PNGs.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, UiMessage, ViewportMessage};
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};

/// Window size matches the app's default & minimum window per the
/// design guidelines.
const WINDOW: (f32, f32) = (1440.0, 900.0);

/// Build the iced simulator `Settings` so the headless renderer sees
/// the same fonts the production app registers in `main.rs`. Without
/// these, the simulator falls back to a default sans and the goldens
/// stop matching what the user actually sees.
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

/// Build a fully-seeded demo app at the requested vertical scroll
/// offset. Uses the public `ViewportMessage::ScrollToY` path so the
/// real reducer + clamping logic runs — the test doesn't poke
/// `viewport.scroll_offset_y` directly.
fn build_app_scrolled(scroll_y: f32) -> Resonance {
    // STARTUP_TAB is a process-global OnceLock — set it once to Arrange
    // so the first test to construct an app pins the startup view.
    // Subsequent `.set` calls are no-ops, so other tests in this file
    // share the same value (which is what we want).
    let _ = STARTUP_TAB.set(ViewMode::Arrange);

    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);

    // Inform the reducer of the on-screen viewport so `ScrollToY`'s
    // clamping uses realistic bounds. Without this, the content-height
    // clamp can pin the offset to zero on the first scroll.
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportWidth(
        WINDOW.0 - theme::TRACK_HEADER_WIDTH,
    )));
    // The track-header column's bottom-side virtualization
    // (`view::track_header::build_track_headers`) relies on the canvas
    // having reported its rendered height. Without this, the test would
    // exercise the `viewport_height == 0` fallback path that disables
    // virtualization entirely.
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportHeight(
        WINDOW.1,
    )));
    let _ = app.update(Message::Viewport(ViewportMessage::TimelineContentSize(
        2000.0,
        // Plenty of vertical headroom so the clamp can't pin us back.
        WINDOW.1 * 4.0,
    )));

    if scroll_y > 0.0 {
        let _ = app.update(Message::Viewport(ViewportMessage::ScrollToY(scroll_y)));
    }
    app
}

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image(path).expect("matches_image i/o"),
        "snapshot diverged from golden: {path}"
    );
}

#[test]
fn track_header_alignment_scroll_0() {
    let app = build_app_scrolled(0.0);
    snapshot_to(&app, "tests/snapshots/track_header_alignment_scroll_0.png");
}

#[test]
fn track_header_alignment_scroll_50() {
    let app = build_app_scrolled(50.0);
    snapshot_to(
        &app,
        "tests/snapshots/track_header_alignment_scroll_50.png",
    );
}

#[test]
fn track_header_alignment_scroll_140() {
    let app = build_app_scrolled(140.0);
    snapshot_to(
        &app,
        "tests/snapshots/track_header_alignment_scroll_140.png",
    );
}

/// Regression for the "tracks bleed into the canvas header" bug: with
/// the global tracks area expanded *and* a partial-row vertical scroll,
/// the partially-scrolled top track used to paint its background and
/// clip body over the ruler + section band + global tracks area. The
/// lane-area clip in `TimelineCanvas::draw_into` confines those paints
/// to below `fixed_header_height()`. A golden taken with this state
/// holds the fix in place.
#[test]
fn timeline_lane_clip_globals_expanded_scrolled() {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportWidth(
        WINDOW.0 - theme::TRACK_HEADER_WIDTH,
    )));
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportHeight(
        WINDOW.1,
    )));
    let _ = app.update(Message::Viewport(ViewportMessage::TimelineContentSize(
        2000.0,
        WINDOW.1 * 4.0,
    )));
    let _ = app.update(Message::Ui(UiMessage::ToggleGlobalTracks));
    let _ = app.update(Message::Viewport(ViewportMessage::ScrollToY(50.0)));

    snapshot_to(
        &app,
        "tests/snapshots/timeline_lane_clip_globals_expanded_scrolled.png",
    );
}

/// Regression for the **column-side** bleed: the track-header column
/// is a separate widget tree from `TimelineCanvas` and its lane area
/// uses negative-padding fractional scroll. In iced 0.14,
/// `container.clip(true)` only narrows the viewport passed to children's
/// `draw()`; it does not clip child `fill_quad` background paints, so
/// negatively-offset track-header backgrounds would render over the
/// ruler / section band / global-shelf strip above. The fix layers the
/// chrome on top of the lane area via `stack![lane_subtree, chrome]`
/// inside `build_track_headers`, so the opaque chrome backgrounds mask
/// any upward overflow. This golden captures the worst case (global
/// shelf expanded + scrolled into a partial top row, where the first
/// track's full button row used to render inside the Signature lane).
#[test]
fn track_header_no_bleed_into_chrome_expanded() {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportWidth(
        WINDOW.0 - theme::TRACK_HEADER_WIDTH,
    )));
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportHeight(
        WINDOW.1,
    )));
    let _ = app.update(Message::Viewport(ViewportMessage::TimelineContentSize(
        2000.0,
        WINDOW.1 * 4.0,
    )));
    let _ = app.update(Message::Ui(UiMessage::ToggleGlobalTracks));
    // Picks a partial-row offset (4 px into row 1) so the first
    // visible-tracks slice carries a meaningful negative top padding —
    // historically the case where the bleed was most obvious.
    let _ = app.update(Message::Viewport(ViewportMessage::ScrollToY(100.0)));

    snapshot_to(
        &app,
        "tests/snapshots/track_header_no_bleed_into_chrome_expanded.png",
    );
}

/// Build a 100-track Arrange view scrolled to the top with no global
/// shelf expansion. With the manual virtualization in place, only the
/// rows that intersect the 900 px viewport's lane area should appear
/// in the widget tree — far fewer than 100. The golden locks in that
/// the column visually shows the expected window of tracks (rows 1..9
/// or so at the default chrome height) and nothing else.
#[test]
fn track_header_virtualizes_100_tracks_scroll_0() {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    demo::seed_many_synth_tracks(&mut app, 100);
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportWidth(
        WINDOW.0 - theme::TRACK_HEADER_WIDTH,
    )));
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportHeight(
        WINDOW.1,
    )));
    let _ = app.update(Message::Viewport(ViewportMessage::TimelineContentSize(
        2000.0,
        100.0 * theme::TRACK_HEIGHT + 200.0,
    )));

    snapshot_to(
        &app,
        "tests/snapshots/track_header_virtualizes_100_tracks_scroll_0.png",
    );
}

/// Structural proof that tracks below the viewport are dropped from
/// the widget tree entirely (not just visually clipped). With a 900 px
/// window the lane area fits roughly 9 tracks at `TRACK_HEIGHT = 96`,
/// plus one overscan row, so `VirtTrack 1` must be findable but
/// `VirtTrack 50` and `VirtTrack 100` must not. This is the
/// performance regression test that locks in the optimization — a
/// future refactor that re-adds every track to the column would still
/// pass the snapshot (off-screen rows would just be clipped) but fail
/// this test because the off-screen text widgets would re-appear in
/// the widget tree.
#[test]
fn track_header_virtualization_drops_offscreen_tracks() {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    demo::seed_many_synth_tracks(&mut app, 100);
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportWidth(
        WINDOW.0 - theme::TRACK_HEADER_WIDTH,
    )));
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportHeight(
        WINDOW.1,
    )));
    let _ = app.update(Message::Viewport(ViewportMessage::TimelineContentSize(
        2000.0,
        100.0 * theme::TRACK_HEIGHT + 200.0,
    )));

    let mut ui =
        Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());

    // First on-screen row must be in the widget tree.
    ui.find("VirtTrack 1").expect(
        "first track must be in the widget tree when scrolled to the top",
    );

    // The lane area is roughly `WINDOW.1 - chrome_h` ≈ 868 px tall,
    // fitting `ceil(868 / 96) + 1 = 11` tracks. Anything beyond
    // ~track 11 must have been dropped from the column entirely.
    let offscreen_only_names = ["VirtTrack 30", "VirtTrack 60", "VirtTrack 100"];
    for name in offscreen_only_names {
        assert!(
            ui.find(name).is_err(),
            "expected off-screen `{name}` to be virtualised out of the \
             widget tree (manual track-list virtualization in \
             view::track_header::build_track_headers)"
        );
    }
}

/// Scroll the same 100-track session deep into the list and check
/// that the window of tracks rendered moves with the scroll position —
/// tracks well above the new scroll point should drop from the tree,
/// tracks around `scroll / TRACK_HEIGHT` should appear, and tracks
/// past the bottom edge should still be virtualised out.
#[test]
fn track_header_virtualization_window_follows_scroll() {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    demo::seed_many_synth_tracks(&mut app, 100);
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportWidth(
        WINDOW.0 - theme::TRACK_HEADER_WIDTH,
    )));
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportHeight(
        WINDOW.1,
    )));
    let _ = app.update(Message::Viewport(ViewportMessage::TimelineContentSize(
        2000.0,
        100.0 * theme::TRACK_HEIGHT + 200.0,
    )));
    // Scroll past the first 30 rows so the column should now be
    // showing roughly tracks 30..42.
    let _ = app.update(Message::Viewport(ViewportMessage::ScrollToY(
        30.0 * theme::TRACK_HEIGHT,
    )));

    let mut ui =
        Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());

    ui.find("VirtTrack 31").expect(
        "track at the top of a scrolled-down viewport must be in the tree",
    );

    // Above the scroll point: dropped by top-skip.
    assert!(
        ui.find("VirtTrack 1").is_err(),
        "top track must be virtualised out when scrolled past 30 rows"
    );

    // Far below the scrolled-into viewport: dropped by the new
    // bottom-skip we just added.
    assert!(
        ui.find("VirtTrack 90").is_err(),
        "track 90 must be virtualised out when scrolled to row 30"
    );
}
