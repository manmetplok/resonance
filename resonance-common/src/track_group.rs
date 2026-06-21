//! Track-group (folder track) data model shared across the app, project I/O
//! and the offline bounce (architecture doc #200, epic #36).
//!
//! A **group** is an *organisational + macro-control* construct: it folds
//! related tracks under one header, brackets them with a colour identity, and
//! exposes group-level mute / solo / level trim that cascade to members. It is
//! deliberately **not** a bus — it introduces no return channel — and it is
//! distinct from instrument sub-tracks (fan-out ports of one plugin); a group's
//! members are still full, independent tracks.
//!
//! This module owns only the persisted data structure. The macro cascade, the
//! drag-and-drop membership edits, the timeline header row and the project
//! registry that ties groups together all live in `resonance-app`; they read
//! and write the fields defined here so live mix, offline bounce and project
//! persistence agree on what a group is.
//!
//! Group id, name, identity colour, ordered membership, nesting, fold state and
//! the macro mute/solo/level all live in the project (the epic requires fold
//! state to survive save/reload) — hence the `Serialize`/`Deserialize` derives.

use serde::{Deserialize, Serialize};

use crate::automation::TrackId;
use crate::group_identity::GroupIdentityColor;

/// Macro level (group trim) value for unity gain — the group neither boosts nor
/// attenuates its members' contribution. The trim *scales* members' output (a
/// macro gain); it never overwrites their per-track faders.
pub const MACRO_LEVEL_UNITY: f32 = 1.0;

/// A folder/group track: an organisational header that brackets a set of member
/// tracks and applies group-level macro controls to them.
///
/// `id` lives in the same identifier space as ordinary tracks ([`TrackId`]) —
/// the group header is a first-class timeline row, so it owns a track id rather
/// than a separate one. `ordered_members` preserves the user's arrangement
/// order within the folder. `nesting_parent` records the enclosing group when
/// this group is nested one level deep (the only depth the epic supports).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackGroup {
    /// Group identifier, in the shared track-id space.
    pub id: TrackId,
    /// User-facing group name (shown bold in the header).
    pub name: String,
    /// Identity colour slot for the swatch and member rail.
    pub identity_color: GroupIdentityColor,
    /// Member track ids, in arrangement order within the folder.
    pub ordered_members: Vec<TrackId>,
    /// Enclosing group, when this group is nested inside another.
    pub nesting_parent: Option<TrackId>,
    /// Whether the folder is collapsed (members hidden, lane shown as a
    /// consolidated overview). Persisted with the project.
    pub is_collapsed: bool,
    /// Macro mute: cascades to members without overwriting their own mute.
    pub macro_mute: bool,
    /// Macro solo: cascades to members without overwriting their own solo.
    pub macro_solo: bool,
    /// Macro level trim — a multiplicative gain scaling members' contribution.
    /// `1.0` ([`MACRO_LEVEL_UNITY`]) is unity.
    pub macro_level: f32,
}

impl TrackGroup {
    /// A new, empty group with the given id, name and identity colour. Starts
    /// expanded, un-nested, with both macros off and the level at unity.
    pub fn new(
        id: TrackId,
        name: impl Into<String>,
        identity_color: GroupIdentityColor,
    ) -> TrackGroup {
        TrackGroup {
            id,
            name: name.into(),
            identity_color,
            ordered_members: Vec::new(),
            nesting_parent: None,
            is_collapsed: false,
            macro_mute: false,
            macro_solo: false,
            macro_level: MACRO_LEVEL_UNITY,
        }
    }
}
