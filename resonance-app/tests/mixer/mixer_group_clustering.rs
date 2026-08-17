//! Mixer **group-clustering** order suite (epic #36, doc #200 — "Mixer
//! reflection", ba todo #691).
//!
//! The mixer reflects track groups as coloured clusters: a group folds its
//! member strips behind one leading group-header strip, dropped into the
//! track-strip lane at the position of the group's first member, with the
//! unrelated tracks keeping their order around it. This mirrors the
//! existing parent + sub-track clustering.
//!
//! These tests pin the **displayed order** — the structural guarantee —
//! without parsing the rendered widget tree: `view_mixer` and the test
//! share one ordering pass (`Resonance::mixer_top_level_items`, surfaced as
//! `test_mixer_top_level`), so a future refactor of the view can't silently
//! re-introduce a "members scattered across the lane" regression. The
//! pixel-level treatment (identity rail + wash + macro controls) is owned
//! by the snapshot tests / the e2e-tester.

use resonance_app::message::{GroupMessage, Message, UiMessage};
use resonance_app::state::{TrackState, ViewMode};
use resonance_app::Resonance;
use resonance_common::group_identity::GroupIdentityColor;

/// Fresh app in Mixer view seeded with `n` instrument tracks, ids `1..=n`,
/// in `.order` 0..n (so `sorted_tracks` yields them in id order).
fn app_with_tracks(n: u64) -> Resonance {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    for id in 1..=n {
        app.test_push_track(TrackState::new_instrument(id, (id - 1) as usize));
    }
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    app
}

/// Group the given members via the real reducer path (selection + "Group
/// selected") and return the freshly-allocated group id.
fn group_tracks(app: &mut Resonance, members: &[u64]) -> u64 {
    let before: std::collections::HashSet<u64> = app
        .test_track_groups()
        .get_all_groups_sorted()
        .iter()
        .map(|g| g.id)
        .collect();
    app.test_set_selected_tracks(members.to_vec());
    let _ = app.update(Message::Group(GroupMessage::CreateGroupFromSelection));
    app.test_track_groups()
        .get_all_groups_sorted()
        .iter()
        .map(|g| g.id)
        .find(|id| !before.contains(id))
        .expect("CreateGroupFromSelection should add a group")
}

#[test]
fn ungrouped_tracks_keep_sorted_order() {
    let app = app_with_tracks(4);
    assert_eq!(
        app.test_mixer_top_level(),
        vec![(false, 1), (false, 2), (false, 3), (false, 4)],
        "with no groups the mixer lists every top-level track in sorted order"
    );
}

#[test]
fn group_clusters_at_first_member_and_consumes_members() {
    let mut app = app_with_tracks(4);
    // Group the two middle tracks. The cluster must land where track 2
    // sat, swallowing tracks 2 and 3; tracks 1 and 4 stay standalone.
    let gid = group_tracks(&mut app, &[2, 3]);
    assert_eq!(
        app.test_mixer_top_level(),
        vec![(false, 1), (true, gid), (false, 4)],
        "the group cluster replaces its members at the first member's slot"
    );
}

#[test]
fn group_uses_first_members_slot_even_when_members_are_non_adjacent() {
    let mut app = app_with_tracks(4);
    // Members 1 and 3 are not adjacent in the lane; the cluster still
    // anchors at the *first* member (1) and consumes 3 from its later slot.
    let gid = group_tracks(&mut app, &[1, 3]);
    assert_eq!(
        app.test_mixer_top_level(),
        vec![(true, gid), (false, 2), (false, 4)],
        "a group anchors at its first member and pulls later members into the cluster"
    );
}

#[test]
fn two_groups_each_form_their_own_cluster() {
    let mut app = app_with_tracks(5);
    let g1 = group_tracks(&mut app, &[1, 2]);
    let g2 = group_tracks(&mut app, &[3, 4]);
    assert_eq!(
        app.test_mixer_top_level(),
        vec![(true, g1), (true, g2), (false, 5)],
        "each group forms its own cluster; the lone track 5 stays standalone"
    );
}

#[test]
fn collapsed_group_still_clusters_and_consumes_members() {
    let mut app = app_with_tracks(4);
    let gid = group_tracks(&mut app, &[2, 3]);
    // Fold the group. Collapsing hides the member strips inside the
    // cluster, but the top-level order is unchanged and the members must
    // never leak back out as standalone strips.
    assert!(app.test_track_groups_mut().set_collapse_state(gid, true));
    assert_eq!(
        app.test_mixer_top_level(),
        vec![(false, 1), (true, gid), (false, 4)],
        "a collapsed group keeps its slot and still consumes its members"
    );
}

#[test]
fn group_carries_the_arrange_identity_colour() {
    let mut app = app_with_tracks(2);
    let gid = group_tracks(&mut app, &[1, 2]);
    // The first group cycles to the first palette colour — the same value
    // the Arrange rail/swatch resolve through `theme::group_identity_colors`,
    // so the two surfaces share one identity colour by construction.
    assert_eq!(
        app.test_track_groups().get_group(gid).unwrap().identity_color,
        GroupIdentityColor::Drum
    );
}

#[test]
fn nested_group_resolves_to_its_root_for_clustering() {
    let mut app = app_with_tracks(3);
    // Build a one-level hierarchy directly on the registry (single-member
    // groups can't be made through the "Group selected" reducer): a child
    // group over {2,3} nested inside a parent group that also holds {1}.
    // Group ids live in the shared id space; pick ids clear of the tracks.
    let (parent, child) = (100u64, 101u64);
    {
        let groups = app.test_track_groups_mut();
        groups.create_group_from_selection(child, &[2, 3]);
        groups.create_group_from_selection(parent, &[1, child]);
        assert!(groups.set_nesting_parent(child, Some(parent)));
    }

    // Every member must resolve to the *root* parent so the whole thing
    // renders as one cluster, not two siblings.
    assert_eq!(app.test_mixer_root_group_of(1), Some(parent));
    assert_eq!(app.test_mixer_root_group_of(2), Some(parent));
    assert_eq!(app.test_mixer_root_group_of(3), Some(parent));

    // The whole hierarchy collapses to a single top-level cluster anchored
    // at the parent.
    let top = app.test_mixer_top_level();
    assert_eq!(
        top.iter().filter(|(is_group, _)| *is_group).count(),
        1,
        "a nested group hierarchy renders as one root cluster: {top:?}"
    );
    assert_eq!(top.first(), Some(&(true, parent)));
}
