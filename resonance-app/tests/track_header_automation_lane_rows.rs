//! Track-header column mirroring of the automation lane rows (doc #256,
//! todo #1098).
//!
//! The header column iterates the same `ArrangeRowLayout` as the timeline
//! canvas, so every `ArrangeRowKind::AutomationLane` row (todos
//! #1096/#1097) gets a slim parameter-label header cell, and a track with
//! at least one lane gains an expand/collapse caret with a lane count that
//! dispatches `AutomationMessage::ToggleTrackExpanded`. This suite pins:
//!
//! * header/canvas mirroring: the label cells resolve in exactly the
//!   layout's row order (the single ordering source, `track_lanes_sorted`),
//!   and the rendered column contains one cell per lane row — including
//!   the raw-id fallback label of a `DeviceParam` lane;
//! * the caret + count cluster: shown with the lane count on automated
//!   tracks, absent on laneless tracks, and a click round-trips the
//!   expansion through the real message path (expand → collapse);
//! * a golden snapshot of the expanded track's header column beside the
//!   canvas lane rows (row-for-row alignment, layout-driven).

mod common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{AutomationMessage, Message, ViewportMessage};
use resonance_app::state::ViewMode;
use resonance_app::view::arrange_layout::ArrangeRowKind;
use resonance_app::view::timeline::automation::{target_label, track_lanes_sorted};
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};
use resonance_common::automation::TrackId;
use resonance_common::{AutomationTarget, CurveKind};

/// Window size matches the app's default & minimum window per the
/// design guidelines.
const WINDOW: (f32, f32) = (1440.0, 900.0);

/// The demo track that gets three lanes and the caret under test.
const TRACK: TrackId = 2;

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

/// Demo session in the Arrange view: three lanes on `TRACK` (gain, pan,
/// and a `DeviceParam { "cutoff" }` lane whose label falls back to the raw
/// param id) plus a single gain lane on track 1 for the `1 auto` count.
fn build_app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);

    let (mut app, _task) = Resonance::new();
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
    let mut seed = |target: AutomationTarget, values: [f32; 2]| {
        for (i, value) in values.into_iter().enumerate() {
            let _ = app.update(Message::Automation(AutomationMessage::AddBreakpoint {
                target: target.clone(),
                time_frames: i as u64 * sr,
                value,
                curve: CurveKind::Linear,
            }));
        }
    };
    seed(AutomationTarget::TrackGain(TRACK), [0.2, 0.9]);
    seed(AutomationTarget::TrackPan(TRACK), [0.8, 0.3]);
    seed(
        AutomationTarget::DeviceParam {
            track: TRACK,
            param_id: "cutoff".to_string(),
        },
        [0.5, 0.6],
    );
    seed(AutomationTarget::TrackGain(1), [0.4, 0.6]);
    app
}

fn expand(app: &mut Resonance) {
    let _ = app.update(Message::Automation(AutomationMessage::ToggleTrackExpanded(
        TRACK,
    )));
}

/// The labels the header column resolves for `TRACK`'s lane rows, in the
/// order the shared layout emits them (the exact resolution
/// `build_track_headers` performs per `AutomationLane` row).
fn header_labels_in_layout_order(app: &Resonance) -> Vec<String> {
    let empty = std::collections::HashMap::new();
    app.test_arrange_row_layout()
        .rows()
        .iter()
        .filter_map(|row| match row.kind {
            ArrangeRowKind::AutomationLane { track, lane } if track == TRACK => {
                let lane = app
                    .test_automation()
                    .lanes
                    .values()
                    .find(|l| l.id == lane)
                    .expect("layout lane exists in the automation mirror");
                // No device definitions are registered in this test, so
                // the DeviceParam label exercises the raw-id fallback —
                // the same map the column passes when nothing resolves.
                Some(target_label(&lane.target, &empty))
            }
            _ => None,
        })
        .collect()
}

fn simulator(app: &Resonance) -> Simulator<'_, Message> {
    Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view())
}

// ---------------------------------------------------------------------
// Header / canvas mirroring
// ---------------------------------------------------------------------

/// The header cells resolve one label per canvas lane row, in the same
/// order as `track_lanes_sorted` — the single ordering source both
/// surfaces consume — and the rendered column actually contains them.
#[test]
fn header_cells_mirror_the_canvas_lane_rows_in_order_and_count() {
    let mut app = build_app();
    expand(&mut app);

    let labels = header_labels_in_layout_order(&app);
    assert_eq!(
        labels,
        vec!["Volume".to_string(), "Pan".to_string(), "cutoff".to_string()],
        "one cell per lane row, in the shared sorted order (device lane \
         falls back to its raw param id)"
    );

    // The canvas draws its labels as raw geometry (not widgets), so a
    // widget-tree `find` proves the *header column* renders each cell.
    let mut ui = simulator(&app);
    for label in &labels {
        ui.find(label.as_str())
            .unwrap_or_else(|_| panic!("header column shows a '{label}' lane cell"));
    }
}

/// Collapsed tracks contribute no lane rows, and the header column
/// contains no lane cells for them (the 'cutoff' text exists nowhere else
/// in the Arrange view).
#[test]
fn collapsed_track_has_no_lane_cells() {
    let app = build_app();
    assert!(header_labels_in_layout_order(&app).is_empty());
    let mut ui = simulator(&app);
    assert!(
        ui.find("cutoff").is_err(),
        "no lane cell renders while the track is collapsed"
    );
}

// ---------------------------------------------------------------------
// Caret + lane count
// ---------------------------------------------------------------------

/// The caret cluster shows the lane count on automated tracks and is
/// absent on laneless ones.
#[test]
fn lane_count_indicator_matches_the_lane_count() {
    let app = build_app();
    let registry = app.test_registry();
    let track2 = registry.tracks.iter().find(|t| t.id == TRACK).unwrap();
    assert_eq!(track_lanes_sorted(app.test_automation(), track2).len(), 3);

    let mut ui = simulator(&app);
    ui.find("3 auto")
        .expect("track 2's caret shows its 3-lane count");
    ui.find("1 auto")
        .expect("track 1's caret shows its single-lane count");
    assert!(
        ui.find("2 auto").is_err(),
        "no track has two lanes, so no '2 auto' cluster renders"
    );
}

/// Clicking the caret cluster publishes `ToggleTrackExpanded` for the
/// track, and dispatching it round-trips the expansion: lane rows appear
/// in the shared layout, a second click collapses them again.
#[test]
fn caret_click_round_trips_the_expansion() {
    let mut app = build_app();
    assert!(header_labels_in_layout_order(&app).is_empty());

    // Expand via a real click on the rendered caret cluster.
    let mut ui = simulator(&app);
    ui.click("3 auto").expect("caret cluster is clickable");
    let toggle = ui
        .into_messages()
        .find(|m| {
            matches!(
                m,
                Message::Automation(AutomationMessage::ToggleTrackExpanded(id)) if *id == TRACK
            )
        })
        .expect("caret click publishes ToggleTrackExpanded for the track");
    app.test_dispatch(toggle);
    assert_eq!(
        header_labels_in_layout_order(&app).len(),
        3,
        "first toggle expands the lane rows"
    );

    // Collapse via a second click (the cluster re-renders with the ▾
    // caret but the same count text).
    let mut ui = simulator(&app);
    ui.click("3 auto").expect("caret cluster still clickable");
    let toggle = ui
        .into_messages()
        .find(|m| {
            matches!(
                m,
                Message::Automation(AutomationMessage::ToggleTrackExpanded(id)) if *id == TRACK
            )
        })
        .expect("second click publishes the toggle again");
    app.test_dispatch(toggle);
    assert!(
        header_labels_in_layout_order(&app).is_empty(),
        "second toggle collapses the lane rows"
    );
}

// ---------------------------------------------------------------------
// Golden snapshot
// ---------------------------------------------------------------------

/// Expanded track 2: the header column shows the three slim lane cells
/// (Volume / Pan / cutoff) row-aligned with the canvas's lane rows, and
/// the track header carries the open caret with "3 auto"; track 1 keeps a
/// closed caret with "1 auto".
#[test]
fn expanded_header_column_snapshot() {
    let mut app = build_app();
    expand(&mut app);
    let mut ui = simulator(&app);
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(
        &snap,
        "tests/snapshots/track_header_automation_lane_rows_expanded.png",
    );
}
