//! Unit tests for the shared arrange-row layout model
//! (`view::arrange_layout`, epic #36, doc #203, todo #729).
//!
//! The builder is a pure function of the sorted arrange tracks + the
//! [`TrackGroupRegistry`]; these tests exercise it directly (no iced, no
//! `Resonance`) across: a flat list (no groups), an expanded group with
//! members, a collapsed group (members omitted, header kept), nested
//! sub-group ordering, and cumulative-Y correctness across the mixed
//! 60/96 px row pitches.

use resonance_app::state::{TrackGroupRegistry, TrackState};
use resonance_app::theme::{GROUP_HEADER_HEIGHT, TRACK_HEIGHT};
use resonance_app::view::arrange_layout::{ArrangeRowKind, ArrangeRowLayout};
use resonance_common::automation::TrackId;
use resonance_common::group_identity::GroupIdentityColor;

/// Build a track with the given id at the given arrange order.
fn track(id: TrackId, order: usize) -> TrackState {
    TrackState::new_instrument(id, order)
}

/// Borrow a slice of owned tracks as the `&[&TrackState]` the builder
/// expects (already in arrange order).
fn refs(tracks: &[TrackState]) -> Vec<&TrackState> {
    tracks.iter().collect()
}

#[test]
fn flat_layout_no_groups() {
    let tracks = vec![track(1, 0), track(2, 1), track(3, 2)];
    let groups = TrackGroupRegistry::new();

    let layout = ArrangeRowLayout::build(&refs(&tracks), &groups);

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

    let layout = ArrangeRowLayout::build(&refs(&tracks), &groups);

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

    let layout = ArrangeRowLayout::build(&refs(&tracks), &groups);

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

    let layout = ArrangeRowLayout::build(&refs(&tracks), &groups);

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
    // A collapsed group, an expanded group, and an ungrouped track, so the
    // layout mixes 60 px headers with 96 px lanes and skips hidden members.
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

    let layout = ArrangeRowLayout::build(&refs(&tracks), &groups);

    // Rows are gapless and monotonically increasing: each y_top equals the
    // previous row's y_bottom, and the last y_bottom equals total_height.
    let mut expected_top = 0.0_f32;
    for row in layout.rows() {
        assert_eq!(row.y_top, expected_top, "rows must be gapless");
        let pitch = match row.kind {
            ArrangeRowKind::GroupHeader(_) => GROUP_HEADER_HEIGHT,
            ArrangeRowKind::Track(_) => TRACK_HEIGHT,
        };
        assert_eq!(row.height, pitch);
        expected_top += pitch;
    }
    assert_eq!(layout.total_height(), expected_top);

    // Sanity on the exact sequence: header(10) collapsed, header(20),
    // track(3), track(4).
    let kinds: Vec<ArrangeRowKind> = layout.rows().iter().map(|r| r.kind).collect();
    assert_eq!(
        kinds,
        vec![
            ArrangeRowKind::GroupHeader(10),
            ArrangeRowKind::GroupHeader(20),
            ArrangeRowKind::Track(3),
            ArrangeRowKind::Track(4),
        ]
    );
}

#[test]
fn empty_layout_has_zero_height() {
    let groups = TrackGroupRegistry::new();
    let layout = ArrangeRowLayout::build(&[], &groups);
    assert!(layout.is_empty());
    assert_eq!(layout.total_height(), 0.0);
    assert!(layout.row_at_y(0.0).is_none());
}
