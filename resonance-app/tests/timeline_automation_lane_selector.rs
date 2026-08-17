//! Per-track automation lane selector on the Arrange overlay (todo #1095,
//! arch doc #162 §3).
//!
//! The overlay band draws one lane per track; before #1095 every other lane
//! was invisible with no hint it existed. This suite pins the short-term
//! discoverability fix:
//!
//! * `track_lanes_sorted` — the cycle order: `(target_priority, param-id)`,
//!   device-param lanes included (post-#1094) and tie-broken
//!   lexicographically;
//! * `shown_lane_for_track` — the transient `lane_selection` override beats
//!   the priority default, and a stale selection falls back silently;
//! * `next_lane_id_for_track` / `AutomationMessage::CycleTrackLane` — the
//!   chip click advances through every lane and wraps;
//! * `lane_chip_rect` — the single geometry source shared by the draw pass
//!   and the pointer hit-test (#732 rule);
//! * golden snapshots of a 3-lane track: the chip plate + "3 lanes" count,
//!   and the band switched to the next lane after one cycle. A single-lane
//!   track sits in the same frame rendering exactly as before #1095.

mod common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{AutomationMessage, Message, ViewportMessage};
use resonance_app::state::{AutomationState, TrackState, ViewMode};
use resonance_app::view::timeline::automation::{
    lane_chip_rect, next_lane_id_for_track, shown_lane_for_track, track_lanes_sorted,
};
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};
use resonance_common::{AutomationLane, AutomationTarget, Breakpoint, CurveKind, LaneId, TrackId};

const TRACK: TrackId = 1;
const OTHER_TRACK: TrackId = 2;

fn lane(id: LaneId, target: AutomationTarget) -> AutomationLane {
    AutomationLane::new(
        id,
        target,
        vec![Breakpoint::new(0, 0.5, CurveKind::Linear)],
    )
}

fn device_target(track: TrackId, param_id: &str) -> AutomationTarget {
    AutomationTarget::DeviceParam {
        track,
        param_id: param_id.to_string(),
    }
}

/// Automation state with four lanes on `TRACK` — gain, pan and two device
/// params — plus a gain lane on `OTHER_TRACK` that must never leak into
/// `TRACK`'s cycle.
fn multi_lane_state() -> AutomationState {
    let mut automation = AutomationState::default();
    for l in [
        lane(11, AutomationTarget::TrackGain(TRACK)),
        lane(12, AutomationTarget::TrackPan(TRACK)),
        lane(13, device_target(TRACK, "resonance")),
        lane(14, device_target(TRACK, "cutoff")),
        lane(21, AutomationTarget::TrackGain(OTHER_TRACK)),
    ] {
        automation.lanes.insert(l.target.clone(), l);
    }
    automation
}

fn track() -> TrackState {
    TrackState::new_instrument(TRACK, 0)
}

// ---------------------------------------------------------------------
// Cycle order
// ---------------------------------------------------------------------

#[test]
fn cycle_order_is_priority_then_param_id_including_device_lanes() {
    let automation = multi_lane_state();
    let ids: Vec<LaneId> = track_lanes_sorted(&automation, &track())
        .iter()
        .map(|l| l.id)
        .collect();
    // Gain (prio 0), Pan (prio 1), then the device tier tie-broken by the
    // lexicographically smaller param id: "cutoff" before "resonance".
    // OTHER_TRACK's lane (id 21) is excluded.
    assert_eq!(ids, vec![11, 12, 14, 13]);
}

// ---------------------------------------------------------------------
// Selection override vs. priority default
// ---------------------------------------------------------------------

#[test]
fn default_shown_lane_is_the_priority_pick() {
    let automation = multi_lane_state();
    assert_eq!(shown_lane_for_track(&automation, &track()).unwrap().id, 11);
}

#[test]
fn selection_override_beats_the_priority_pick() {
    let mut automation = multi_lane_state();
    automation.lane_selection.insert(TRACK, 13);
    assert_eq!(shown_lane_for_track(&automation, &track()).unwrap().id, 13);
}

#[test]
fn stale_selection_falls_back_to_the_default_silently() {
    let mut automation = multi_lane_state();
    // Lane id 99 doesn't exist at all.
    automation.lane_selection.insert(TRACK, 99);
    assert_eq!(shown_lane_for_track(&automation, &track()).unwrap().id, 11);
    // Lane id 21 exists but belongs to OTHER_TRACK — equally stale here.
    automation.lane_selection.insert(TRACK, 21);
    assert_eq!(shown_lane_for_track(&automation, &track()).unwrap().id, 11);
}

#[test]
fn no_lanes_means_no_shown_lane() {
    let automation = AutomationState::default();
    assert!(shown_lane_for_track(&automation, &track()).is_none());
    assert!(next_lane_id_for_track(&automation, &track()).is_none());
}

// ---------------------------------------------------------------------
// Cycling wraps
// ---------------------------------------------------------------------

#[test]
fn cycling_visits_every_lane_and_wraps() {
    let mut automation = multi_lane_state();
    let mut visited = Vec::new();
    for _ in 0..4 {
        let next = next_lane_id_for_track(&automation, &track()).unwrap();
        automation.lane_selection.insert(TRACK, next);
        visited.push(next);
    }
    // From the default (11): 12 → 14 → 13 → wrap back to 11.
    assert_eq!(visited, vec![12, 14, 13, 11]);
}

#[test]
fn cycling_from_a_stale_selection_restarts_from_the_default() {
    let mut automation = multi_lane_state();
    automation.lane_selection.insert(TRACK, 99);
    // Stale ⇒ shown is the default (11) ⇒ next is 12.
    assert_eq!(next_lane_id_for_track(&automation, &track()), Some(12));
}

#[test]
fn cycling_a_single_lane_track_wraps_to_itself() {
    let mut automation = AutomationState::default();
    let l = lane(11, AutomationTarget::TrackGain(TRACK));
    automation.lanes.insert(l.target.clone(), l);
    assert_eq!(next_lane_id_for_track(&automation, &track()), Some(11));
}

// ---------------------------------------------------------------------
// Chip geometry (shared by draw + hit-test)
// ---------------------------------------------------------------------

#[test]
fn chip_rect_covers_the_drawn_label() {
    let band_top = 100.0_f32;
    let rect = lane_chip_rect(band_top, "Volume");
    // The draw pass renders the 9 px label at (rect.x + 2, band_top - 13)
    // and fills this exact rect as the chip plate; the hit-test checks the
    // same rect. Pin that the label's top-left and its approximate extent
    // sit inside.
    assert!(rect.contains(iced::Point::new(rect.x + 2.0, band_top - 13.0)));
    assert!(rect.contains(iced::Point::new(rect.x + 2.0, band_top - 4.0)));
    assert!(
        rect.width >= 2.0 * 2.0 + 6.0 * 5.0,
        "six 9px glyphs must fit; got width {}",
        rect.width
    );
    // Geometry scales with the label so long device-param names stay
    // clickable end to end.
    assert!(lane_chip_rect(band_top, "Filter Cutoff").width > rect.width);
}

// ---------------------------------------------------------------------
// Message round-trip through the update layer
// ---------------------------------------------------------------------

/// App with an active project, one instrument track, and three lanes on it
/// (gain, pan, one device param), seeded through the normal edit path.
fn app_with_three_lanes() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_instrument(TRACK, 0));
    for target in [
        AutomationTarget::TrackGain(TRACK),
        AutomationTarget::TrackPan(TRACK),
        device_target(TRACK, "cutoff"),
    ] {
        app.test_dispatch(Message::Automation(AutomationMessage::AddBreakpoint {
            target,
            time_frames: 0,
            value: 0.5,
            curve: CurveKind::Linear,
        }));
    }
    app
}

/// The target of the lane the overlay currently shows for `TRACK`.
fn shown_target(app: &Resonance) -> AutomationTarget {
    let registry = app.test_registry();
    let track = registry.tracks.iter().find(|t| t.id == TRACK).unwrap();
    shown_lane_for_track(app.test_automation(), track)
        .unwrap()
        .target
        .clone()
}

#[test]
fn cycle_message_advances_and_wraps_and_selection_survives_redraws() {
    let mut app = app_with_three_lanes();
    assert_eq!(shown_target(&app), AutomationTarget::TrackGain(TRACK));

    app.test_dispatch(Message::Automation(AutomationMessage::CycleTrackLane(
        TRACK,
    )));
    assert_eq!(shown_target(&app), AutomationTarget::TrackPan(TRACK));

    app.test_dispatch(Message::Automation(AutomationMessage::CycleTrackLane(
        TRACK,
    )));
    assert_eq!(shown_target(&app), device_target(TRACK, "cutoff"));

    // The selection is plain app state (not canvas-local), so it survives
    // any number of view rebuilds between clicks by construction; two
    // reads in a row see the same lane.
    assert_eq!(shown_target(&app), device_target(TRACK, "cutoff"));

    app.test_dispatch(Message::Automation(AutomationMessage::CycleTrackLane(
        TRACK,
    )));
    assert_eq!(shown_target(&app), AutomationTarget::TrackGain(TRACK));
}

#[test]
fn cycle_message_on_a_laneless_track_is_a_no_op() {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_instrument(TRACK, 0));
    app.test_dispatch(Message::Automation(AutomationMessage::CycleTrackLane(
        TRACK,
    )));
    assert!(app.test_automation().lane_selection.is_empty());
}

#[test]
fn removing_the_selected_lane_falls_back_to_the_default() {
    let mut app = app_with_three_lanes();
    app.test_dispatch(Message::Automation(AutomationMessage::CycleTrackLane(
        TRACK,
    )));
    assert_eq!(shown_target(&app), AutomationTarget::TrackPan(TRACK));

    app.test_dispatch(Message::Automation(AutomationMessage::RemoveLane(
        AutomationTarget::TrackPan(TRACK),
    )));
    // The stale selection is ignored; the band shows the priority default.
    assert_eq!(shown_target(&app), AutomationTarget::TrackGain(TRACK));
}

// ---------------------------------------------------------------------
// Golden snapshots
// ---------------------------------------------------------------------

/// Window size matches the app's default & minimum window per the
/// design guidelines.
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

/// Demo session in the Arrange view with THREE lanes (gain, pan, mute) on
/// demo track 2 (Synth Bass) and a SINGLE gain lane on demo track 1
/// (Drums, the visible row above it) — the multi-lane band must grow the
/// chip plate + "3 lanes" count while the single-lane band renders exactly
/// as before #1095 (plain label, no plate, no count).
fn build_demo_app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);

    let (mut app, _task) = Resonance::new_for_test();
    demo::seed_demo_content(&mut app);

    let _ = app.update(Message::Viewport(ViewportMessage::ViewportWidth(
        WINDOW.0 - theme::TRACK_HEADER_WIDTH,
    )));
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportHeight(WINDOW.1)));
    let _ = app.update(Message::Viewport(ViewportMessage::TimelineContentSize(
        2000.0,
        WINDOW.1 * 4.0,
    )));

    let sr = 48_000u64;
    let seed = |app: &mut Resonance, target: AutomationTarget, values: [f32; 2]| {
        for (i, value) in values.into_iter().enumerate() {
            let _ = app.update(Message::Automation(AutomationMessage::AddBreakpoint {
                target: target.clone(),
                time_frames: i as u64 * sr,
                value,
                curve: CurveKind::Linear,
            }));
        }
    };
    seed(&mut app, AutomationTarget::TrackGain(2), [0.2, 0.9]);
    seed(&mut app, AutomationTarget::TrackPan(2), [0.8, 0.3]);
    seed(&mut app, AutomationTarget::TrackMute(2), [1.0, 0.0]);
    seed(&mut app, AutomationTarget::TrackGain(1), [0.4, 0.6]);
    app
}

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui =
        Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

/// Default state: track 2's band shows the gain lane with the chip plate
/// and the "3 lanes" count; track 1's single-lane band has neither.
#[test]
fn multi_lane_chip_and_count_snapshot() {
    let app = build_demo_app();
    snapshot_to(&app, "tests/snapshots/timeline_automation_lane_selector_multi.png");
}

/// After one chip cycle, track 2's band shows the Pan lane (label and
/// envelope both switch); the count still reads "3 lanes".
#[test]
fn cycled_lane_snapshot() {
    let mut app = build_demo_app();
    let _ = app.update(Message::Automation(AutomationMessage::CycleTrackLane(2)));
    snapshot_to(
        &app,
        "tests/snapshots/timeline_automation_lane_selector_cycled.png",
    );
}
