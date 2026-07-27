//! Golden-image snapshots for the Arrange **timeline canvas** when a
//! track group is present (epic #36, doc #203, todo #731).
//!
//! Before #731 the canvas derived every lane Y from a uniform
//! `index * TRACK_HEIGHT` pitch and had no notion of group-header rows,
//! so a group folder never appeared on the canvas. With the shared
//! [`ArrangeRowLayout`] wired in, the canvas now:
//!
//! 1. draws a 60 px group-colour **lane band** aligned to the
//!    header-column group row — the faint "spans all members" identity
//!    tint while the group is expanded;
//! 2. places member clips at their correct Y under the variable row
//!    pitch (a 60 px header pushes everything below it down by 60, not
//!    96); and
//! 3. drives the vertical scrollbar from `layout.total_height()`.
//!
//! Two states are pinned: an **expanded** group (header band + indented
//! members visible) and a **collapsed** group (header band only, members
//! hidden so their clips are not drawn — the consolidated-overview strip
//! is a separate follow-up, #733).
//!
//! Window size is the app's 1440×900 minimum (per `ux-guidelines.md`).
//! On first run `matches_image()` writes the goldens under
//! `tests/snapshots/`; subsequent runs diff against the committed PNGs.

mod common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, ViewportMessage};
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};
use resonance_common::automation::TrackId;

/// Window size matches the app's default & minimum window per the
/// design guidelines.
const WINDOW: (f32, f32) = (1440.0, 900.0);

/// Build the iced simulator `Settings` so the headless renderer sees the
/// same fonts the production app registers in `main.rs`.
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

/// Seed a demo session in the Arrange view, then fold two adjacent demo
/// tracks (Synth Bass = id 2, Synth Pad = id 3) into a single group.
/// Returns the app and the new group's id.
fn build_app_with_group() -> (Resonance, TrackId) {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);

    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);

    // Report a realistic viewport so virtualization + scroll clamping
    // behave as they do on screen.
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportWidth(
        WINDOW.0 - theme::TRACK_HEADER_WIDTH,
    )));
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportHeight(WINDOW.1)));
    let _ = app.update(Message::Viewport(ViewportMessage::TimelineContentSize(
        2000.0,
        WINDOW.1 * 4.0,
    )));

    // Fold two adjacent demo tracks into a group directly on the
    // registry — deterministic and independent of the click-selection
    // plumbing. The group id (9000) is well clear of the demo track ids.
    let group_id: TrackId = 9000;
    app.test_track_groups_mut()
        .create_group_from_selection(group_id, &[2, 3]);

    (app, group_id)
}

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui =
        Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

/// Expanded group: the canvas shows a 60 px tinted group-header band
/// followed by its member lanes, with member clips sitting at the
/// correct (variable-pitch) Y below the band.
#[test]
fn timeline_group_lane_expanded() {
    let (app, _group) = build_app_with_group();
    snapshot_to(&app, "tests/snapshots/timeline_group_lane_expanded.png");
}

/// Collapsed group: the member lanes drop out of the layout, so only the
/// group-header band remains and the member clips are not drawn. (The
/// consolidated-overview repaint of the collapsed band is a follow-up,
/// todo #733 — for now it is a plain band.)
///
/// Collapse state is set directly on the registry: the live
/// `GroupMessage::ToggleCollapse` reducer is still a no-op on this branch
/// (the fold-state wiring is owned by todo #686), so the canvas-side
/// rendering is exercised by toggling the persisted flag itself.
#[test]
fn timeline_group_lane_collapsed() {
    let (mut app, group_id) = build_app_with_group();
    app.test_track_groups_mut().set_collapse_state(group_id, true);
    snapshot_to(&app, "tests/snapshots/timeline_group_lane_collapsed.png");
}
