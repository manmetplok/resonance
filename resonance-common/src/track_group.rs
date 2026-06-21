//! Track grouping & folder tracks data model (epic #36, doc #200).
//!
//! A [`TrackGroup`] represents a folder/group of tracks in the timeline.
//! Groups provide organizational structure (nesting, collapse) and macro
//! controls (mute, solo, level trim) that cascade to their members.
//!
//! The model is pure data; persistence, UI, and engine integration live in
//! their respective crates. This module is the single source of truth for
//! the group data structure.

use serde::{Deserialize, Serialize};

use crate::automation::TrackId;
use crate::group_identity::GroupIdentityColor;

/// Identifier for a [`TrackGroup`], unique within a project.
pub type GroupId = u64;

/// A group of tracks with shared organizational and control properties.
///
/// A group is an organizational construct that:
/// - Folds related lanes under one header
/// - Brackets them with a colour identity
/// - Exposes group-level mute / solo / level trim that cascade to members
///
/// The group itself is NOT a routing bus; it does not introduce a return
/// channel. The macro level trim is a convenience gain applied to members.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackGroup {
    /// Unique identifier for this group.
    pub id: GroupId,
    /// Human-readable name displayed in the group header.
    pub name: String,
    /// The identity colour for this group (swatch in header, rail on members).
    pub identity_color: GroupIdentityColor,
    /// Ordered list of track IDs that are members of this group.
    /// The order determines display order in the timeline.
    pub ordered_members: Vec<TrackId>,
    /// If this group is nested inside another group, this is the parent's ID.
    /// `None` for top-level groups.
    pub nesting_parent: Option<GroupId>,
    /// Whether the group is currently collapsed (members hidden).
    /// This state is persisted in the project.
    pub is_collapsed: bool,
    /// Macro mute: when true, all members are muted via the group.
    pub macro_mute: bool,
    /// Macro solo: when true, only members of this group are audible
    /// (standard solo behaviour — other groups and ungrouped tracks are muted).
    pub macro_solo: bool,
    /// Macro level trim: a gain multiplier applied to all members.
    /// This is a linear multiplier, not dB. Value of 1.0 = unity gain.
    pub macro_level: f32,
}

impl TrackGroup {
    /// Create a new track group with the given parameters.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for the group
    /// * `name` - Display name for the group
    /// * `identity_color` - Colour identity for visual grouping
    /// * `ordered_members` - Initial set of member track IDs
    /// * `nesting_parent` - Optional parent group ID for nested groups
    /// * `is_collapsed` - Initial collapse state
    /// * `macro_mute` - Initial macro mute state
    /// * `macro_solo` - Initial macro solo state
    /// * `macro_level` - Initial macro level (1.0 = unity)
    pub fn new(
        id: GroupId,
        name: impl Into<String>,
        identity_color: GroupIdentityColor,
        ordered_members: Vec<TrackId>,
        nesting_parent: Option<GroupId>,
        is_collapsed: bool,
        macro_mute: bool,
        macro_solo: bool,
        macro_level: f32,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            identity_color,
            ordered_members,
            nesting_parent,
            is_collapsed,
            macro_mute,
            macro_solo,
            macro_level,
        }
    }

    /// Create a new top-level (non-nested) track group with default states.
    ///
    /// This is a convenience constructor for the common case of creating a
    /// new group with default values for macro controls and collapse state.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for the group
    /// * `name` - Display name for the group
    /// * `identity_color` - Colour identity for visual grouping
    /// * `ordered_members` - Initial set of member track IDs
    pub fn new_top_level(
        id: GroupId,
        name: impl Into<String>,
        identity_color: GroupIdentityColor,
        ordered_members: Vec<TrackId>,
    ) -> Self {
        Self::new(
            id,
            name,
            identity_color,
            ordered_members,
            None,
            false, // not collapsed by default
            false, // macro mute off
            false, // macro solo off
            1.0,   // unity gain
        )
    }

    /// Returns true if this group has any members.
    pub fn has_members(&self) -> bool {
        !self.ordered_members.is_empty()
    }

    /// Returns the number of members in this group.
    pub fn member_count(&self) -> usize {
        self.ordered_members.len()
    }

    /// Returns true if this group is nested inside another group.
    pub fn is_nested(&self) -> bool {
        self.nesting_parent.is_some()
    }

    /// Returns true if the macro mute is active.
    pub fn is_macro_muted(&self) -> bool {
        self.macro_mute
    }

    /// Returns true if the macro solo is active.
    pub fn is_macro_soloed(&self) -> bool {
        self.macro_solo
    }

    /// Check if a specific track is a member of this group.
    pub fn contains_track(&self, track_id: TrackId) -> bool {
        self.ordered_members.contains(&track_id)
    }

    /// Get the index of a track in the ordered members list.
    /// Returns `None` if the track is not a member.
    pub fn index_of_track(&self, track_id: TrackId) -> Option<usize> {
        self.ordered_members.iter().position(|&id| id == track_id)
    }

    /// Add a track to the group at the end of the ordered members list.
    pub fn add_member(&mut self, track_id: TrackId) {
        if !self.ordered_members.contains(&track_id) {
            self.ordered_members.push(track_id);
        }
    }

    /// Remove a track from the group.
    pub fn remove_member(&mut self, track_id: TrackId) {
        self.ordered_members.retain(|&id| id != track_id);
    }

    /// Move a member to a new position in the ordered list.
    /// Returns `false` if the track is not a member or the index is out of bounds.
    pub fn move_member_to(&mut self, track_id: TrackId, new_index: usize) -> bool {
        if let Some(old_index) = self.index_of_track(track_id) {
            if new_index < self.ordered_members.len() {
                self.ordered_members.remove(old_index);
                self.ordered_members.insert(new_index, track_id);
                return true;
            }
        }
        false
    }

    /// Set the collapse state of the group.
    pub fn set_collapsed(&mut self, collapsed: bool) {
        self.is_collapsed = collapsed;
    }

    /// Toggle the collapse state of the group.
    pub fn toggle_collapsed(&mut self) {
        self.is_collapsed = !self.is_collapsed;
    }

    /// Set the macro mute state.
    pub fn set_macro_mute(&mut self, muted: bool) {
        self.macro_mute = muted;
    }

    /// Toggle the macro mute state.
    pub fn toggle_macro_mute(&mut self) {
        self.macro_mute = !self.macro_mute;
    }

    /// Set the macro solo state.
    pub fn set_macro_solo(&mut self, soloed: bool) {
        self.macro_solo = soloed;
    }

    /// Toggle the macro solo state.
    pub fn toggle_macro_solo(&mut self) {
        self.macro_solo = !self.macro_solo;
    }

    /// Set the macro level.
    /// The value is clamped to a reasonable range (0.0 to 4.0).
    pub fn set_macro_level(&mut self, level: f32) {
        // Clamp to a reasonable range: 0.0 (silent) to 4.0 (4x boost)
        self.macro_level = level.clamp(0.0, 4.0);
    }

    /// Apply a delta to the macro level.
    /// The result is clamped to the valid range.
    pub fn apply_macro_level_delta(&mut self, delta: f32) {
        self.set_macro_level(self.macro_level + delta);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_track_group() {
        let group = TrackGroup::new_top_level(
            1,
            "Drums",
            GroupIdentityColor::Drum,
            vec![10, 20, 30],
        );

        assert_eq!(group.id, 1);
        assert_eq!(group.name, "Drums");
        assert_eq!(group.identity_color, GroupIdentityColor::Drum);
        assert_eq!(group.ordered_members, vec![10, 20, 30]);
        assert_eq!(group.nesting_parent, None);
        assert!(!group.is_collapsed);
        assert!(!group.macro_mute);
        assert!(!group.macro_solo);
        assert!((group.macro_level - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_has_members() {
        let group_empty = TrackGroup::new_top_level(1, "Empty", GroupIdentityColor::Drum, vec![]);
        let group_with = TrackGroup::new_top_level(2, "Full", GroupIdentityColor::Vocal, vec![10]);

        assert!(!group_empty.has_members());
        assert!(group_with.has_members());
    }

    #[test]
    fn test_member_count() {
        let group = TrackGroup::new_top_level(1, "Test", GroupIdentityColor::Keys, vec![1, 2, 3]);
        assert_eq!(group.member_count(), 3);
    }

    #[test]
    fn test_is_nested() {
        let top_level = TrackGroup::new_top_level(1, "Top", GroupIdentityColor::Drum, vec![10]);
        let nested = TrackGroup::new(
            2,
            "Nested",
            GroupIdentityColor::Vocal,
            vec![20],
            Some(1),
            false,
            false,
            false,
            1.0,
        );

        assert!(!top_level.is_nested());
        assert!(nested.is_nested());
    }

    #[test]
    fn test_contains_track() {
        let group = TrackGroup::new_top_level(1, "Test", GroupIdentityColor::Guitar, vec![10, 20, 30]);

        assert!(group.contains_track(10));
        assert!(group.contains_track(20));
        assert!(group.contains_track(30));
        assert!(!group.contains_track(99));
    }

    #[test]
    fn test_index_of_track() {
        let group = TrackGroup::new_top_level(1, "Test", GroupIdentityColor::Drum, vec![10, 20, 30]);

        assert_eq!(group.index_of_track(10), Some(0));
        assert_eq!(group.index_of_track(20), Some(1));
        assert_eq!(group.index_of_track(30), Some(2));
        assert_eq!(group.index_of_track(99), None);
    }

    #[test]
    fn test_add_remove_member() {
        let mut group = TrackGroup::new_top_level(1, "Test", GroupIdentityColor::Keys, vec![10]);

        group.add_member(20);
        assert!(group.contains_track(20));
        assert_eq!(group.member_count(), 2);

        group.remove_member(10);
        assert!(!group.contains_track(10));
        assert_eq!(group.member_count(), 1);
    }

    #[test]
    fn test_move_member_to() {
        let mut group = TrackGroup::new_top_level(1, "Test", GroupIdentityColor::Vocal, vec![10, 20, 30]);

        assert!(group.move_member_to(20, 0));
        assert_eq!(group.ordered_members, vec![20, 10, 30]);

        assert!(group.move_member_to(10, 2));
        assert_eq!(group.ordered_members, vec![20, 30, 10]);

        // Try to move non-member
        assert!(!group.move_member_to(99, 0));

        // Try to move to out of bounds
        assert!(!group.move_member_to(20, 100));
    }

    #[test]
    fn test_set_toggle_collapsed() {
        let mut group = TrackGroup::new_top_level(1, "Test", GroupIdentityColor::Drum, vec![10]);

        assert!(!group.is_collapsed);

        group.set_collapsed(true);
        assert!(group.is_collapsed);

        group.toggle_collapsed();
        assert!(!group.is_collapsed);
    }

    #[test]
    fn test_macro_mute() {
        let mut group = TrackGroup::new_top_level(1, "Test", GroupIdentityColor::Keys, vec![10]);

        assert!(!group.is_macro_muted());

        group.set_macro_mute(true);
        assert!(group.is_macro_muted());

        group.toggle_macro_mute();
        assert!(!group.is_macro_muted());
    }

    #[test]
    fn test_macro_solo() {
        let mut group = TrackGroup::new_top_level(1, "Test", GroupIdentityColor::Guitar, vec![10]);

        assert!(!group.is_macro_soloed());

        group.set_macro_solo(true);
        assert!(group.is_macro_soloed());

        group.toggle_macro_solo();
        assert!(!group.is_macro_soloed());
    }

    #[test]
    fn test_macro_level() {
        let mut group = TrackGroup::new_top_level(1, "Test", GroupIdentityColor::Drum, vec![10]);

        group.set_macro_level(2.0);
        assert!((group.macro_level - 2.0).abs() < f32::EPSILON);

        // Test clamping at upper bound
        group.set_macro_level(10.0);
        assert!((group.macro_level - 4.0).abs() < f32::EPSILON);

        // Test clamping at lower bound
        group.set_macro_level(-5.0);
        assert!((group.macro_level - 0.0).abs() < f32::EPSILON);

        // Test delta
        group.set_macro_level(1.0);
        group.apply_macro_level_delta(0.5);
        assert!((group.macro_level - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_serde_roundtrip() {
        let group = TrackGroup::new(
            42,
            "My Group",
            GroupIdentityColor::Vocal,
            vec![1, 2, 3],
            Some(10),
            true,
            true,
            false,
            1.5,
        );

        let serialized = serde_json::to_string(&group).unwrap();
        let deserialized: TrackGroup = serde_json::from_str(&serialized).unwrap();

        assert_eq!(group, deserialized);
    }
}
