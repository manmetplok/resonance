//! Golden-image snapshots for the timeline **automation overlay** when a
//! track group is present (epic #36, doc #203, todo #1084).
//!
//! Before #1084 the overlay computed every lane band Y from the uniform
//! `index * TRACK_HEIGHT` pitch, so an interleaved 60 px group-header row
//! shifted every band below it off its track, and members of a collapsed
//! group still drew their envelope over whatever row slid into their old
//! slot. Ported onto the shared [`ArrangeRowLayout`], the overlay now:
//!
//! 1. draws each envelope band inside its track's *actual* lane rect under
//!    the mixed 60/96 px row pitch (a lane on a track below the group sits
//!    60 px lower than the old uniform math placed it);
//! 2. draws nothing for tracks hidden inside a collapsed group
//!    (`track_row_rect` -> `None`), mirroring clip behaviour.
//!
//! Two states are pinned: an **expanded** group (bands on a member track
//! and on an ungrouped track below the group, both aligned to their lanes)
//! and a **collapsed** group (the member's band vanishes with its lane;
//! the ungrouped track's band rides up with the shortened layout).
//!
//! Window size is the app's 1440×900 minimum (per `ux-guidelines.md`).
//! On first run `matches_image()` writes the goldens under
//! `tests/snapshots/`; subsequent runs diff against the committed PNGs.

mod common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{AutomationMessage, Message, ViewportMessage};
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};
use resonance_common::automation::TrackId;
use resonance_common::{AutomationTarget, CurveKind};

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

/// Seed a gain lane on `track` with a Linear and a Stepped segment so both
/// envelope branches render.
fn seed_gain_lane(app: &mut Resonance, track: TrackId) {
    let target = AutomationTarget::TrackGain(track);
    let sr = 48_000u64;
    for (frames, value, curve) in [
        (0u64, 0.2f32, CurveKind::Linear),
        (sr, 0.9, CurveKind::Stepped),
        (sr * 2, 0.5, CurveKind::Linear),
    ] {
        let _ = app.update(Message::Automation(AutomationMessage::AddBreakpoint {
            target: target.clone(),
            time_frames: frames,
            value,
            curve,
        }));
    }
}

/// Seed a demo session in the Arrange view, fold two adjacent demo tracks
/// (Synth Bass = id 2, Synth Pad = id 3) into a group, and add gain
/// automation lanes on a group **member** (track 2) and on an ungrouped
/// track **below** the group (track 4). Returns the app and the group id.
fn build_app_with_grouped_automation() -> (Resonance, TrackId) {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);

    let (mut app, _task) = Resonance::new_for_test();
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

    // One lane inside the group, one below it — the latter pins the
    // 60 px header shift, the former pins collapse-hides-the-band.
    seed_gain_lane(&mut app, 2);
    seed_gain_lane(&mut app, 4);

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

/// Expanded group: both envelope bands align with their tracks' visible
/// lanes — the member's band inside the group, and track 4's band shifted
/// down by the 60 px group-header row (not the uniform-pitch position).
#[test]
fn timeline_automation_group_expanded() {
    let (app, _group) = build_app_with_grouped_automation();
    assert!(
        app.test_automation()
            .lanes
            .contains_key(&AutomationTarget::TrackGain(2)),
        "member lane seeded"
    );
    snapshot_to(&app, "tests/snapshots/timeline_automation_group_expanded.png");
}

/// Collapsed group: track 2's lane row is gone from the layout, so its
/// envelope band draws nothing (mirroring hidden clips); track 4's band
/// rides up with the shortened layout and stays glued to its lane.
#[test]
fn timeline_automation_group_collapsed() {
    let (mut app, group_id) = build_app_with_grouped_automation();
    app.test_track_groups_mut().set_collapse_state(group_id, true);
    snapshot_to(&app, "tests/snapshots/timeline_automation_group_collapsed.png");
}
