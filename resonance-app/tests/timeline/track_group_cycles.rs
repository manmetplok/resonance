//! Regression coverage for membership cycles in `track_groups`.
//!
//! A project file whose groups referenced each other in a loop used to
//! recurse forever in `TrackGroupRegistry::get_all_member_ids` and abort
//! the app with a stack overflow — the flattening walk runs from view
//! code every frame. Two layers guard it now: `add_group` drops the
//! membership edge that closes a cycle at ingestion (both project-load
//! paths insert groups through it), and the flattening walk itself is
//! iterative with a visited set so no registry contents can make it
//! recurse.

use resonance_app::project::ProjectFile;
use resonance_app::state::{TrackGroupRegistry, ViewMode};
use resonance_app::Resonance;
use resonance_common::group_identity::GroupIdentityColor;
use resonance_common::track_group::TrackGroup;

fn group(id: u64, members: Vec<u64>) -> TrackGroup {
    let mut g = TrackGroup::new(id, format!("Group {id}"), GroupIdentityColor::Drum);
    g.ordered_members = members;
    g
}

#[test]
fn cyclic_project_track_groups_load_with_the_cycle_broken() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    let file = ProjectFile {
        track_groups: vec![
            group(100, vec![10, 101]),
            group(101, vec![20, 100]),
            // Self-membership is the tightest possible cycle.
            group(102, vec![102, 30]),
        ],
        ..ProjectFile::default()
    };
    app.test_replay_loaded_project(file);

    let groups = app.test_track_groups();
    assert_eq!(groups.len(), 3, "the whole project loads; no group is refused");
    // Group 101 arrived second, so its edge back to 100 is the one that
    // closed the loop and gets dropped; every other member survives.
    assert_eq!(groups.get_all_member_ids(100), vec![10, 20]);
    assert_eq!(groups.get_all_member_ids(101), vec![20]);
    assert_eq!(groups.get_all_member_ids(102), vec![30]);
}

#[test]
fn a_longer_membership_loop_is_broken_at_ingestion() {
    let mut registry = TrackGroupRegistry::new();
    registry.add_group(group(1, vec![11, 2]));
    registry.add_group(group(2, vec![12, 3]));
    registry.add_group(group(3, vec![13, 1]));

    // Group 3 closed the 1 -> 2 -> 3 -> 1 loop, so its edge back to 1 is
    // dropped while the rest of the chain keeps flattening through it.
    assert_eq!(registry.get_all_member_ids(1), vec![11, 12, 13]);
    assert_eq!(registry.get_all_member_ids(2), vec![12, 13]);
    assert_eq!(registry.get_all_member_ids(3), vec![13]);
}

#[test]
fn flattening_terminates_even_on_a_directly_mutated_cycle() {
    // Runtime code edits membership through `get_group_mut` /
    // `update_group` without re-validation, so the walk itself must hold
    // on a registry that really is cyclic.
    let mut registry = TrackGroupRegistry::new();
    registry.add_group_new(1, "A", GroupIdentityColor::Drum);
    registry.add_group_new(2, "B", GroupIdentityColor::Vocal);
    registry.update_group(1, |g| g.ordered_members = vec![10, 2]);
    registry.update_group(2, |g| g.ordered_members = vec![20, 1]);

    // Before the iterative walk this recursed until the stack blew.
    assert_eq!(registry.get_all_member_ids(1), vec![10, 20]);
    assert_eq!(registry.get_all_member_ids(2), vec![20, 10]);
}
