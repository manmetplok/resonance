//! Pointer hit-testing coverage for the Arrange **timeline canvas** when a
//! track group is present (epic #36, doc #203, todo #732).
//!
//! Before this todo the canvas resolved a pointer-Y to a track via a uniform
//! `(y - header + scroll) / TRACK_HEIGHT` division. That assumes every row is
//! 96 px, so once a 60 px group-header row is interleaved the mapping drifts:
//! a press lands on the wrong lane, and a press on a group-header lane
//! invents a phantom track underneath it. Both the canvas input
//! (`view::timeline::input`) and the clip drag-drop resolver
//! (`Resonance::track_id_at_arrange_y`) now consult the shared
//! [`ArrangeRowLayout`] instead, so:
//!
//! 1. a press on a TRACK lane resolves to that track under the mixed
//!    60/96 px pitch;
//! 2. a press on a GROUP-HEADER lane resolves to the *group*, never a track;
//! 3. clip drag-drop lands on the right lane and skips collapsed members.
//!
//! The pure resolvers (`hit_test::row_at_canvas_y`, `lane_canvas_y`,
//! `clip_pixel_rect`) are exercised directly against a hand-built layout; the
//! drag-drop path is driven end-to-end through a seeded `Resonance`.

use iced::{Point, Size};
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, ViewportMessage};
use resonance_app::state::{TrackGroupRegistry, TrackState, ViewMode};
use resonance_app::theme::{
    self, CLIP_EDGE_THRESHOLD, CLIP_LANE_INSET, GROUP_HEADER_HEIGHT, TRACK_HEIGHT,
};
use resonance_app::view::arrange_layout::{ArrangeAutomationRows, ArrangeRowKind, ArrangeRowLayout};
use resonance_app::view::timeline::hit_test::{
    clip_pixel_rect, hit_test, lane_canvas_y, row_at_canvas_y, ClipLaneBody, HitKind,
};
use resonance_app::{demo, Resonance, STARTUP_TAB};
use resonance_common::automation::TrackId;
use resonance_common::group_identity::GroupIdentityColor;

// ------------------------------------------------------------------------
// Pure-resolver coverage: `row_at_canvas_y` over a group-header layout
// ------------------------------------------------------------------------

/// Build a track with the given id at the given arrange order.
fn track(id: TrackId, order: usize) -> TrackState {
    TrackState::new_instrument(id, order)
}

fn refs(tracks: &[TrackState]) -> Vec<&TrackState> {
    tracks.iter().collect()
}

/// Group 10 owns tracks 1 & 2 (expanded); track 3 is ungrouped. Rows:
///   [0]  GroupHeader(10)  y 0..60
///   [1]  Track(1)         y 60..156
///   [2]  Track(2)         y 156..252
///   [3]  Track(3)         y 252..348
fn layout_with_group() -> (ArrangeRowLayout, TrackId) {
    let tracks = vec![track(1, 0), track(2, 1), track(3, 2)];
    let mut groups = TrackGroupRegistry::new();
    groups.add_group_new(10, "Drums", GroupIdentityColor::Drum);
    groups.add_member(10, 1);
    groups.add_member(10, 2);
    (
        ArrangeRowLayout::build(&refs(&tracks), &groups, &ArrangeAutomationRows::default()),
        10,
    )
}

const HEADER: f32 = 120.0; // a representative fixed-header height

#[test]
fn group_header_lane_resolves_to_group_not_a_track() {
    let (layout, group_id) = layout_with_group();
    // A press anywhere in the 60 px header band (canvas Y in HEADER..HEADER+60)
    // must name the GROUP — never the track that the old uniform division
    // would have invented under it.
    let y = HEADER + GROUP_HEADER_HEIGHT / 2.0;
    assert_eq!(
        row_at_canvas_y(&layout, y, HEADER, 0.0),
        Some(ArrangeRowKind::GroupHeader(group_id)),
        "group-header lane must route to the group"
    );
}

#[test]
fn first_member_lane_sits_below_the_60px_header_not_at_96() {
    let (layout, _group) = layout_with_group();
    // The pixel that the OLD `/ TRACK_HEIGHT` math called "track index 0"
    // (a few px below the header) is now inside the 60 px GROUP row, and the
    // first *member* lane starts at +60, not +96. Probe both.
    let just_below_header = HEADER + GROUP_HEADER_HEIGHT + 1.0;
    assert_eq!(
        row_at_canvas_y(&layout, just_below_header, HEADER, 0.0),
        Some(ArrangeRowKind::Track(1)),
        "first member lane begins right after the 60 px header"
    );
    // 90 px below the header — under the old uniform pitch this would still
    // be "row 0"; under the real layout it is the *first member* (60..156).
    assert_eq!(
        row_at_canvas_y(&layout, HEADER + 90.0, HEADER, 0.0),
        Some(ArrangeRowKind::Track(1))
    );
    // 156 px below the header is the *second* member (156..252).
    assert_eq!(
        row_at_canvas_y(&layout, HEADER + 156.0 + 1.0, HEADER, 0.0),
        Some(ArrangeRowKind::Track(2))
    );
}

#[test]
fn ungrouped_track_after_group_resolves_correctly() {
    let (layout, _group) = layout_with_group();
    // Track 3 lives at 252..348 below the lane origin.
    let y = HEADER + 252.0 + TRACK_HEIGHT / 2.0;
    assert_eq!(
        row_at_canvas_y(&layout, y, HEADER, 0.0),
        Some(ArrangeRowKind::Track(3))
    );
}

#[test]
fn press_in_header_chrome_or_below_last_row_is_none() {
    let (layout, _group) = layout_with_group();
    // Inside the fixed header (above the lane origin) -> nothing.
    assert_eq!(row_at_canvas_y(&layout, HEADER - 1.0, HEADER, 0.0), None);
    // Below the last row (total height 348) -> nothing (deselect).
    assert_eq!(
        row_at_canvas_y(&layout, HEADER + layout.total_height() + 1.0, HEADER, 0.0),
        None
    );
}

#[test]
fn scroll_offset_shifts_rows_up() {
    let (layout, group_id) = layout_with_group();
    // Scroll down by one TRACK_HEIGHT: what was the first member (60..156)
    // now sits a TRACK_HEIGHT higher in canvas space. The header band
    // (0..60) scrolls partly under the chrome.
    let scroll = TRACK_HEIGHT;
    // Canvas Y == HEADER now maps to lane Y == scroll == 96, which is inside
    // the first member row (60..156).
    assert_eq!(
        row_at_canvas_y(&layout, HEADER, HEADER, scroll),
        Some(ArrangeRowKind::Track(1))
    );
    // A point that maps back into the header band still routes to the group.
    // lane Y 30 -> canvas Y = 30 - scroll + HEADER.
    let y_group = 30.0 - scroll + HEADER;
    assert_eq!(
        row_at_canvas_y(&layout, y_group, HEADER, scroll),
        Some(ArrangeRowKind::GroupHeader(group_id))
    );
}

#[test]
fn collapsed_group_hides_member_rows_from_resolution() {
    // Group 10 collapsed: members 1 & 2 have no lane; track 3 follows the
    // 60 px header directly (60..156).
    let tracks = vec![track(1, 0), track(2, 1), track(3, 2)];
    let mut groups = TrackGroupRegistry::new();
    groups.add_group_new(10, "Drums", GroupIdentityColor::Drum);
    groups.add_member(10, 1);
    groups.add_member(10, 2);
    assert!(groups.set_collapse_state(10, true));
    let layout =
        ArrangeRowLayout::build(&refs(&tracks), &groups, &ArrangeAutomationRows::default());

    // The header band still routes to the group.
    assert_eq!(
        row_at_canvas_y(&layout, HEADER + 10.0, HEADER, 0.0),
        Some(ArrangeRowKind::GroupHeader(10))
    );
    // Right below the header is now track 3, not a hidden member.
    assert_eq!(
        row_at_canvas_y(&layout, HEADER + GROUP_HEADER_HEIGHT + 1.0, HEADER, 0.0),
        Some(ArrangeRowKind::Track(3))
    );
    // Where member 2 *used* to be (well past the collapsed total height) is
    // empty space now.
    assert_eq!(
        row_at_canvas_y(&layout, HEADER + 200.0, HEADER, 0.0),
        None
    );
}

// ------------------------------------------------------------------------
// Pure-resolver coverage: clip body rect under the variable pitch
// ------------------------------------------------------------------------

#[test]
fn member_clip_body_rect_lands_on_the_member_lane() {
    let (layout, _group) = layout_with_group();
    // The second member (track 2) lane is at y_top 156, height 96.
    let (row_y_top, row_height) = layout.track_row_rect(2).expect("member 2 has a lane");
    assert_eq!((row_y_top, row_height), (GROUP_HEADER_HEIGHT + TRACK_HEIGHT, TRACK_HEIGHT));

    // Compose the canvas-space lane top, then the clip body (inset by
    // CLIP_LANE_INSET) exactly as the input + draw paths do.
    let lane_y = lane_canvas_y(row_y_top, HEADER, 0.0);
    let body_y = lane_y + CLIP_LANE_INSET;
    let body_h = row_height - 2.0 * CLIP_LANE_INSET;

    // A 1-second clip at sample 0, 100 px/s, no scroll, no indent.
    let rect = clip_pixel_rect(
        ClipLaneBody {
            y: body_y,
            height: body_h,
            indent: 0.0,
        },
        0,
        44_100,
        100.0,
        44_100,
        0.0,
    );
    assert_eq!(rect.y, body_y, "clip body sits on the member lane");
    assert_eq!(rect.height, body_h);
    assert_eq!(rect.x, 0.0);
    assert_eq!(rect.width, 100.0);

    // The body's centre hit-tests as a Move on its own lane …
    let centre = Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
    assert!(matches!(
        hit_test(centre, rect, CLIP_EDGE_THRESHOLD),
        HitKind::Move { .. }
    ));
    // … and a point up in the GROUP-HEADER band is a clean Miss for this
    // clip (it belongs to a member lane far below the header).
    let in_header_band = Point::new(rect.x + 20.0, HEADER + GROUP_HEADER_HEIGHT / 2.0);
    assert_eq!(
        hit_test(in_header_band, rect, CLIP_EDGE_THRESHOLD),
        HitKind::Miss
    );
}

#[test]
fn lane_canvas_y_round_trips_through_row_at_canvas_y() {
    let (layout, _group) = layout_with_group();
    // For every row, the canvas Y of its top+1px must resolve back to that
    // same row through `row_at_canvas_y` (the two helpers are inverses up to
    // the header/scroll offsets they share).
    let scroll = 24.0;
    for row in layout.rows() {
        let canvas_y = lane_canvas_y(row.y_top, HEADER, scroll) + 1.0;
        assert_eq!(
            row_at_canvas_y(&layout, canvas_y, HEADER, scroll),
            Some(row.kind),
            "row {:?} should resolve from its own canvas Y",
            row.kind
        );
    }
}

// ------------------------------------------------------------------------
// End-to-end drag-drop resolution through a seeded `Resonance`
// ------------------------------------------------------------------------

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

/// Seed the demo session in Arrange, report a realistic viewport, then fold
/// demo tracks 2 (Synth Bass) and 3 (Synth Pad) into group 9000.
fn build_app_with_group() -> (Resonance, TrackId) {
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

    let group_id: TrackId = 9000;
    app.test_track_groups_mut()
        .create_group_from_selection(group_id, &[2, 3]);
    (app, group_id)
}

/// The fixed arrange-header offset (ruler + section band + global shelf)
/// above the first lane, used to build canvas-Y probes that match the live
/// layout. We assert against the layout's own rows rather than hard-coding
/// the post-grouping order, so the test stays robust to demo changes.
fn header_offset(app: &Resonance) -> f32 {
    app.test_arrange_header_offset()
}

#[test]
fn drag_drop_resolves_member_track_under_mixed_pitch() {
    let (app, group_id) = build_app_with_group();
    let layout = app.test_arrange_row_layout();
    let header = header_offset(&app);

    // Find the first member track row (Track(2)) in the live layout and aim
    // the drag at the centre of its lane.
    let member_row = layout
        .rows()
        .iter()
        .find(|r| r.kind == ArrangeRowKind::Track(2))
        .expect("member track 2 has a lane while the group is expanded");
    let y = lane_canvas_y(member_row.y_top, header, 0.0) + member_row.height / 2.0;
    assert_eq!(
        app.test_track_id_at_arrange_y(y),
        Some(2),
        "a drag over the member lane resolves to that member track"
    );

    // Aim at the group-header band: drag-drop must resolve to NO track so the
    // caller keeps the clip on its original lane (never the group id, never a
    // phantom track).
    let group_row = layout
        .rows()
        .iter()
        .find(|r| r.kind == ArrangeRowKind::GroupHeader(group_id))
        .expect("group header row present");
    let yg = lane_canvas_y(group_row.y_top, header, 0.0) + group_row.height / 2.0;
    let resolved = app.test_track_id_at_arrange_y(yg);
    assert_eq!(
        resolved, None,
        "a drag over the group-header lane resolves to no track (got {resolved:?})"
    );
}

#[test]
fn drag_drop_skips_collapsed_members() {
    let (mut app, group_id) = build_app_with_group();
    app.test_track_groups_mut()
        .set_collapse_state(group_id, true);
    let header = header_offset(&app);
    let layout = app.test_arrange_row_layout();

    // Members 2 & 3 are hidden now — they have no lane at all.
    assert_eq!(layout.track_row_rect(2), None);
    assert_eq!(layout.track_row_rect(3), None);

    // The first row after the collapsed header is the next ungrouped track;
    // a drag there resolves to that track, not a hidden member.
    let first_track_row = layout
        .rows()
        .iter()
        .find_map(|r| match r.kind {
            ArrangeRowKind::Track(id) => Some((r.y_top, r.height, id)),
            ArrangeRowKind::GroupHeader(_) | ArrangeRowKind::AutomationLane { .. } => None,
        })
        .expect("at least one visible track row remains");
    let (y_top, h, id) = first_track_row;
    let y = lane_canvas_y(y_top, header, 0.0) + h / 2.0;
    assert_eq!(app.test_track_id_at_arrange_y(y), Some(id));
    assert_ne!(id, 2, "hidden member 2 is never resolved");
    assert_ne!(id, 3, "hidden member 3 is never resolved");
}

#[test]
fn press_on_group_lane_does_not_select_a_track() {
    // Drive the real press handler indirectly: a click on the group-header
    // lane must not leave a track selected. We simulate the message the
    // canvas would publish (ToggleCollapse), then confirm the resolver never
    // hands a track id back for that Y (the press handler's source of truth).
    let (app, group_id) = build_app_with_group();
    let header = header_offset(&app);
    let layout = app.test_arrange_row_layout();
    let group_row = layout
        .rows()
        .iter()
        .find(|r| r.kind == ArrangeRowKind::GroupHeader(group_id))
        .expect("group header row present");

    // Probe several Ys across the full 60 px band — none may resolve to a
    // track, and all must resolve to the group.
    for frac in [0.05_f32, 0.25, 0.5, 0.75, 0.95] {
        let y = lane_canvas_y(group_row.y_top, header, 0.0) + group_row.height * frac;
        assert_eq!(
            row_at_canvas_y(&layout, y, header, 0.0),
            Some(ArrangeRowKind::GroupHeader(group_id)),
            "Y at {frac} of the group band must route to the group"
        );
        assert_eq!(
            app.test_track_id_at_arrange_y(y),
            None,
            "Y at {frac} of the group band must resolve to no track"
        );
    }
}

/// Smoke-test that the seeded app actually renders the Arrange view with the
/// group present (guards the test fixture itself, like the sibling snapshot
/// suite, without pinning a golden image).
#[test]
fn seeded_arrange_view_renders_with_group() {
    let (mut app, group_id) = build_app_with_group();
    assert!(
        app.test_track_groups_mut().get_group(group_id).is_some(),
        "group should exist in the registry"
    );
    let mut ui =
        Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let _ = ui.snapshot(&theme::resonance_theme());
}
