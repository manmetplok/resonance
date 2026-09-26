//! Track-group (folder-track) reducers (epic #36, doc #200).
//!
//! The group-header row view (todo #680) emits [`GroupMessage`]s for its
//! caret, macro `M`/`S` buttons and level trim. The behaviour behind each
//! of those controls is owned by dedicated follow-up todos so the macro
//! cascade and fold-state logic land with their tests, not buried in the
//! view todo:
//!
//! - macro mute with cascade to members — todo #687
//! - macro solo with cascade to members — todo #688
//!
//! The collapse / fold state (todo #686), group level trim (todo #689) and
//! both macro cascades (mute #687 / solo #688) are implemented below, each
//! with its own reducer and tests so the header is wired end-to-end (and
//! snapshot-testable).
//!
//! Two reducer families *are* implemented here: group creation from a
//! multi-track selection (todo #684) and drag-and-drop membership editing
//! (todo #685 — see the "Drag-and-drop membership" section below).

use iced::Task;

use crate::message::Message;
use crate::state::{MembershipDragState, MembershipDragSubject, MembershipDropTarget};
use crate::Resonance;
use resonance_audio::types::{AudioCommand, TrackId};

/// Track-group (folder-track) messages emitted by the group-header row
/// (epic #36, doc #200). The header is an organisational + macro-control
/// strip: caret folds the group, `M`/`S` toggle the macro mute/solo that
/// cascade to members, and the level trim scales members' contribution.
///
/// The header *view* (todo #680) emits the caret / macro / trim variants;
/// the reducers that apply them — collapse/fold (#686), macro mute (#687),
/// macro solo (#688) and level trim (#689) — land in their own todos.
/// Until then they route to the placeholder `update::group::handle`.
///
/// The `*MembershipDrag*` / `*Membership*` variants drive drag-and-drop
/// group membership (todo #685): a track row or group header is dragged
/// onto a group to join / nest, or onto open space to ungroup / un-nest.
/// Their reducers live in `update::group` and mutate the registry directly.
#[derive(Debug, Clone)]
pub enum GroupMessage {
    /// Fold / unfold a group, hiding or showing its member lanes.
    ToggleCollapse(TrackId),
    /// Toggle the group's macro mute (cascades to members non-destructively).
    ToggleMacroMute(TrackId),
    /// Toggle the group's macro solo (cascades to members non-destructively).
    ToggleMacroSolo(TrackId),
    /// Set the group's macro level trim — a multiplicative gain scaling
    /// members' contribution (`1.0` is unity).
    SetMacroLevel(TrackId, f32),
    /// Create a new group from the current multi-track selection (the
    /// "Group selected" floating-bar action and the `Cmd-G` shortcut,
    /// todo #684). The selected tracks become the new group's members; a
    /// no-op when fewer than two tracks are selected.
    CreateGroupFromSelection,
    /// Begin a drag-and-drop membership edit (todo #685). The subject is the
    /// track row or group header that was grabbed; `cursor_y` is the pointer
    /// Y in the header column at grab, for the drag ghost.
    StartMembershipDrag(MembershipDragSubject, f32),
    /// The active membership drag's pointer moved. `target` is the drop
    /// target the view resolved under the cursor (`None` when over nothing
    /// droppable); `cursor_y` is the latest pointer Y.
    UpdateMembershipDrag {
        target: Option<MembershipDropTarget>,
        cursor_y: f32,
    },
    /// Commit the active membership drag, applying the hovered target's
    /// change to the group registry. A no-op when nothing is dragging or no
    /// valid target is hovered.
    DropMembership,
    /// Abandon the active membership drag with no change (released off any
    /// target, or `Esc`).
    CancelMembershipDrag,
}

impl GroupMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::{CoalesceKey, UndoAction};
        match self {
            // Creating a group from the selection mutates the persisted group
            // registry, so it's a recordable edit (todo #684).
            Self::CreateGroupFromSelection => UndoAction::Record,
            // Committing a drag-and-drop membership change is the single
            // recordable point of the gesture (todo #685): the registry's
            // membership / nesting is part of the persisted project, so an
            // undo restores the prior grouping.
            Self::DropMembership => UndoAction::Record,
            // Toggling a group's macro solo / mute mutates the persisted group
            // registry (macro_solo / macro_mute live in the project), so each
            // is a single recordable edit; a member's own solo / mute is left
            // untouched, so undo restores the exact prior group + per-track
            // picture (#687, #688).
            Self::ToggleMacroSolo(..) | Self::ToggleMacroMute(..) => UndoAction::Record,
            // Folding / unfolding a group flips the persisted `is_collapsed`
            // flag (todo #686, persisted via #690), so a single toggle is a
            // recordable edit — undo restores the prior fold picture and the
            // hidden member rows reappear.
            Self::ToggleCollapse(..) => UndoAction::Record,
            // The group level trim is a continuous control (a slider drag, like
            // a fader): coalesce the burst into one entry keyed by group so a
            // drag undoes in a single step, restoring the persisted
            // `macro_level` (todo #689).
            Self::SetMacroLevel(group_id, _) => {
                UndoAction::RecordCoalesced(CoalesceKey::GroupMacroLevel(*group_id))
            }
            // The membership drag's start / update / cancel phases are transient
            // UI bookkeeping and skip the history.
            Self::StartMembershipDrag(..)
            | Self::UpdateMembershipDrag { .. }
            | Self::CancelMembershipDrag => UndoAction::Skip,
        }
    }
}

pub fn handle(r: &mut Resonance, m: GroupMessage) -> Task<Message> {
    match m {
        // Collapse / fold state in the timeline — todo #686.
        GroupMessage::ToggleCollapse(group_id) => toggle_collapse(r, group_id),
        // Macro solo (todo #688) and macro mute (todo #687) both cascade to
        // members; the group level trim (todo #689) writes the persisted
        // macro_level. Each routes to its dedicated reducer below.
        GroupMessage::ToggleMacroSolo(group_id) => toggle_macro_solo(r, group_id),
        GroupMessage::ToggleMacroMute(group_id) => toggle_macro_mute(r, group_id),
        GroupMessage::SetMacroLevel(group_id, level) => set_macro_level(r, group_id, level),
        GroupMessage::CreateGroupFromSelection => create_group_from_selection(r),
        GroupMessage::StartMembershipDrag(subject, cursor_y) => {
            start_membership_drag(r, subject, cursor_y)
        }
        GroupMessage::UpdateMembershipDrag { target, cursor_y } => {
            update_membership_drag(r, target, cursor_y)
        }
        GroupMessage::DropMembership => drop_membership(r),
        GroupMessage::CancelMembershipDrag => {
            r.ui.interaction.membership_drag = None;
        }
    }
    Task::none()
}

/// Fold / unfold a group (todo #686). Flips the group's `is_collapsed`
/// flag; the arrange view reads it through the shared `ArrangeRowLayout`
/// (doc #203), which omits a collapsed group's member rows from both the
/// timeline canvas and the track-header column while keeping the group's
/// own header row — so the group can always be re-expanded. The flag is
/// persisted in the project file (todo #690), so the fold state survives a
/// save/reload round-trip and is captured by the undo snapshot. A no-op
/// for an unknown id (stale message after the group was removed).
fn toggle_collapse(r: &mut Resonance, group_id: TrackId) {
    r.track_groups
        .update_group(group_id, |group| group.is_collapsed = !group.is_collapsed);
}

/// Fold the current multi-track selection into a fresh group (the
/// "Group selected" bar / `Cmd-G`). A group of one is meaningless, so this
/// no-ops below two selected tracks — matching the bar's visibility
/// threshold. The group id is drawn from the shared track-id allocator so
/// it can never collide with a real track. The selection is cleared once
/// the group exists, so the floating bar dismisses itself.
fn create_group_from_selection(r: &mut Resonance) {
    let members = r.ui.interaction.selected_tracks.clone();
    if members.len() < 2 {
        return;
    }
    // Tracks and groups share one id space; the allocator skips both
    // (STATE-04, FU-A1c).
    let group_id = r.allocate_track_id();
    r.track_groups.create_group_from_selection(group_id, &members);
    r.ui.interaction.select_single_track(None);
}

/// Set the group's macro level trim — a multiplicative gain that scales
/// every member's contribution (`1.0` is unity). The trim deliberately does
/// **not** touch the members' own faders: a member's effective level is
/// `member_volume * group.macro_level`, so returning the trim to unity
/// restores each member exactly. This writes the persisted group state only
/// (mirroring the rest of the group-macro family); the engine-side cascade
/// that turns the macro values into live gain lands with the shared
/// group→engine integration, not here. A no-op for an unknown id.
fn set_macro_level(r: &mut Resonance, group_id: TrackId, level: f32) {
    r.track_groups.update_group(group_id, |group| {
        group.macro_level = level;
    });
}

/// Toggle a group's macro solo and cascade the *effective* solo to every
/// member track (todo #688, doc #200).
///
/// Macro solo is non-destructive: it never writes a member's own `soloed`
/// flag. Instead the engine receives each member's *effective* solo — its
/// own solo OR any containing group's macro solo — so a member stays
/// soloed while the group solo holds and reverts to its own state when the
/// group solo clears. Engaging the group solo therefore dims every
/// ungrouped track through the engine's standard solo logic. Nested-group
/// members travel with the cascade via the flattened member list.
fn toggle_macro_solo(r: &mut Resonance, group_id: TrackId) {
    if r.track_groups
        .update_group(group_id, |g| g.macro_solo = !g.macro_solo)
        .is_none()
    {
        return; // no such group — stale message after removal
    }
    for member in r.track_groups.get_all_member_ids(group_id) {
        let soloed = member_effective_solo(r, member);
        let _ = r.engine.send(AudioCommand::SetTrackSolo {
            track_id: member,
            soloed,
        });
    }
}

/// A track's *effective* solo: its own `soloed` flag OR membership in any
/// macro-soloed group. This is what the audio engine must see so a group
/// solo and a per-track solo compose instead of overwriting one another.
fn member_effective_solo(r: &Resonance, track_id: TrackId) -> bool {
    let own = r
        .registry
        .sorted_tracks()
        .iter()
        .any(|t| t.id == track_id && t.soloed);
    own || r.track_groups.is_track_soloed_via_group(track_id)
}

/// Toggle a group's macro mute and cascade the *effective* mute to every
/// member track (todo #687, doc #200).
///
/// Macro mute is non-destructive: it never writes a member's own `muted`
/// flag. Instead the engine receives each member's *effective* mute — its
/// own mute OR any containing group's macro mute — so a member stays muted
/// while the group mute holds and reverts to its own state when the group
/// mute clears. Nested-group members travel with the cascade via the
/// flattened member list.
fn toggle_macro_mute(r: &mut Resonance, group_id: TrackId) {
    if r.track_groups
        .update_group(group_id, |g| g.macro_mute = !g.macro_mute)
        .is_none()
    {
        return; // no such group — stale message after removal
    }
    for member in r.track_groups.get_all_member_ids(group_id) {
        let muted = member_effective_mute(r, member);
        let _ = r.engine.send(AudioCommand::SetTrackMute {
            track_id: member,
            muted,
        });
    }
}

/// A track's *effective* mute: its own `muted` flag OR membership in any
/// macro-muted group. This is what the audio engine must see so a group
/// mute and a per-track mute compose instead of overwriting one another.
fn member_effective_mute(r: &Resonance, track_id: TrackId) -> bool {
    let own = r
        .registry
        .sorted_tracks()
        .iter()
        .any(|t| t.id == track_id && t.muted);
    own || r.track_groups.is_track_muted_via_group(track_id)
}

// ---------------------------------------------------------------------
// Drag-and-drop membership (todo #685)
// ---------------------------------------------------------------------
//
// Direct manipulation of group membership: drag a track row onto a group
// to join it, onto open space to ungroup it; drag a group header onto
// another group to nest it (one level deep, members travelling with it),
// onto open space to un-nest it. The drag is a transient three-phase
// gesture — start (grab) → update (hover) → drop (commit) — mirroring the
// clip-drag reducers. Only the committed change touches the registry (and
// the undo history via `GroupMessage::DropMembership`); start / update /
// cancel are pure UI bookkeeping.

/// Open a membership drag. Records what is being dragged and the group it
/// currently sits in, so a later drop onto open space knows what to detach
/// from.
fn start_membership_drag(r: &mut Resonance, subject: MembershipDragSubject, cursor_y: f32) {
    let origin_group = match subject {
        MembershipDragSubject::Track(track_id) => r.track_groups.group_of_member(track_id),
        MembershipDragSubject::Group(group_id) => {
            r.track_groups.get_group(group_id).and_then(|g| g.nesting_parent)
        }
    };
    r.ui.interaction.membership_drag = Some(MembershipDragState {
        subject,
        origin_group,
        hover: None,
        cursor_y,
    });
}

/// Update the hovered drop target and pointer position of the active drag.
/// A no-op when no drag is in flight.
fn update_membership_drag(
    r: &mut Resonance,
    target: Option<MembershipDropTarget>,
    cursor_y: f32,
) {
    if let Some(drag) = r.ui.interaction.membership_drag.as_mut() {
        drag.hover = target;
        drag.cursor_y = cursor_y;
    }
}

/// Commit the active membership drag, applying the change implied by the
/// hovered target. Clears the drag either way. A no-op (leaving the
/// registry untouched) when nothing is dragging or no target is hovered —
/// so a release into empty space cancels rather than mutating.
fn drop_membership(r: &mut Resonance) {
    let Some(drag) = r.ui.interaction.membership_drag.take() else {
        return;
    };
    let Some(target) = drag.hover else {
        return;
    };
    apply_membership_change(r, drag.subject, drag.origin_group, target);
}

/// Apply a resolved membership change to the registry. Pure registry
/// mutation; the caller owns clearing the drag state.
fn apply_membership_change(
    r: &mut Resonance,
    subject: MembershipDragSubject,
    origin_group: Option<TrackId>,
    target: MembershipDropTarget,
) {
    match (subject, target) {
        // Track joins a group: detach from its current group first so a
        // track is never listed under two groups at once.
        (MembershipDragSubject::Track(track_id), MembershipDropTarget::IntoGroup(dest)) => {
            // Ignore a drop onto a non-group id, or back onto the same group.
            if r.track_groups.get_group(dest).is_none() || origin_group == Some(dest) {
                return;
            }
            if let Some(from) = origin_group {
                r.track_groups.remove_member(from, track_id);
            }
            r.track_groups.add_member(dest, track_id);
        }
        // Track dropped on open space: leave whatever group it was in.
        (MembershipDragSubject::Track(track_id), MembershipDropTarget::Ungrouped) => {
            if let Some(from) = origin_group {
                r.track_groups.remove_member(from, track_id);
            }
        }
        // Group nests under another group (members travel with it). Refuse
        // self-nesting, dropping onto a non-group, and nesting a group that
        // is itself a parent (which would push its child two levels deep).
        // `set_nesting_parent` independently rejects a parent that already
        // has a parent, closing the cycle.
        (MembershipDragSubject::Group(group_id), MembershipDropTarget::IntoGroup(dest)) => {
            if group_id == dest
                || r.track_groups.get_group(dest).is_none()
                || r.track_groups.is_parent_group(group_id)
            {
                return;
            }
            r.track_groups.set_nesting_parent(group_id, Some(dest));
        }
        // Group dropped on open space: detach it back to the top level.
        (MembershipDragSubject::Group(group_id), MembershipDropTarget::Ungrouped) => {
            r.track_groups.set_nesting_parent(group_id, None);
        }
    }
}
