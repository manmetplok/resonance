//! Group macro solo with cascade to members (epic #36, doc #200, todo
//! #688).
//!
//! Two layers are pinned here:
//!   * the registry query
//!     ([`TrackGroupRegistry::is_track_soloed_via_group`]) — a track is
//!     "soloed via group" when any group it belongs to (directly or one
//!     level of nesting deep) has its macro solo engaged; and
//!   * the reducer path through `Resonance::update`: toggling a group's
//!     macro solo flips the persisted `macro_solo` flag and marks every
//!     member soloed-via-group, while leaving each member's *own* solo
//!     untouched (the cascade is non-destructive), and toggling again
//!     clears it.

use resonance_app::message::{GroupMessage, Message};
use resonance_app::state::{TrackGroupRegistry, TrackState};
use resonance_app::Resonance;
use resonance_common::automation::TrackId;
use resonance_common::group_identity::GroupIdentityColor;

// ---- Registry query -----------------------------------------------------

#[test]
fn macro_solo_marks_direct_members_via_group() {
    let mut registry = TrackGroupRegistry::new();
    let g = registry.add_group_new(100, "Group 1", GroupIdentityColor::Drum);
    registry.add_member(g, 1);
    registry.add_member(g, 2);

    // Off by default: membership alone does not solo a track.
    assert!(!registry.is_track_soloed_via_group(1));
    assert!(!registry.is_track_soloed_via_group(2));

    registry.get_group_mut(g).unwrap().macro_solo = true;

    // Now both members read as soloed-via-group, but a non-member does not.
    assert!(registry.is_track_soloed_via_group(1));
    assert!(registry.is_track_soloed_via_group(2));
    assert!(!registry.is_track_soloed_via_group(999));
}

#[test]
fn macro_solo_cascades_through_one_level_of_nesting() {
    let mut registry = TrackGroupRegistry::new();
    let parent = registry.add_group_new(100, "Parent", GroupIdentityColor::Drum);
    let child = registry.add_group_new(101, "Child", GroupIdentityColor::Vocal);
    // Child group nests under the parent; a leaf track lives in the child.
    registry.add_member(parent, child);
    registry.add_member(child, 1);
    registry.set_nesting_parent(child, Some(parent));

    // Soloing the *parent* cascades to the nested child's member.
    registry.get_group_mut(parent).unwrap().macro_solo = true;
    assert!(registry.is_track_soloed_via_group(1));
}

// ---- Reducer path through Resonance::update -----------------------------

fn active_app() -> Resonance {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app
}

fn audio_track(id: TrackId, soloed: bool) -> TrackState {
    let mut t = TrackState::new_audio(id, id as usize);
    t.soloed = soloed;
    t
}

fn track_own_solo(app: &Resonance, id: TrackId) -> bool {
    app.test_registry()
        .sorted_tracks()
        .iter()
        .find(|t| t.id == id)
        .map(|t| t.soloed)
        .expect("track exists")
}

/// Build a two-track group through the public reducer path and return its
/// id. Members keep whatever own-solo state they were pushed with.
fn group_of(app: &mut Resonance, a: TrackState, b: TrackState) -> TrackId {
    let (ia, ib) = (a.id, b.id);
    app.test_push_track(a);
    app.test_push_track(b);
    app.test_set_selected_tracks(vec![ia, ib]);
    let _ = app.update(Message::Group(GroupMessage::CreateGroupFromSelection));
    app.test_track_groups().get_all_groups()[0].id
}

#[test]
fn toggle_macro_solo_sets_flag_and_cascades() {
    let mut app = active_app();
    let gid = group_of(&mut app, audio_track(10, false), audio_track(11, false));

    let _ = app.update(Message::Group(GroupMessage::ToggleMacroSolo(gid)));

    let groups = app.test_track_groups();
    assert!(groups.get_group(gid).unwrap().macro_solo);
    assert!(groups.is_track_soloed_via_group(10));
    assert!(groups.is_track_soloed_via_group(11));

    // The cascade is non-destructive: members' own solo stays off.
    assert!(!track_own_solo(&app, 10));
    assert!(!track_own_solo(&app, 11));
}

#[test]
fn toggle_macro_solo_twice_clears_the_cascade() {
    let mut app = active_app();
    let gid = group_of(&mut app, audio_track(10, false), audio_track(11, false));

    let _ = app.update(Message::Group(GroupMessage::ToggleMacroSolo(gid)));
    let _ = app.update(Message::Group(GroupMessage::ToggleMacroSolo(gid)));

    let groups = app.test_track_groups();
    assert!(!groups.get_group(gid).unwrap().macro_solo);
    assert!(!groups.is_track_soloed_via_group(10));
    assert!(!groups.is_track_soloed_via_group(11));
}

#[test]
fn macro_solo_leaves_a_members_own_solo_independent() {
    let mut app = active_app();
    // Track 20 is soloed by the user before any group solo is engaged.
    let gid = group_of(&mut app, audio_track(20, true), audio_track(21, false));

    // Engage then release the group's macro solo.
    let _ = app.update(Message::Group(GroupMessage::ToggleMacroSolo(gid)));
    let _ = app.update(Message::Group(GroupMessage::ToggleMacroSolo(gid)));

    // The macro cascade is gone, but track 20's own solo survived it.
    assert!(!app.test_track_groups().is_track_soloed_via_group(20));
    assert!(track_own_solo(&app, 20));
    assert!(!track_own_solo(&app, 21));
}

#[test]
fn toggle_macro_solo_on_missing_group_is_a_noop() {
    let mut app = active_app();
    // A stale message for an id that is not a group must not panic.
    let _ = app.update(Message::Group(GroupMessage::ToggleMacroSolo(424242)));
    assert!(app.test_track_groups().is_empty());
}
