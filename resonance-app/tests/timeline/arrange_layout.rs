//! Unit tests for the shared arrange-row layout model
//! (`view::arrange_layout`, epic #36, doc #203, todo #729; automation
//! sub-rows: doc #256, todo #1096).
//!
//! The builder is a pure function of the sorted arrange tracks + the
//! [`TrackGroupRegistry`] + the [`ArrangeAutomationRows`] inputs; these
//! tests exercise it directly (no iced, no `Resonance`) across: a flat
//! list (no groups), an expanded group with members, a collapsed group
//! (members omitted, header kept), nested sub-group ordering,
//! cumulative-Y correctness across the mixed 44/60/96 px row pitches,
//! expanded automation lane rows (order, rects, hiding inside collapsed
//! groups), the nothing-expanded identity guarantee, and the
//! `ArrangeAutomationRows::collect` priority ordering.

use std::collections::{HashMap, HashSet};

use resonance_app::state::{AutomationState, TrackGroupRegistry, TrackState};
use resonance_app::theme::{AUTOMATION_LANE_ROW_HEIGHT, GROUP_HEADER_HEIGHT, TRACK_HEIGHT};
use resonance_app::view::arrange_layout::{
    ArrangeAutomationRows, ArrangeRowKind, ArrangeRowLayout,
};
use resonance_common::automation::{LaneId, TrackId};
use resonance_common::group_identity::GroupIdentityColor;
use resonance_common::{AutomationLane, AutomationTarget};

/// Build a track with the given id at the given arrange order.
fn track(id: TrackId, order: usize) -> TrackState {
    TrackState::new_instrument(id, order)
}

/// Borrow a slice of owned tracks as the `&[&TrackState]` the builder
/// expects (already in arrange order).
fn refs(tracks: &[TrackState]) -> Vec<&TrackState> {
    tracks.iter().collect()
}

/// Shorthand for the no-automation build input (today's behaviour).
fn no_automation() -> ArrangeAutomationRows {
    ArrangeAutomationRows::default()
}

/// Build an [`ArrangeAutomationRows`] directly from per-track lane lists
/// and the expanded set, bypassing `collect` (the builder consumes the
/// lists as given — pre-sorted by the caller).
fn automation_rows(
    lanes_by_track: &[(TrackId, &[LaneId])],
    expanded: &[TrackId],
) -> ArrangeAutomationRows {
    ArrangeAutomationRows {
        lanes_by_track: lanes_by_track
            .iter()
            .map(|(track, lanes)| (*track, lanes.to_vec()))
            .collect::<HashMap<_, _>>(),
        expanded: expanded.iter().copied().collect::<HashSet<_>>(),
    }
}

#[test]
fn flat_layout_no_groups() {
    let tracks = vec![track(1, 0), track(2, 1), track(3, 2)];
    let groups = TrackGroupRegistry::new();

    let layout = ArrangeRowLayout::build(&refs(&tracks), &groups, &no_automation());

    assert_eq!(layout.len(), 3);
    assert_eq!(
        layout.rows()[0].kind,
        ArrangeRowKind::Track(1),
        "first row should be track 1"
    );
    assert_eq!(layout.rows()[1].kind, ArrangeRowKind::Track(2));
    assert_eq!(layout.rows()[2].kind, ArrangeRowKind::Track(3));

    // Uniform 96 px pitch when there are no group headers.
    assert_eq!(layout.rows()[0].y_top, 0.0);
    assert_eq!(layout.rows()[1].y_top, TRACK_HEIGHT);
    assert_eq!(layout.rows()[2].y_top, TRACK_HEIGHT * 2.0);
    assert_eq!(layout.total_height(), TRACK_HEIGHT * 3.0);

    // Every track has a rect; groups have none.
    assert_eq!(layout.track_row_rect(2), Some((TRACK_HEIGHT, TRACK_HEIGHT)));
    assert_eq!(layout.group_row_rect(1), None);
}

#[test]
fn expanded_group_with_members() {
    // Group 10 owns tracks 1 & 2 (expanded); track 3 is ungrouped.
    let tracks = vec![track(1, 0), track(2, 1), track(3, 2)];
    let mut groups = TrackGroupRegistry::new();
    groups.add_group_new(10, "Drums", GroupIdentityColor::Drum);
    groups.add_member(10, 1);
    groups.add_member(10, 2);

    let layout = ArrangeRowLayout::build(&refs(&tracks), &groups, &no_automation());

    // header, member, member, ungrouped track.
    assert_eq!(layout.len(), 4);
    assert_eq!(layout.rows()[0].kind, ArrangeRowKind::GroupHeader(10));
    assert_eq!(layout.rows()[1].kind, ArrangeRowKind::Track(1));
    assert_eq!(layout.rows()[2].kind, ArrangeRowKind::Track(2));
    assert_eq!(layout.rows()[3].kind, ArrangeRowKind::Track(3));

    // Mixed 60 / 96 cumulative pitch.
    assert_eq!(layout.group_row_rect(10), Some((0.0, GROUP_HEADER_HEIGHT)));
    assert_eq!(
        layout.track_row_rect(1),
        Some((GROUP_HEADER_HEIGHT, TRACK_HEIGHT))
    );
    assert_eq!(
        layout.track_row_rect(2),
        Some((GROUP_HEADER_HEIGHT + TRACK_HEIGHT, TRACK_HEIGHT))
    );
    assert_eq!(
        layout.track_row_rect(3),
        Some((GROUP_HEADER_HEIGHT + TRACK_HEIGHT * 2.0, TRACK_HEIGHT))
    );
    assert_eq!(
        layout.total_height(),
        GROUP_HEADER_HEIGHT + TRACK_HEIGHT * 3.0
    );

    // row_at_y maps pointer Y back to the right row.
    assert_eq!(
        layout.row_at_y(GROUP_HEADER_HEIGHT / 2.0).map(|r| r.kind),
        Some(ArrangeRowKind::GroupHeader(10))
    );
    assert_eq!(
        layout
            .row_at_y(GROUP_HEADER_HEIGHT + 1.0)
            .map(|r| r.kind),
        Some(ArrangeRowKind::Track(1))
    );
    // Past the end -> nothing.
    assert!(layout.row_at_y(layout.total_height() + 1.0).is_none());
}

#[test]
fn collapsed_group_omits_members_keeps_header() {
    // Group 10 collapsed (members 1 & 2 hidden); track 3 ungrouped.
    let tracks = vec![track(1, 0), track(2, 1), track(3, 2)];
    let mut groups = TrackGroupRegistry::new();
    groups.add_group_new(10, "Drums", GroupIdentityColor::Drum);
    groups.add_member(10, 1);
    groups.add_member(10, 2);
    assert!(groups.set_collapse_state(10, true));

    let layout = ArrangeRowLayout::build(&refs(&tracks), &groups, &no_automation());

    // Only the header + the ungrouped track survive.
    assert_eq!(layout.len(), 2);
    assert_eq!(layout.rows()[0].kind, ArrangeRowKind::GroupHeader(10));
    assert_eq!(layout.rows()[1].kind, ArrangeRowKind::Track(3));

    // Header still present and re-expandable; members have no rect.
    assert_eq!(layout.group_row_rect(10), Some((0.0, GROUP_HEADER_HEIGHT)));
    assert_eq!(layout.track_row_rect(1), None);
    assert_eq!(layout.track_row_rect(2), None);

    // The ungrouped track sits directly below the 60 px header.
    assert_eq!(
        layout.track_row_rect(3),
        Some((GROUP_HEADER_HEIGHT, TRACK_HEIGHT))
    );
    assert_eq!(layout.total_height(), GROUP_HEADER_HEIGHT + TRACK_HEIGHT);
}

#[test]
fn nested_sub_group_ordering() {
    // Parent group 100 holds track 1 then nested child group 101;
    // child 101 holds tracks 2 & 3.
    let tracks = vec![track(1, 0), track(2, 1), track(3, 2)];
    let mut groups = TrackGroupRegistry::new();
    groups.add_group_new(100, "Parent", GroupIdentityColor::Keys);
    groups.add_group_new(101, "Child", GroupIdentityColor::Vocal);
    groups.add_member(100, 1);
    groups.add_member(100, 101); // nested sub-group as a member of the parent
    groups.add_member(101, 2);
    groups.add_member(101, 3);
    assert!(groups.set_nesting_parent(101, Some(100)));

    let layout = ArrangeRowLayout::build(&refs(&tracks), &groups, &no_automation());

    // Parent header, parent's direct track, child header, child's tracks.
    let kinds: Vec<ArrangeRowKind> = layout.rows().iter().map(|r| r.kind).collect();
    assert_eq!(
        kinds,
        vec![
            ArrangeRowKind::GroupHeader(100),
            ArrangeRowKind::Track(1),
            ArrangeRowKind::GroupHeader(101),
            ArrangeRowKind::Track(2),
            ArrangeRowKind::Track(3),
        ]
    );

    // Cumulative Y across two 60 px headers and three 96 px tracks.
    assert_eq!(layout.group_row_rect(100), Some((0.0, GROUP_HEADER_HEIGHT)));
    assert_eq!(
        layout.track_row_rect(1),
        Some((GROUP_HEADER_HEIGHT, TRACK_HEIGHT))
    );
    assert_eq!(
        layout.group_row_rect(101),
        Some((GROUP_HEADER_HEIGHT + TRACK_HEIGHT, GROUP_HEADER_HEIGHT))
    );
    assert_eq!(
        layout.track_row_rect(2),
        Some((GROUP_HEADER_HEIGHT * 2.0 + TRACK_HEIGHT, TRACK_HEIGHT))
    );
    assert_eq!(
        layout.track_row_rect(3),
        Some((GROUP_HEADER_HEIGHT * 2.0 + TRACK_HEIGHT * 2.0, TRACK_HEIGHT))
    );
    assert_eq!(
        layout.total_height(),
        GROUP_HEADER_HEIGHT * 2.0 + TRACK_HEIGHT * 3.0
    );
}

#[test]
fn cumulative_y_is_gapless_across_mixed_pitches() {
    // A collapsed group, an expanded group, and an ungrouped track with
    // expanded automation, so the layout mixes 60 px headers, 96 px lanes
    // and 44 px automation sub-rows, and skips hidden members.
    let tracks = vec![
        track(1, 0), // member of collapsed group 10
        track(2, 1), // member of collapsed group 10
        track(3, 2), // member of expanded group 20
        track(4, 3), // ungrouped, automation expanded (lanes 40, 41)
    ];
    let mut groups = TrackGroupRegistry::new();
    groups.add_group_new(10, "Folded", GroupIdentityColor::Drum);
    groups.add_member(10, 1);
    groups.add_member(10, 2);
    assert!(groups.set_collapse_state(10, true));
    groups.add_group_new(20, "Open", GroupIdentityColor::Guitar);
    groups.add_member(20, 3);
    let automation = automation_rows(&[(4, &[40, 41])], &[4]);

    let layout = ArrangeRowLayout::build(&refs(&tracks), &groups, &automation);

    // Rows are gapless and monotonically increasing: each y_top equals the
    // previous row's y_bottom, and the last y_bottom equals total_height.
    let mut expected_top = 0.0_f32;
    for row in layout.rows() {
        assert_eq!(row.y_top, expected_top, "rows must be gapless");
        let pitch = match row.kind {
            ArrangeRowKind::GroupHeader(_) => GROUP_HEADER_HEIGHT,
            ArrangeRowKind::Track(_) => TRACK_HEIGHT,
            ArrangeRowKind::AutomationLane { .. } => AUTOMATION_LANE_ROW_HEIGHT,
            ArrangeRowKind::TakeRow { .. } => resonance_app::theme::TAKE_ROW_HEIGHT,
        };
        assert_eq!(row.height, pitch);
        expected_top += pitch;
    }
    assert_eq!(layout.total_height(), expected_top);

    // Sanity on the exact sequence: header(10) collapsed, header(20),
    // track(3), track(4), then track 4's two lane sub-rows.
    let kinds: Vec<ArrangeRowKind> = layout.rows().iter().map(|r| r.kind).collect();
    assert_eq!(
        kinds,
        vec![
            ArrangeRowKind::GroupHeader(10),
            ArrangeRowKind::GroupHeader(20),
            ArrangeRowKind::Track(3),
            ArrangeRowKind::Track(4),
            ArrangeRowKind::AutomationLane { track: 4, lane: 40 },
            ArrangeRowKind::AutomationLane { track: 4, lane: 41 },
        ]
    );
}

#[test]
fn empty_layout_has_zero_height() {
    let groups = TrackGroupRegistry::new();
    let layout = ArrangeRowLayout::build(&[], &groups, &no_automation());
    assert!(layout.is_empty());
    assert_eq!(layout.total_height(), 0.0);
    assert!(layout.row_at_y(0.0).is_none());
}

// -----------------------------------------------------------------------
// Automation lane sub-rows (doc #256, todo #1096)
// -----------------------------------------------------------------------

#[test]
fn expanded_track_emits_lane_rows_in_order() {
    // Flat stack 1, 2, 3. Track 2 is expanded with three lanes (7, 5, 9 —
    // deliberately not id-sorted: the builder must keep the caller's
    // pre-sorted display order, not re-sort). Track 1 has lanes but is NOT
    // expanded; track 3 is expanded but has NO lanes.
    let tracks = vec![track(1, 0), track(2, 1), track(3, 2)];
    let groups = TrackGroupRegistry::new();
    let automation = automation_rows(&[(1, &[11]), (2, &[7, 5, 9])], &[2, 3]);

    let layout = ArrangeRowLayout::build(&refs(&tracks), &groups, &automation);

    // Track(1), Track(2), its three 44 px lane rows, Track(3).
    let kinds: Vec<ArrangeRowKind> = layout.rows().iter().map(|r| r.kind).collect();
    assert_eq!(
        kinds,
        vec![
            ArrangeRowKind::Track(1),
            ArrangeRowKind::Track(2),
            ArrangeRowKind::AutomationLane { track: 2, lane: 7 },
            ArrangeRowKind::AutomationLane { track: 2, lane: 5 },
            ArrangeRowKind::AutomationLane { track: 2, lane: 9 },
            ArrangeRowKind::Track(3),
        ]
    );

    // Each lane row is 44 px, stacked directly beneath the owning track.
    assert_eq!(
        layout.automation_row_rect(2, 7),
        Some((TRACK_HEIGHT * 2.0, AUTOMATION_LANE_ROW_HEIGHT))
    );
    assert_eq!(
        layout.automation_row_rect(2, 5),
        Some((TRACK_HEIGHT * 2.0 + AUTOMATION_LANE_ROW_HEIGHT, AUTOMATION_LANE_ROW_HEIGHT))
    );
    assert_eq!(
        layout.automation_row_rect(2, 9),
        Some((
            TRACK_HEIGHT * 2.0 + AUTOMATION_LANE_ROW_HEIGHT * 2.0,
            AUTOMATION_LANE_ROW_HEIGHT
        ))
    );

    // Rows after the expanded track shift down by the lane stack; total
    // height includes it.
    assert_eq!(
        layout.track_row_rect(3),
        Some((
            TRACK_HEIGHT * 2.0 + AUTOMATION_LANE_ROW_HEIGHT * 3.0,
            TRACK_HEIGHT
        ))
    );
    assert_eq!(
        layout.total_height(),
        TRACK_HEIGHT * 3.0 + AUTOMATION_LANE_ROW_HEIGHT * 3.0
    );

    // row_at_y resolves points inside a lane row to that lane row.
    assert_eq!(
        layout
            .row_at_y(TRACK_HEIGHT * 2.0 + AUTOMATION_LANE_ROW_HEIGHT + 1.0)
            .map(|r| r.kind),
        Some(ArrangeRowKind::AutomationLane { track: 2, lane: 5 })
    );

    // The unexpanded track's lanes produced no rows; the expanded-but-
    // laneless track produced only its own row.
    assert_eq!(layout.automation_row_rect(1, 11), None);
    assert_eq!(layout.track_row_rect(1), Some((0.0, TRACK_HEIGHT)));
}

#[test]
fn collapsed_group_hides_member_lane_rows() {
    // Group 10 owns tracks 1 & 2; track 1 is expanded with one lane.
    let tracks = vec![track(1, 0), track(2, 1), track(3, 2)];
    let mut groups = TrackGroupRegistry::new();
    groups.add_group_new(10, "Drums", GroupIdentityColor::Drum);
    groups.add_member(10, 1);
    groups.add_member(10, 2);
    let automation = automation_rows(&[(1, &[5])], &[1]);

    // Expanded group: the member's lane row sits between the two members.
    let layout = ArrangeRowLayout::build(&refs(&tracks), &groups, &automation);
    let kinds: Vec<ArrangeRowKind> = layout.rows().iter().map(|r| r.kind).collect();
    assert_eq!(
        kinds,
        vec![
            ArrangeRowKind::GroupHeader(10),
            ArrangeRowKind::Track(1),
            ArrangeRowKind::AutomationLane { track: 1, lane: 5 },
            ArrangeRowKind::Track(2),
            ArrangeRowKind::Track(3),
        ]
    );
    assert_eq!(
        layout.automation_row_rect(1, 5),
        Some((GROUP_HEADER_HEIGHT + TRACK_HEIGHT, AUTOMATION_LANE_ROW_HEIGHT))
    );

    // Collapse the group: the hidden member's lane rows vanish with it.
    assert!(groups.set_collapse_state(10, true));
    let layout = ArrangeRowLayout::build(&refs(&tracks), &groups, &automation);
    let kinds: Vec<ArrangeRowKind> = layout.rows().iter().map(|r| r.kind).collect();
    assert_eq!(
        kinds,
        vec![ArrangeRowKind::GroupHeader(10), ArrangeRowKind::Track(3)]
    );
    assert_eq!(layout.automation_row_rect(1, 5), None);
    assert_eq!(layout.total_height(), GROUP_HEADER_HEIGHT + TRACK_HEIGHT);
}

#[test]
fn nothing_expanded_is_identical_to_automation_unaware_layout() {
    // The compatibility bar for todo #1096: over a grouped scene (collapsed
    // group + expanded group + nesting + ungrouped track), a build whose
    // tracks HAVE lanes but where nothing is expanded must be exactly
    // today's layout — i.e. identical to a build with no automation input
    // at all.
    let tracks = vec![
        track(1, 0), // member of collapsed group 10
        track(2, 1), // member of collapsed group 10
        track(3, 2), // member of expanded group 20
        track(4, 3), // ungrouped
    ];
    let mut groups = TrackGroupRegistry::new();
    groups.add_group_new(10, "Folded", GroupIdentityColor::Drum);
    groups.add_member(10, 1);
    groups.add_member(10, 2);
    assert!(groups.set_collapse_state(10, true));
    groups.add_group_new(20, "Open", GroupIdentityColor::Guitar);
    groups.add_member(20, 3);

    // Lanes on every track — but no track expanded.
    let automation = automation_rows(&[(1, &[1]), (2, &[2]), (3, &[3, 4]), (4, &[5])], &[]);

    let with_lanes = ArrangeRowLayout::build(&refs(&tracks), &groups, &automation);
    let without = ArrangeRowLayout::build(&refs(&tracks), &groups, &no_automation());

    assert_eq!(
        with_lanes, without,
        "with nothing expanded the layout must be identical to the automation-unaware one"
    );
    assert!(with_lanes
        .rows()
        .iter()
        .all(|r| !matches!(r.kind, ArrangeRowKind::AutomationLane { .. })));
}

#[test]
fn collect_orders_lanes_by_target_priority() {
    // Track 1 owns five lanes across the priority tiers: gain (0), pan
    // (1), mute (2) and two device params (5, tie broken by param id).
    // A master-gain lane belongs to no arrange track; track 2 has no lanes.
    let tracks = vec![track(1, 0), track(2, 1)];
    let mut automation = AutomationState::default();
    let mut add = |id: LaneId, target: AutomationTarget| {
        automation
            .lanes
            .insert(target.clone(), AutomationLane::new(id, target, Vec::new()));
    };
    add(11, AutomationTarget::TrackPan(1));
    add(10, AutomationTarget::TrackGain(1));
    add(12, AutomationTarget::TrackMute(1));
    add(
        14,
        AutomationTarget::DeviceParam {
            track: 1,
            param_id: "cutoff".to_string(),
        },
    );
    add(
        13,
        AutomationTarget::DeviceParam {
            track: 1,
            param_id: "attack".to_string(),
        },
    );
    add(99, AutomationTarget::MasterGain);

    let expanded: HashSet<TrackId> = [1].into_iter().collect();
    let rows = ArrangeAutomationRows::collect(&automation, &refs(&tracks), &expanded);

    // Gain, pan, mute, then device params ordered by param id ("attack"
    // before "cutoff") — the same deterministic order as
    // `primary_lane_for_track`, so the first row is the primary lane.
    assert_eq!(
        rows.lanes_by_track.get(&1).map(Vec::as_slice),
        Some([10, 11, 12, 13, 14].as_slice())
    );
    // Laneless tracks are absent; the master lane maps to no track.
    assert!(!rows.lanes_by_track.contains_key(&2));
    assert_eq!(rows.lanes_by_track.len(), 1);
    assert_eq!(rows.expanded, expanded);

    // End-to-end through the builder: the collected order drives the rows.
    let groups = TrackGroupRegistry::new();
    let layout = ArrangeRowLayout::build(&refs(&tracks), &groups, &rows);
    let lane_rows: Vec<LaneId> = layout
        .rows()
        .iter()
        .filter_map(|r| match r.kind {
            ArrangeRowKind::AutomationLane { track: 1, lane } => Some(lane),
            _ => None,
        })
        .collect();
    assert_eq!(lane_rows, vec![10, 11, 12, 13, 14]);
}
