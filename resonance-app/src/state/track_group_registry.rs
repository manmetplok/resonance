//! Track group registry for managing group state in resonance-app.
//!
//! This module provides the `TrackGroupRegistry` struct which holds all
//! `TrackGroup` instances and provides methods for managing group state,
//! membership, and nesting.

use std::collections::HashMap;

use resonance_common::track_group::TrackGroup;
use resonance_common::group_identity::GroupIdentityColor;
use resonance_common::automation::TrackId;

/// Central registry that holds all track group instances.
///
/// Groups are indexed by their `TrackId` for fast lookup. The registry
/// maintains ordered membership within each group and supports
/// nesting up to one level deep.
#[derive(Debug, Default, Clone)]
pub struct TrackGroupRegistry {
    /// All groups, indexed by their id.
    groups: HashMap<TrackId, TrackGroup>,
}

impl TrackGroupRegistry {
    /// Creates a new, empty track group registry.
    pub fn new() -> Self {
        Self {
            groups: HashMap::new(),
        }
    }

    /// Returns a reference to the group with the given id, if it exists.
    pub fn get_group(&self, id: TrackId) -> Option<&TrackGroup> {
        self.groups.get(&id)
    }

    /// Returns a mutable reference to the group with the given id, if it exists.
    pub fn get_group_mut(&mut self, id: TrackId) -> Option<&mut TrackGroup> {
        self.groups.get_mut(&id)
    }

    /// Returns all groups that contain the given track as a member.
    ///
    /// This includes both direct membership and nested group membership
    /// (if the track is a member of a nested group, the parent group is
    /// also returned).
    pub fn get_groups_containing_track(&self, track_id: TrackId) -> Vec<&TrackGroup> {
        self.groups
            .values()
            .filter(|group| {
                // Check if track is a direct member
                if group.ordered_members.contains(&track_id) {
                    return true;
                }
                // Check if track is a member of a nested group
                // (the nested group itself is a member of this group)
                group.ordered_members.iter().any(|member_id| {
                    if let Some(nested_group) = self.groups.get(member_id) {
                        nested_group.ordered_members.contains(&track_id)
                    } else {
                        false
                    }
                })
            })
            .collect()
    }

    /// Returns true when the track is soloed *via a group* — it belongs
    /// (directly, or through one level of nesting) to at least one group
    /// whose macro solo is engaged.
    ///
    /// This is deliberately independent of the track's own `soloed` flag:
    /// the macro cascade never touches a member's own solo. The flag drives
    /// both the "via group" header chip and the *effective* solo the audio
    /// engine receives, so a group solo and a per-track solo compose
    /// instead of fighting (todo #688).
    pub fn is_track_soloed_via_group(&self, track_id: TrackId) -> bool {
        self.get_groups_containing_track(track_id)
            .iter()
            .any(|group| group.macro_solo)
    }

    /// Returns the indent depth for a track based on its group membership.
    ///
    /// - Returns 0 for tracks that are not members of any group
    /// - Returns 1 for direct members of a group
    /// - Returns 2 for members of a nested group (group inside a group)
    ///
    /// This is used to visually indent group member tracks in the UI.
    pub fn indent_depth(&self, track_id: TrackId) -> usize {
        let groups = self.get_groups_containing_track(track_id);
        groups
            .iter()
            .filter_map(|group| {
                let mut depth = 1;
                let mut current_group = *group;
                // Walk up the nesting chain
                while let Some(parent_id) = current_group.nesting_parent {
                    depth += 1;
                    if let Some(parent) = self.groups.get(&parent_id) {
                        current_group = parent;
                    } else {
                        break;
                    }
                }
                Some(depth)
            })
            .max()
            .unwrap_or(0)
    }

    /// Returns the group identity colours for a track's parent groups.
    ///
    /// Returns a vector of identity colours, one for each level of group
    /// membership (direct parent first, then grandparent, etc.).
    /// Used for rendering the coloured rails on track headers.
    /// Tracks that are not group members return an empty vector.
    pub fn get_group_identity_colors(&self, track_id: TrackId) -> Vec<GroupIdentityColor> {
        let groups = self.get_groups_containing_track(track_id);
        groups
            .iter()
            .map(|group| group.identity_color)
            .collect()
    }

    pub fn get_all_groups(&self) -> Vec<&TrackGroup> {
        self.groups.values().collect()
    }

    /// Returns all groups ordered by id.
    ///
    /// Group ids live in the monotonically-assigned track-id space, so an
    /// id sort gives a stable, creation-ordered list. This is the order
    /// project save uses so the same registry always serialises to the
    /// same on-disk `track_groups` array — without it the hash-map
    /// iteration order leaks into `project.json` and re-saving an
    /// unchanged project produces a spurious diff.
    pub fn get_all_groups_sorted(&self) -> Vec<&TrackGroup> {
        let mut groups: Vec<&TrackGroup> = self.groups.values().collect();
        groups.sort_unstable_by_key(|g| g.id);
        groups
    }

    /// Returns all groups as mutable references.
    pub fn get_all_groups_mut(&mut self) -> Vec<&mut TrackGroup> {
        self.groups.values_mut().collect()
    }

    /// Adds a new group to the registry.
    ///
    /// The group's id must not already exist in the registry.
    /// Returns `Some(&TrackGroup)` if the group was added, or `None` if a
    /// group with that id already exists.
    pub fn add_group(&mut self, group: TrackGroup) -> Option<&TrackGroup> {
        if self.groups.contains_key(&group.id) {
            return None;
        }
        self.groups.insert(group.id, group.clone());
        self.groups.get(&group.id)
    }

    /// Adds a new group with the given parameters and returns its id.
    ///
    /// This is a convenience method that creates a new `TrackGroup` and adds it
    /// to the registry in one step. The group starts with no members, not
    /// collapsed, and default macro settings.
    pub fn add_group_new(
        &mut self,
        id: TrackId,
        name: impl Into<String>,
        identity_color: GroupIdentityColor,
    ) -> TrackId {
        let group = TrackGroup::new(id, name, identity_color);
        self.groups.insert(group.id, group);
        id
    }

    /// Creates a group from a selection of member tracks (todo #684).
    ///
    /// `id` is the freshly-allocated group id (from the shared track-id
    /// space). The group's name is auto-generated (`"Group N"`, where `N`
    /// counts up from the number of existing groups) and its identity
    /// colour cycles through [`GroupIdentityColor::all`] by that same
    /// count, so successive groups get visually distinct swatches. The
    /// `members` are added in the given order; duplicates are ignored by
    /// [`add_member`](Self::add_member). Returns the new group's id.
    pub fn create_group_from_selection(
        &mut self,
        id: TrackId,
        members: &[TrackId],
    ) -> TrackId {
        let n = self.groups.len();
        let name = format!("Group {}", n + 1);
        let palette = GroupIdentityColor::all();
        let color = palette[n % palette.len()];
        self.add_group_new(id, name, color);
        for &member in members {
            self.add_member(id, member);
        }
        id
    }

    /// Removes the group with the given id from the registry.
    ///
    /// Returns the removed group if it existed, or `None` if no such group existed.
    ///
    /// Note: This does NOT automatically remove the group's members from other
    /// groups they may belong to. Callers should handle cleanup of nested
    /// relationships if needed.
    pub fn remove_group(&mut self, id: TrackId) -> Option<TrackGroup> {
        self.groups.remove(&id)
    }

    /// Updates the group with the given id using the provided update function.
    ///
    /// Returns `Some(T)` if the group existed and was updated (where `T` is the
    /// return type of the closure), or `None` if no such group existed.
    pub fn update_group<F, R>(&mut self, id: TrackId, f: F) -> Option<R>
    where
        F: FnOnce(&mut TrackGroup) -> R,
    {
        self.groups.get_mut(&id).map(f)
    }

    /// Reorders the members of the group with the given id.
    ///
    /// The `new_order` parameter is a vector of track ids in the desired order.
    /// Only tracks that are already members of the group are included; others
    /// are silently ignored.
    ///
    /// Returns `true` if the group existed and was updated, `false` otherwise.
    pub fn reorder_members(&mut self, group_id: TrackId, new_order: Vec<TrackId>) -> bool {
        if let Some(group) = self.groups.get_mut(&group_id) {
            // Filter to only existing members and preserve their order
            let mut filtered_order = Vec::new();
            for &track_id in &new_order {
                if group.ordered_members.contains(&track_id)
                    && !filtered_order.contains(&track_id)
                {
                    filtered_order.push(track_id);
                }
            }
            // Add any members not in the new order at the end
            for &track_id in &group.ordered_members {
                if !filtered_order.contains(&track_id) {
                    filtered_order.push(track_id);
                }
            }
            group.ordered_members = filtered_order;
            true
        } else {
            false
        }
    }

    /// Sets the collapse state of the group with the given id.
    ///
    /// Returns `true` if the group existed and was updated, `false` otherwise.
    pub fn set_collapse_state(&mut self, group_id: TrackId, collapsed: bool) -> bool {
        if let Some(group) = self.groups.get_mut(&group_id) {
            group.is_collapsed = collapsed;
            true
        } else {
            false
        }
    }

    /// Returns the id of the group that lists `track_id` as a *direct*
    /// member, if any.
    ///
    /// Unlike [`get_groups_containing_track`](Self::get_groups_containing_track),
    /// this only considers direct membership (not nested-parent reach) and
    /// returns at most one group — the model treats a track as belonging to
    /// a single group at a time. When several groups happen to list the same
    /// track (which membership edits avoid), the lowest group id wins so the
    /// answer is deterministic. Used by drag-and-drop membership (todo #685)
    /// to know what to detach a dragged track from.
    pub fn group_of_member(&self, track_id: TrackId) -> Option<TrackId> {
        self.groups
            .values()
            .filter(|g| g.ordered_members.contains(&track_id))
            .map(|g| g.id)
            .min()
    }

    /// Returns true if `group_id` is the nesting parent of any other group.
    ///
    /// A group that already holds a nested child cannot itself be nested
    /// (that would push its child two levels deep), so drag-and-drop nesting
    /// (todo #685) refuses to move a parent group under another group.
    pub fn is_parent_group(&self, group_id: TrackId) -> bool {
        self.groups
            .values()
            .any(|g| g.nesting_parent == Some(group_id))
    }

    /// Adds a track to the specified group's membership.
    ///
    /// If the track is already a member, this is a no-op.
    /// Returns `true` if the track was added (or was already a member),
    /// `false` if the group doesn't exist.
    pub fn add_member(&mut self, group_id: TrackId, track_id: TrackId) -> bool {
        if let Some(group) = self.groups.get_mut(&group_id) {
            if !group.ordered_members.contains(&track_id) {
                group.ordered_members.push(track_id);
            }
            true
        } else {
            false
        }
    }

    /// Removes a track from the specified group's membership.
    ///
    /// Returns `true` if the track was removed, `false` if the group doesn't
    /// exist or the track wasn't a member.
    pub fn remove_member(&mut self, group_id: TrackId, track_id: TrackId) -> bool {
        if let Some(group) = self.groups.get_mut(&group_id) {
            let len_before = group.ordered_members.len();
            group.ordered_members.retain(|&id| id != track_id);
            len_before != group.ordered_members.len()
        } else {
            false
        }
    }

    /// Returns the number of groups in the registry.
    pub fn len(&self) -> usize {
        self.groups.len()
    }

    /// Returns true if the registry contains no groups.
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// Returns an iterator over all group ids.
    pub fn group_ids(&self) -> impl Iterator<Item = TrackId> + '_ {
        self.groups.keys().copied()
    }

    /// Validates that nesting does not exceed one level deep.
    ///
    /// Returns `true` if all nesting is valid (max 1 level deep),
    /// `false` if there's illegal nesting (e.g., a group nested inside
    /// another group that's already nested).
    pub fn validate_nesting(&self) -> bool {
        for group in self.groups.values() {
            if let Some(parent_id) = group.nesting_parent {
                // Check if the parent itself has a parent (2+ levels deep)
                if let Some(parent) = self.groups.get(&parent_id) {
                    if parent.nesting_parent.is_some() {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Sets the nesting parent of a group.
    ///
    /// Returns `true` if the operation succeeded, `false` if:
    /// - The group doesn't exist
    /// - The parent doesn't exist
    /// - Setting this parent would create nesting deeper than 1 level
    pub fn set_nesting_parent(
        &mut self,
        group_id: TrackId,
        parent_id: Option<TrackId>,
    ) -> bool {
        // First check if the group exists
        if !self.groups.contains_key(&group_id) {
            return false;
        }

        // If setting a parent, validate it first (before borrowing group mutably)
        if let Some(new_parent_id) = parent_id {
            // A group cannot be its own parent.
            if new_parent_id == group_id {
                return false;
            }
            // Parent must exist
            if !self.groups.contains_key(&new_parent_id) {
                return false;
            }
            // Check nesting depth: parent must not have a parent
            if let Some(parent) = self.groups.get(&new_parent_id) {
                if parent.nesting_parent.is_some() {
                    return false; // Would create 2+ levels
                }
            }
        }

        // Now we can safely borrow the group mutably
        if let Some(group) = self.groups.get_mut(&group_id) {
            group.nesting_parent = parent_id;
            true
        } else {
            false
        }
    }

    /// Returns all root-level groups (those not nested inside another group).
    pub fn get_root_groups(&self) -> Vec<&TrackGroup> {
        self.groups
            .values()
            .filter(|group| group.nesting_parent.is_none())
            .collect()
    }

    /// Returns all nested groups (those with a parent) along with their parent's id.
    pub fn get_nested_groups(&self) -> Vec<(TrackId, &TrackGroup)> {
        self.groups
            .values()
            .filter_map(|group| {
                group.nesting_parent.map(|parent_id| (parent_id, group))
            })
            .collect()
    }

    /// Returns the parent group of the given group, if it has one.
    pub fn get_parent_group(&self, group_id: TrackId) -> Option<&TrackGroup> {
        self.groups
            .get(&group_id)
            .and_then(|group| group.nesting_parent)
            .and_then(|parent_id| self.groups.get(&parent_id))
    }

    /// Returns all member track ids for the given group, including nested
    /// group members (flattened).
    ///
    /// This recursively collects all tracks that are effectively members of
    /// the group, including those in nested groups.
    pub fn get_all_member_ids(&self, group_id: TrackId) -> Vec<TrackId> {
        let mut result = Vec::new();
        self.collect_all_member_ids(group_id, &mut result);
        result
    }

    fn collect_all_member_ids(&self, group_id: TrackId, result: &mut Vec<TrackId>) {
        if let Some(group) = self.groups.get(&group_id) {
            for &member_id in &group.ordered_members {
                // Check if the member is itself a group (nested)
                if let Some(nested_group) = self.groups.get(&member_id) {
                    // Recursively collect nested group's members
                    self.collect_all_member_ids(nested_group.id, result);
                } else {
                    result.push(member_id);
                }
            }
        }
    }
}

impl std::ops::Index<TrackId> for TrackGroupRegistry {
    type Output = TrackGroup;

    fn index(&self, id: TrackId) -> &Self::Output {
        self.groups.get(&id).expect("TrackGroupRegistry: group not found")
    }
}
