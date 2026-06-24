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
//! The collapse / fold state (todo #686) and group level trim (todo #689)
//! are implemented below. Until the remaining macro-mute variant lands its
//! control routes to a no-op so the header is wired end-to-end (and
//! snapshot-testable) without pre-empting the reducer todos.
//!
//! Two reducer families *are* implemented here: group creation from a
//! multi-track selection (todo #684) and drag-and-drop membership editing
//! (todo #685 — see the "Drag-and-drop membership" section below).

use iced::Task;

use crate::message::{GroupMessage, Message};
use crate::state::{MembershipDragState, MembershipDragSubject, MembershipDropTarget};
use crate::Resonance;
use resonance_audio::types::{AudioCommand, TrackId};

pub fn handle(r: &mut Resonance, m: GroupMessage) -> Task<Message> {
    match m {
        // Collapse / fold state in the timeline — todo #686.
        GroupMessage::ToggleCollapse(group_id) => toggle_collapse(r, group_id),
        // Macro solo implemented (todo #688). Macro mute implemented in
        // todo #687. Macro level trim below (todo #689).
        GroupMessage::ToggleMacroMute(_) => {}
        GroupMessage::ToggleMacroSolo(group_id) => toggle_macro_solo(r, group_id),
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
            r.interaction.membership_drag = None;
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
    let members = r.interaction.selected_tracks.clone();
    if members.len() < 2 {
        return;
    }
    let group_id = r.registry.allocate_sub_track_id();
    r.track_groups.create_group_from_selection(group_id, &members);
    r.interaction.select_single_track(None);
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
    r.interaction.membership_drag = Some(MembershipDragState {
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
    if let Some(drag) = r.interaction.membership_drag.as_mut() {
        drag.hover = target;
        drag.cursor_y = cursor_y;
    }
}

/// Commit the active membership drag, applying the change implied by the
/// hovered target. Clears the drag either way. A no-op (leaving the
/// registry untouched) when nothing is dragging or no target is hovered —
/// so a release into empty space cancels rather than mutating.
fn drop_membership(r: &mut Resonance) {
    let Some(drag) = r.interaction.membership_drag.take() else {
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
