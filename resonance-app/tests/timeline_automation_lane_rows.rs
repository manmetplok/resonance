//! Dedicated automation lane rows on the timeline canvas (doc #256, todo
//! #1097).
//!
//! When a track's automation is expanded (`AutomationMessage::
//! ToggleTrackExpanded`), each of its lanes renders as a full editable band
//! inside its own 44 px `ArrangeRowKind::AutomationLane` sub-row, and the
//! in-track overlay band (todo #381/#1095) is suppressed for that track.
//! This suite pins:
//!
//! * golden snapshots of an expanded 3-lane track (gain + pan + a
//!   `DeviceParam` lane) and of the restored overlay after collapse;
//! * overlay suppression: while expanded, no band gesture resolves on the
//!   track's own row — and collapse restores the pre-expansion behaviour;
//! * per-row gesture routing through the real `canvas::Program::update`
//!   input path + the real `Resonance::update` reducers: click-add, drag,
//!   double-click curve toggle and right-click delete all land on a
//!   NON-primary lane (pan) via its dedicated row;
//! * the canvas cache fingerprint changes on every expansion toggle (and
//!   returns to the original when toggled back), so the cached geometry
//!   repaints instead of going stale under the reshaped row layout;
//! * collapsed-group interplay: an expanded member's lane rows vanish from
//!   the layout together with its track row.

mod common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{AutomationMessage, Message, ViewportMessage};
use resonance_app::state::ViewMode;
use resonance_app::view::arrange_layout::ArrangeRowKind;
use resonance_app::view::timeline::automation::{lane_row_band, value_from_y, value_to_y};
use resonance_app::view::timeline::TimelineState;
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};
use resonance_common::automation::TrackId;
use resonance_common::{AutomationTarget, CurveKind};

/// Window size matches the app's default & minimum window per the
/// design guidelines.
const WINDOW: (f32, f32) = (1440.0, 900.0);

/// The demo track that gets three lanes and the expansion toggle.
const TRACK: TrackId = 2;

fn gain() -> AutomationTarget {
    AutomationTarget::TrackGain(TRACK)
}

fn pan() -> AutomationTarget {
    AutomationTarget::TrackPan(TRACK)
}

fn cutoff() -> AutomationTarget {
    AutomationTarget::DeviceParam {
        track: TRACK,
        param_id: "cutoff".to_string(),
    }
}

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

/// Demo session in the Arrange view with three lanes on `TRACK` (gain,
/// pan, and a `DeviceParam { "cutoff" }` lane — the device tier renders
/// its raw param id when no device definition resolves, which is exactly
/// the fallback the label path promises).
fn build_app() -> Resonance {
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
    seed(&mut app, gain(), [0.2, 0.9]);
    seed(&mut app, pan(), [0.8, 0.3]);
    seed(&mut app, cutoff(), [0.5, 0.6]);
    app
}

fn expand(app: &mut Resonance) {
    let _ = app.update(Message::Automation(AutomationMessage::ToggleTrackExpanded(
        TRACK,
    )));
}

/// Canvas-space `(band_top, band_height)` of `TRACK`'s dedicated sub-row
/// for `target`'s lane, resolved through the same shared layout the canvas
/// draws from.
fn lane_row_band_of(app: &Resonance, target: &AutomationTarget) -> (f32, f32) {
    let lane_id = app.test_automation().lanes.get(target).expect("lane").id;
    let layout = app.test_arrange_row_layout();
    let (y_top, height) = layout
        .automation_row_rect(TRACK, lane_id)
        .expect("expanded lane row in layout");
    lane_row_band(app.test_arrange_header_offset() + y_top, height)
}

/// Canvas-space y of the *overlay* band centre inside `TRACK`'s own track
/// row (the pre-#1097 in-track band position).
fn overlay_band_center_y(app: &Resonance) -> f32 {
    let layout = app.test_arrange_row_layout();
    let (y_top, height) = layout.track_row_rect(TRACK).expect("track row");
    app.test_arrange_header_offset() + y_top + height / 2.0
}

fn left_press() -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left))
}

fn right_press() -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::ButtonPressed(
        iced::mouse::Button::Right,
    ))
}

fn cursor_moved() -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::CursorMoved {
        position: iced::Point::ORIGIN,
    })
}

fn left_release() -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
        iced::mouse::Button::Left,
    ))
}

// ---------------------------------------------------------------------
// Overlay suppression
// ---------------------------------------------------------------------

/// While `TRACK` is expanded, no automation gesture resolves inside its
/// own (clip) row — the overlay band is suppressed; collapsing restores
/// the pre-expansion band-add on the shown (gain) lane.
#[test]
fn overlay_band_is_suppressed_while_expanded_and_restored_on_collapse() {
    let mut app = build_app();
    let y = overlay_band_center_y(&app);

    let before = app.test_timeline_band_add_at(300.0, y);
    assert_eq!(
        before.as_ref().map(|(t, _, _)| t.clone()),
        Some(gain()),
        "collapsed: overlay band-add resolves the shown (gain) lane"
    );

    expand(&mut app);
    assert_eq!(
        app.test_timeline_band_add_at(300.0, y),
        None,
        "expanded: the in-track overlay band must not accept gestures"
    );
    // The overlay breakpoint dots are gone too: the gain dot at frame 0 /
    // value 0.2 used to sit in the overlay band's lower half.
    assert_eq!(
        app.test_timeline_breakpoint_hit(0.0, y),
        None,
        "expanded: no overlay dot hits inside the track row"
    );

    expand(&mut app); // collapse again
    let after = app.test_timeline_band_add_at(300.0, y);
    assert_eq!(
        after.as_ref().map(|(t, _, _)| t.clone()),
        Some(gain()),
        "collapse restores the overlay band"
    );
}

// ---------------------------------------------------------------------
// Per-row gesture routing (the key new capability): all four #382
// gestures on the NON-primary pan lane's dedicated row, through the real
// canvas input path + the real update reducers.
// ---------------------------------------------------------------------

/// Click-add on empty band space of the pan row inserts a breakpoint into
/// the PAN lane (not the primary gain lane).
#[test]
fn click_add_routes_to_the_pan_rows_lane() {
    let mut app = build_app();
    expand(&mut app);
    let (band_top, band_height) = lane_row_band_of(&app, &pan());
    let y = value_to_y(0.5, band_top, band_height);

    let mut state = TimelineState::default();
    let msg = app
        .test_timeline_canvas_event(&mut state, &left_press(), 700.0, y)
        .expect("press publishes a message");
    let Message::Automation(AutomationMessage::AddBreakpoint { target, value, .. }) = &msg else {
        panic!("expected AddBreakpoint on the pan lane, got {msg:?}");
    };
    assert_eq!(target, &pan(), "the row's own lane is the edit target");
    assert!((value - 0.5).abs() < 0.05, "value maps from the row's band");

    let points_before = app.test_automation().lanes[&pan()].points.len();
    app.test_dispatch(msg);
    let lane = &app.test_automation().lanes[&pan()];
    assert_eq!(lane.points.len(), points_before + 1);
    // The other lanes are untouched.
    assert_eq!(app.test_automation().lanes[&gain()].points.len(), 2);
    assert_eq!(app.test_automation().lanes[&cutoff()].points.len(), 2);
}

/// Press-drag-release on the pan row's frame-0 dot drags that breakpoint's
/// value; the gain lane's identical-x dot is never touched.
#[test]
fn drag_routes_to_the_pan_rows_breakpoint() {
    let mut app = build_app();
    expand(&mut app);
    let (band_top, band_height) = lane_row_band_of(&app, &pan());
    // Pan point 0: frame 0, value 0.8 → dot at x≈0.
    let y_dot = value_to_y(0.8, band_top, band_height);

    let mut state = TimelineState::default();
    let press = app
        .test_timeline_canvas_event(&mut state, &left_press(), 1.0, y_dot)
        .expect("dot press publishes");
    assert!(
        matches!(
            &press,
            Message::Automation(AutomationMessage::StartBreakpointDrag { target, index: 0 })
                if *target == pan()
        ),
        "expected StartBreakpointDrag on pan[0], got {press:?}"
    );
    app.test_dispatch(press);

    // Drag down to value ≈ 0.25 within the same row band.
    let y_to = value_to_y(0.25, band_top, band_height);
    let drag = app
        .test_timeline_canvas_event(&mut state, &cursor_moved(), 1.0, y_to)
        .expect("move publishes a drag");
    let Message::Automation(AutomationMessage::DragBreakpoint {
        target,
        index,
        value,
        ..
    }) = &drag
    else {
        panic!("expected DragBreakpoint, got {drag:?}");
    };
    assert_eq!((target, index), (&pan(), &0));
    assert!((value - 0.25).abs() < 0.05, "value maps against the pan row band");
    app.test_dispatch(drag);

    let release = app
        .test_timeline_canvas_event(&mut state, &left_release(), 1.0, y_to)
        .expect("release publishes");
    assert!(matches!(
        release,
        Message::Automation(AutomationMessage::EndBreakpointDrag)
    ));
    app.test_dispatch(release);

    let pan_lane = &app.test_automation().lanes[&pan()];
    assert!(
        (pan_lane.points[0].value - 0.25).abs() < 0.05,
        "pan[0] followed the drag, got {}",
        pan_lane.points[0].value
    );
    // The gain lane's frame-0 point (same x, different row) is untouched.
    assert_eq!(app.test_automation().lanes[&gain()].points[0].value, 0.2);
}

/// A quick second press on the same pan-row dot is a double-click: it
/// publishes the curve-kind toggle for that dot.
#[test]
fn double_click_toggles_the_pan_dots_curve() {
    let mut app = build_app();
    expand(&mut app);
    let (band_top, band_height) = lane_row_band_of(&app, &pan());
    let y_dot = value_to_y(0.8, band_top, band_height);

    let mut state = TimelineState::default();
    let first = app
        .test_timeline_canvas_event(&mut state, &left_press(), 1.0, y_dot)
        .expect("first press");
    app.test_dispatch(first);
    let second = app
        .test_timeline_canvas_event(&mut state, &left_press(), 1.0, y_dot)
        .expect("second press");
    let Message::Automation(AutomationMessage::SetCurveKind {
        target,
        index,
        curve,
    }) = &second
    else {
        panic!("expected SetCurveKind, got {second:?}");
    };
    assert_eq!((target, index, curve), (&pan(), &0, &CurveKind::Stepped));

    app.test_dispatch(second);
    assert_eq!(
        app.test_automation().lanes[&pan()].points[0].curve,
        CurveKind::Stepped
    );
}

/// Right-click on the pan row's dot deletes that breakpoint (and only it).
#[test]
fn right_click_deletes_the_pan_rows_breakpoint() {
    let mut app = build_app();
    expand(&mut app);
    let (band_top, band_height) = lane_row_band_of(&app, &pan());
    let y_dot = value_to_y(0.8, band_top, band_height);

    let mut state = TimelineState::default();
    let msg = app
        .test_timeline_canvas_event(&mut state, &right_press(), 1.0, y_dot)
        .expect("right press publishes");
    assert!(
        matches!(
            &msg,
            Message::Automation(AutomationMessage::DeleteBreakpoint { target, index: 0 })
                if *target == pan()
        ),
        "expected DeleteBreakpoint on pan[0], got {msg:?}"
    );

    app.test_dispatch(msg);
    assert_eq!(app.test_automation().lanes[&pan()].points.len(), 1);
    assert_eq!(app.test_automation().lanes[&gain()].points.len(), 2);
}

/// Sanity for the value mapping helpers on the slim lane-row band: the
/// round trip used by the gesture tests holds on the 44 px row geometry.
#[test]
fn lane_row_band_value_round_trip() {
    let (band_top, band_height) = lane_row_band(100.0, theme::AUTOMATION_LANE_ROW_HEIGHT);
    for v in [0.0f32, 0.25, 0.5, 1.0] {
        let y = value_to_y(v, band_top, band_height);
        assert!((value_from_y(y, band_top, band_height) - v).abs() < 1e-6);
        assert!(y >= 100.0 && y <= 100.0 + theme::AUTOMATION_LANE_ROW_HEIGHT);
    }
}

// ---------------------------------------------------------------------
// Cache fingerprint
// ---------------------------------------------------------------------

/// Toggling expansion must change the canvas fingerprint (the cached
/// geometry layer repaints under the reshaped row layout), and toggling
/// back must return to the original fingerprint (no spurious repaint
/// keys).
#[test]
fn fingerprint_changes_on_expand_and_returns_on_collapse() {
    let mut app = build_app();
    let collapsed = app.test_timeline_fingerprint();

    expand(&mut app);
    let expanded = app.test_timeline_fingerprint();
    assert_ne!(
        collapsed, expanded,
        "expanding must invalidate the cached canvas layer"
    );

    expand(&mut app);
    assert_eq!(
        app.test_timeline_fingerprint(),
        collapsed,
        "collapsing back restores the original fingerprint"
    );
}

// ---------------------------------------------------------------------
// Collapsed-group interplay
// ---------------------------------------------------------------------

/// An expanded member of a collapsed group contributes neither its track
/// row nor its lane sub-rows — the rows vanish with the hidden track.
#[test]
fn collapsed_group_hides_the_expanded_members_lane_rows() {
    let mut app = build_app();
    expand(&mut app);

    let group_id: TrackId = 9000;
    app.test_track_groups_mut()
        .create_group_from_selection(group_id, &[2, 3]);

    let lane_rows = |app: &Resonance| {
        app.test_arrange_row_layout()
            .rows()
            .iter()
            .filter(|row| {
                matches!(row.kind, ArrangeRowKind::AutomationLane { track, .. } if track == TRACK)
            })
            .count()
    };
    assert_eq!(lane_rows(&app), 3, "expanded member shows one row per lane");

    app.test_track_groups_mut().set_collapse_state(group_id, true);
    assert_eq!(
        lane_rows(&app),
        0,
        "collapsing the group hides the member's lane rows"
    );
    assert!(
        app.test_arrange_row_layout().track_row_rect(TRACK).is_none(),
        "the member's track row is hidden too"
    );

    app.test_track_groups_mut().set_collapse_state(group_id, false);
    assert_eq!(lane_rows(&app), 3, "unfolding the group restores the rows");
}

// ---------------------------------------------------------------------
// Golden snapshots
// ---------------------------------------------------------------------

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui =
        Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

/// Expanded state: track 2 shows three dedicated 44 px lane rows (Volume,
/// Pan, cutoff — the `DeviceParam` label falls back to its raw param id),
/// each with its own envelope band; the in-track overlay band and its
/// "3 lanes" chip are gone from track 2's clip row.
#[test]
fn expanded_lane_rows_snapshot() {
    let mut app = build_app();
    expand(&mut app);
    snapshot_to(
        &app,
        "tests/snapshots/timeline_automation_lane_rows_expanded.png",
    );
}

/// Collapse round-trip: after expanding and collapsing again the timeline
/// renders the pre-expansion overlay (gain band + chip + "3 lanes" count
/// on the track row, no sub-rows).
#[test]
fn collapsed_after_round_trip_snapshot() {
    let mut app = build_app();
    expand(&mut app);
    expand(&mut app);
    snapshot_to(
        &app,
        "tests/snapshots/timeline_automation_lane_rows_collapsed.png",
    );
}
