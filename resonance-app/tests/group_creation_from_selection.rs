//! "Group selected" action: multi-track selection + group creation
//! (epic #36, doc #200, todo #684).
//!
//! Two layers are pinned here:
//!   * the registry policy ([`TrackGroupRegistry::create_group_from_selection`])
//!     — auto-named, colour-cycled, members in click order; and
//!   * the reducer path through `Resonance::update`: additive (Shift/Cmd)
//!     clicks build a multi-selection, `CreateGroupFromSelection` folds it
//!     into a group and clears the selection, and a sub-two selection is a
//!     no-op.

use iced::keyboard::Modifiers;
use resonance_app::message::{GroupMessage, Message, UiMessage};
use resonance_app::state::TrackGroupRegistry;
use resonance_app::Resonance;
use resonance_common::automation::TrackId;
use resonance_common::group_identity::GroupIdentityColor;

fn track_id(i: u64) -> TrackId {
    i
}

// ---- Registry policy ----------------------------------------------------

#[test]
fn create_group_from_selection_names_and_colours_by_count() {
    let mut registry = TrackGroupRegistry::new();

    let g1 = registry.create_group_from_selection(track_id(100), &[track_id(1), track_id(2)]);
    let g2 = registry.create_group_from_selection(track_id(101), &[track_id(3), track_id(4)]);

    let first = registry.get_group(g1).unwrap();
    assert_eq!(first.name, "Group 1");
    assert_eq!(first.identity_color, GroupIdentityColor::Drum);

    let second = registry.get_group(g2).unwrap();
    assert_eq!(second.name, "Group 2");
    // Colour cycles to the next palette entry for the second group.
    assert_eq!(second.identity_color, GroupIdentityColor::Vocal);
}

#[test]
fn create_group_from_selection_keeps_member_order() {
    let mut registry = TrackGroupRegistry::new();
    let members = [track_id(7), track_id(3), track_id(9)];

    let id = registry.create_group_from_selection(track_id(200), &members);

    let group = registry.get_group(id).unwrap();
    assert_eq!(group.ordered_members, members);
}

#[test]
fn create_group_from_selection_dedupes_members() {
    let mut registry = TrackGroupRegistry::new();

    let id = registry.create_group_from_selection(
        track_id(200),
        &[track_id(5), track_id(5), track_id(6)],
    );

    let group = registry.get_group(id).unwrap();
    assert_eq!(group.ordered_members, vec![track_id(5), track_id(6)]);
}

// ---- Reducer path through Resonance::update -----------------------------

fn active_app() -> Resonance {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app
}

fn select(app: &mut Resonance, id: TrackId) {
    let _ = app.update(Message::Ui(UiMessage::SelectTrack(Some(id))));
}

fn set_modifiers(app: &mut Resonance, mods: Modifiers) {
    let _ = app.update(Message::Ui(UiMessage::ModifiersChanged(mods)));
}

#[test]
fn additive_clicks_build_multi_selection() {
    let mut app = active_app();

    // A plain click selects exactly one track.
    select(&mut app, 1);
    assert_eq!(app.test_selected_tracks(), &[1]);

    // With a modifier held, further clicks extend the selection.
    set_modifiers(&mut app, Modifiers::SHIFT);
    select(&mut app, 2);
    select(&mut app, 3);
    assert_eq!(app.test_selected_tracks(), &[1, 2, 3]);

    // Re-clicking a selected track (still additive) toggles it back off.
    select(&mut app, 2);
    assert_eq!(app.test_selected_tracks(), &[1, 3]);

    // Releasing the modifier and clicking replaces the whole selection.
    set_modifiers(&mut app, Modifiers::empty());
    select(&mut app, 5);
    assert_eq!(app.test_selected_tracks(), &[5]);
}

#[test]
fn create_group_from_selection_groups_and_clears() {
    let mut app = active_app();

    set_modifiers(&mut app, Modifiers::SHIFT);
    select(&mut app, 10);
    select(&mut app, 11);
    select(&mut app, 12);
    assert_eq!(app.test_selected_tracks().len(), 3);

    let _ = app.update(Message::Group(GroupMessage::CreateGroupFromSelection));

    // Exactly one group, holding the three selected tracks.
    let groups = app.test_track_groups();
    assert_eq!(groups.len(), 1);
    let group = groups.get_all_groups()[0];
    assert_eq!(group.ordered_members, vec![10, 11, 12]);

    // The selection is cleared so the floating bar dismisses itself.
    assert!(app.test_selected_tracks().is_empty());
}

#[test]
fn create_group_from_selection_noops_below_two_tracks() {
    let mut app = active_app();

    select(&mut app, 42);
    assert_eq!(app.test_selected_tracks(), &[42]);

    let _ = app.update(Message::Group(GroupMessage::CreateGroupFromSelection));

    // No group created, and the single selection is left untouched.
    assert_eq!(app.test_track_groups().len(), 0);
    assert_eq!(app.test_selected_tracks(), &[42]);
}
