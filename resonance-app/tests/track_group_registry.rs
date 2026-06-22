//! Tests for `TrackGroupRegistry` — group state, membership, ordering and
//! one-level nesting (ba todo #678, epic #36).

use resonance_app::state::TrackGroupRegistry;
use resonance_common::automation::TrackId;
use resonance_common::group_identity::GroupIdentityColor;
use resonance_common::track_group::TrackGroup;

fn track_id(i: u64) -> TrackId {
    i
}

#[test]
fn new_registry_is_empty() {
    let registry = TrackGroupRegistry::new();
    assert!(registry.is_empty());
    assert_eq!(registry.len(), 0);
}

#[test]
fn add_and_get_group() {
    let mut registry = TrackGroupRegistry::new();
    let group_id = track_id(1);
    let group = TrackGroup::new(group_id, "Drums", GroupIdentityColor::Drum);

    assert!(registry.add_group(group.clone()).is_some());
    assert_eq!(registry.len(), 1);

    let retrieved = registry.get_group(group_id);
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().id, group_id);
    assert_eq!(retrieved.unwrap().name, "Drums");
}

#[test]
fn add_group_duplicate_id_is_rejected() {
    let mut registry = TrackGroupRegistry::new();
    let group_id = track_id(1);
    let group1 = TrackGroup::new(group_id, "Drums", GroupIdentityColor::Drum);
    let group2 = TrackGroup::new(group_id, "Vocals", GroupIdentityColor::Vocal);

    assert!(registry.add_group(group1).is_some());
    assert!(registry.add_group(group2).is_none());
    assert_eq!(registry.len(), 1);
}

#[test]
fn remove_group() {
    let mut registry = TrackGroupRegistry::new();
    let group_id = track_id(1);
    let group = TrackGroup::new(group_id, "Drums", GroupIdentityColor::Drum);

    registry.add_group(group);
    assert_eq!(registry.len(), 1);

    let removed = registry.remove_group(group_id);
    assert!(removed.is_some());
    assert_eq!(registry.len(), 0);
}

#[test]
fn add_member() {
    let mut registry = TrackGroupRegistry::new();
    let group_id = track_id(1);
    let track1 = track_id(10);
    let track2 = track_id(11);

    registry.add_group_new(group_id, "Drums", GroupIdentityColor::Drum);

    assert!(registry.add_member(group_id, track1));
    assert!(registry.add_member(group_id, track2));

    let group = registry.get_group(group_id).unwrap();
    assert!(group.ordered_members.contains(&track1));
    assert!(group.ordered_members.contains(&track2));
}

#[test]
fn add_member_is_idempotent() {
    let mut registry = TrackGroupRegistry::new();
    let group_id = track_id(1);
    let track1 = track_id(10);

    registry.add_group_new(group_id, "Drums", GroupIdentityColor::Drum);
    assert!(registry.add_member(group_id, track1));
    assert!(registry.add_member(group_id, track1));

    let group = registry.get_group(group_id).unwrap();
    assert_eq!(group.ordered_members, vec![track1]);
}

#[test]
fn remove_member() {
    let mut registry = TrackGroupRegistry::new();
    let group_id = track_id(1);
    let track1 = track_id(10);
    let track2 = track_id(11);

    let mut group = TrackGroup::new(group_id, "Drums", GroupIdentityColor::Drum);
    group.ordered_members = vec![track1, track2];
    registry.add_group(group);

    assert!(registry.remove_member(group_id, track1));
    let group = registry.get_group(group_id).unwrap();
    assert!(!group.ordered_members.contains(&track1));
    assert!(group.ordered_members.contains(&track2));
}

#[test]
fn get_groups_containing_track() {
    let mut registry = TrackGroupRegistry::new();
    let group1_id = track_id(1);
    let group2_id = track_id(2);
    let track1 = track_id(10);
    let track2 = track_id(11);

    let mut group1 = TrackGroup::new(group1_id, "Drums", GroupIdentityColor::Drum);
    group1.ordered_members = vec![track1, track2];
    registry.add_group(group1);

    let mut group2 = TrackGroup::new(group2_id, "Vocals", GroupIdentityColor::Vocal);
    group2.ordered_members = vec![track2];
    registry.add_group(group2);

    let containing = registry.get_groups_containing_track(track2);
    assert_eq!(containing.len(), 2);
    assert!(containing.iter().any(|g| g.id == group1_id));
    assert!(containing.iter().any(|g| g.id == group2_id));
}

#[test]
fn set_collapse_state() {
    let mut registry = TrackGroupRegistry::new();
    let group_id = track_id(1);

    registry.add_group_new(group_id, "Drums", GroupIdentityColor::Drum);

    let group = registry.get_group(group_id).unwrap();
    assert!(!group.is_collapsed);

    assert!(registry.set_collapse_state(group_id, true));
    let group = registry.get_group(group_id).unwrap();
    assert!(group.is_collapsed);
}

#[test]
fn set_collapse_state_unknown_group_returns_false() {
    let mut registry = TrackGroupRegistry::new();
    assert!(!registry.set_collapse_state(track_id(99), true));
}

#[test]
fn reorder_members() {
    let mut registry = TrackGroupRegistry::new();
    let group_id = track_id(1);
    let track1 = track_id(10);
    let track2 = track_id(11);
    let track3 = track_id(12);

    let mut group = TrackGroup::new(group_id, "Drums", GroupIdentityColor::Drum);
    group.ordered_members = vec![track1, track2, track3];
    registry.add_group(group);

    assert!(registry.reorder_members(group_id, vec![track3, track1, track2]));
    let group = registry.get_group(group_id).unwrap();
    assert_eq!(group.ordered_members, vec![track3, track1, track2]);
}

#[test]
fn reorder_members_ignores_non_members_and_keeps_omitted() {
    let mut registry = TrackGroupRegistry::new();
    let group_id = track_id(1);
    let track1 = track_id(10);
    let track2 = track_id(11);
    let stranger = track_id(99);

    let mut group = TrackGroup::new(group_id, "Drums", GroupIdentityColor::Drum);
    group.ordered_members = vec![track1, track2];
    registry.add_group(group);

    // `stranger` is not a member (ignored); `track2` is omitted from the new
    // order, so it should be appended at the end.
    assert!(registry.reorder_members(group_id, vec![stranger, track1]));
    let group = registry.get_group(group_id).unwrap();
    assert_eq!(group.ordered_members, vec![track1, track2]);
}

#[test]
fn nesting_validation_rejects_two_levels() {
    let mut registry = TrackGroupRegistry::new();
    let parent_id = track_id(1);
    let child_id = track_id(2);
    let grandchild_id = track_id(3);

    registry.add_group_new(parent_id, "Parent", GroupIdentityColor::Drum);
    registry.add_group_new(child_id, "Child", GroupIdentityColor::Vocal);
    registry.add_group_new(grandchild_id, "Grandchild", GroupIdentityColor::Keys);

    // Set child's parent to parent (valid: 1 level).
    assert!(registry.set_nesting_parent(child_id, Some(parent_id)));
    assert!(registry.validate_nesting());

    // Try to set grandchild's parent to child (would be 2 levels).
    assert!(!registry.set_nesting_parent(grandchild_id, Some(child_id)));
    assert!(registry.validate_nesting());
}

#[test]
fn set_nesting_parent_rejects_self_and_missing_parent() {
    let mut registry = TrackGroupRegistry::new();
    let group_id = track_id(1);
    registry.add_group_new(group_id, "Group", GroupIdentityColor::Drum);

    // A group cannot be its own parent.
    assert!(!registry.set_nesting_parent(group_id, Some(group_id)));
    // Parent must exist.
    assert!(!registry.set_nesting_parent(group_id, Some(track_id(42))));
    // Clearing the parent always succeeds.
    assert!(registry.set_nesting_parent(group_id, None));
}

#[test]
fn get_root_groups() {
    let mut registry = TrackGroupRegistry::new();
    let root1_id = track_id(1);
    let root2_id = track_id(2);
    let child_id = track_id(3);

    registry.add_group_new(root1_id, "Root1", GroupIdentityColor::Drum);
    registry.add_group_new(root2_id, "Root2", GroupIdentityColor::Vocal);
    registry.add_group_new(child_id, "Child", GroupIdentityColor::Keys);
    registry.set_nesting_parent(child_id, Some(root1_id));

    let roots = registry.get_root_groups();
    assert_eq!(roots.len(), 2);
    assert!(roots.iter().any(|g| g.id == root1_id));
    assert!(roots.iter().any(|g| g.id == root2_id));
}

#[test]
fn get_all_groups_sorted_orders_by_id_regardless_of_insertion() {
    let mut registry = TrackGroupRegistry::new();
    // Insert out of id order; the sorted view must still come back by id.
    registry.add_group_new(track_id(30), "C", GroupIdentityColor::Keys);
    registry.add_group_new(track_id(10), "A", GroupIdentityColor::Drum);
    registry.add_group_new(track_id(20), "B", GroupIdentityColor::Vocal);

    let ids: Vec<TrackId> = registry
        .get_all_groups_sorted()
        .iter()
        .map(|g| g.id)
        .collect();
    assert_eq!(ids, vec![track_id(10), track_id(20), track_id(30)]);
}

#[test]
fn get_all_member_ids_flattens_nesting() {
    let mut registry = TrackGroupRegistry::new();
    let parent_id = track_id(1);
    let child_id = track_id(2);
    let track1 = track_id(10);
    let track2 = track_id(11);

    let mut parent = TrackGroup::new(parent_id, "Parent", GroupIdentityColor::Drum);
    parent.ordered_members = vec![child_id, track1];
    registry.add_group(parent);

    let mut child = TrackGroup::new(child_id, "Child", GroupIdentityColor::Vocal);
    child.nesting_parent = Some(parent_id);
    child.ordered_members = vec![track2];
    registry.add_group(child);

    let all_members = registry.get_all_member_ids(parent_id);
    assert_eq!(all_members.len(), 2);
    assert!(all_members.contains(&track1));
    assert!(all_members.contains(&track2));
}

#[test]
fn indent_depth_ungrouped_track() {
    let mut registry = TrackGroupRegistry::new();
    let group_id = track_id(1);
    let track1 = track_id(10);
    let track2 = track_id(11);

    // Add a group with track1 as member
    let mut group = TrackGroup::new(group_id, "Drums", GroupIdentityColor::Drum);
    group.ordered_members = vec![track1];
    registry.add_group(group);

    // track2 is not a member of any group
    assert_eq!(registry.indent_depth(track2), 0);
}

#[test]
fn indent_depth_direct_member() {
    let mut registry = TrackGroupRegistry::new();
    let group_id = track_id(1);
    let track1 = track_id(10);

    // Add a group with track1 as member
    let mut group = TrackGroup::new(group_id, "Drums", GroupIdentityColor::Drum);
    group.ordered_members = vec![track1];
    registry.add_group(group);

    // track1 is a direct member of one group
    assert_eq!(registry.indent_depth(track1), 1);
}

#[test]
fn indent_depth_nested_member() {
    let mut registry = TrackGroupRegistry::new();
    let parent_id = track_id(1);
    let child_id = track_id(2);
    let track1 = track_id(10);

    // Create parent group with child as member
    let mut parent = TrackGroup::new(parent_id, "Parent", GroupIdentityColor::Drum);
    parent.ordered_members = vec![child_id];
    registry.add_group(parent);

    // Create child group (nested under parent) with track1 as member
    let mut child = TrackGroup::new(child_id, "Child", GroupIdentityColor::Vocal);
    child.nesting_parent = Some(parent_id);
    child.ordered_members = vec![track1];
    registry.add_group(child);

    // track1 is a member of a nested group (child inside parent)
    assert_eq!(registry.indent_depth(track1), 2);
}

#[test]
fn indent_depth_multiple_groups() {
    let mut registry = TrackGroupRegistry::new();
    let group1_id = track_id(1);
    let group2_id = track_id(2);
    let track1 = track_id(10);

    // Add two groups, both containing track1
    let mut group1 = TrackGroup::new(group1_id, "Group1", GroupIdentityColor::Drum);
    group1.ordered_members = vec![track1];
    registry.add_group(group1);

    let mut group2 = TrackGroup::new(group2_id, "Group2", GroupIdentityColor::Vocal);
    group2.ordered_members = vec![track1];
    registry.add_group(group2);

    // track1 is a direct member of both groups, max depth is 1
    assert_eq!(registry.indent_depth(track1), 1);
}

#[test]
fn get_group_identity_colors_ungrouped_track() {
    let registry = TrackGroupRegistry::new();
    let track1 = track_id(10);

    // Ungrouped track should return empty vector
    let colors = registry.get_group_identity_colors(track1);
    assert!(colors.is_empty());
}

#[test]
fn get_group_identity_colors_direct_member() {
    let mut registry = TrackGroupRegistry::new();
    let group_id = track_id(1);
    let track1 = track_id(10);

    // Create a group with Drums identity
    let mut group = TrackGroup::new(group_id, "Drums", GroupIdentityColor::Drum);
    group.ordered_members = vec![track1];
    registry.add_group(group);

    // track1 should have Drums color
    let colors = registry.get_group_identity_colors(track1);
    assert_eq!(colors.len(), 1);
    assert_eq!(colors[0], GroupIdentityColor::Drum);
}

#[test]
fn get_group_identity_colors_nested_member() {
    let mut registry = TrackGroupRegistry::new();
    let parent_id = track_id(1);
    let child_id = track_id(2);
    let track1 = track_id(10);

    // Create parent group with Drums identity
    let mut parent = TrackGroup::new(parent_id, "Parent", GroupIdentityColor::Drum);
    parent.ordered_members = vec![child_id];
    registry.add_group(parent);

    // Create child group (nested under parent) with Vocal identity
    let mut child = TrackGroup::new(child_id, "Child", GroupIdentityColor::Vocal);
    child.nesting_parent = Some(parent_id);
    child.ordered_members = vec![track1];
    registry.add_group(child);

    // track1 is a member of child which is nested in parent
    // get_groups_containing_track should return both parent and child
    // So colors should include both Vocal and Drums
    let colors = registry.get_group_identity_colors(track1);
    assert_eq!(colors.len(), 2);
    // Colors should be ordered outermost->innermost: parent (Drum, depth 0) then child (Vocal, depth 1)
    assert_eq!(colors, vec![GroupIdentityColor::Drum, GroupIdentityColor::Vocal]);
}

#[test]
fn get_group_identity_colors_multiple_groups() {
    let mut registry = TrackGroupRegistry::new();
    let group1_id = track_id(1);
    let group2_id = track_id(2);
    let track1 = track_id(10);

    // Add two groups, both containing track1
    let mut group1 = TrackGroup::new(group1_id, "Group1", GroupIdentityColor::Drum);
    group1.ordered_members = vec![track1];
    registry.add_group(group1);

    let mut group2 = TrackGroup::new(group2_id, "Group2", GroupIdentityColor::Vocal);
    group2.ordered_members = vec![track1];
    registry.add_group(group2);

    // track1 is a direct member of both groups
    let colors = registry.get_group_identity_colors(track1);
    assert_eq!(colors.len(), 2);
    // Both groups have depth 0, so order is by id: group1 (Drum, id=1) then group2 (Vocal, id=2)
    assert_eq!(colors, vec![GroupIdentityColor::Drum, GroupIdentityColor::Vocal]);
}
