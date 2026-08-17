//! Drag-and-drop track-group membership (epic #36, doc #200, todo #685).
//!
//! Exercises the reducer half of direct-manipulation membership editing
//! through `Resonance::update`: a track row or group header is grabbed
//! (`StartMembershipDrag`), dragged over a candidate target
//! (`UpdateMembershipDrag`) and released (`DropMembership`), and the group
//! registry's membership / nesting changes accordingly. The drag is purely
//! transient — only the drop mutates the registry — and dropping onto open
//! space ungroups / un-nests while dropping onto nothing (or cancelling)
//! leaves the grouping untouched. One level of group nesting is allowed;
//! deeper nesting is refused.

use resonance_app::message::{GroupMessage, Message};
use resonance_app::state::{MembershipDragSubject, MembershipDropTarget};
use resonance_app::Resonance;
use resonance_common::automation::TrackId;

fn active_app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app
}

/// Fold `members` into a fresh group and return its allocated id. The id is
/// drawn from the high sub-track-id space, so it never clashes with the
/// small track ids the tests use as members.
fn make_group(app: &mut Resonance, members: &[TrackId]) -> TrackId {
    let before: Vec<TrackId> = app.test_track_groups().group_ids().collect();
    app.test_set_selected_tracks(members.to_vec());
    let _ = app.update(Message::Group(GroupMessage::CreateGroupFromSelection));
    app.test_track_groups()
        .group_ids()
        .find(|id| !before.contains(id))
        .expect("a new group should have been created")
}

/// Drive a full grab → hover → drop gesture in one shot.
fn drag_drop(app: &mut Resonance, subject: MembershipDragSubject, target: MembershipDropTarget) {
    let _ = app.update(Message::Group(GroupMessage::StartMembershipDrag(subject, 0.0)));
    let _ = app.update(Message::Group(GroupMessage::UpdateMembershipDrag {
        target: Some(target),
        cursor_y: 0.0,
    }));
    let _ = app.update(Message::Group(GroupMessage::DropMembership));
}

fn members(app: &Resonance, group: TrackId) -> Vec<TrackId> {
    app.test_track_groups()
        .get_group(group)
        .map(|g| g.ordered_members.clone())
        .unwrap_or_default()
}

fn nesting_parent(app: &Resonance, group: TrackId) -> Option<TrackId> {
    app.test_track_groups()
        .get_group(group)
        .and_then(|g| g.nesting_parent)
}

// ---- Track membership ---------------------------------------------------

#[test]
fn drag_track_onto_group_joins_it() {
    let mut app = active_app();
    let g = make_group(&mut app, &[10, 11]);

    drag_drop(
        &mut app,
        MembershipDragSubject::Track(20),
        MembershipDropTarget::IntoGroup(g),
    );

    assert_eq!(members(&app, g), vec![10, 11, 20]);
    // The transient drag state is consumed by the drop.
    assert!(app.test_membership_drag().is_none());
}

#[test]
fn drag_member_to_open_space_ungroups_it() {
    let mut app = active_app();
    let g = make_group(&mut app, &[10, 11, 12]);

    drag_drop(
        &mut app,
        MembershipDragSubject::Track(11),
        MembershipDropTarget::Ungrouped,
    );

    assert_eq!(members(&app, g), vec![10, 12]);
}

#[test]
fn drag_track_between_groups_moves_membership() {
    let mut app = active_app();
    let g1 = make_group(&mut app, &[10, 11]);
    let g2 = make_group(&mut app, &[20, 21]);

    // 10 starts in g1; drag it into g2.
    drag_drop(
        &mut app,
        MembershipDragSubject::Track(10),
        MembershipDropTarget::IntoGroup(g2),
    );

    // It left g1 and is never listed under two groups at once.
    assert_eq!(members(&app, g1), vec![11]);
    assert_eq!(members(&app, g2), vec![20, 21, 10]);
}

#[test]
fn drag_member_back_onto_its_own_group_is_a_noop() {
    let mut app = active_app();
    let g = make_group(&mut app, &[10, 11]);

    drag_drop(
        &mut app,
        MembershipDragSubject::Track(10),
        MembershipDropTarget::IntoGroup(g),
    );

    // No duplicate, original order preserved.
    assert_eq!(members(&app, g), vec![10, 11]);
}

// ---- Group nesting ------------------------------------------------------

#[test]
fn drag_group_onto_group_nests_one_level_members_travel() {
    let mut app = active_app();
    let parent = make_group(&mut app, &[10, 11]);
    let child = make_group(&mut app, &[20, 21]);

    drag_drop(
        &mut app,
        MembershipDragSubject::Group(child),
        MembershipDropTarget::IntoGroup(parent),
    );

    assert_eq!(nesting_parent(&app, child), Some(parent));
    // The child's own members are untouched — they travel with it.
    assert_eq!(members(&app, child), vec![20, 21]);
    assert!(app.test_track_groups().validate_nesting());
}

#[test]
fn drag_nested_group_to_open_space_un_nests_it() {
    let mut app = active_app();
    let parent = make_group(&mut app, &[10, 11]);
    let child = make_group(&mut app, &[20, 21]);
    drag_drop(
        &mut app,
        MembershipDragSubject::Group(child),
        MembershipDropTarget::IntoGroup(parent),
    );
    assert_eq!(nesting_parent(&app, child), Some(parent));

    drag_drop(
        &mut app,
        MembershipDragSubject::Group(child),
        MembershipDropTarget::Ungrouped,
    );

    assert_eq!(nesting_parent(&app, child), None);
}

#[test]
fn two_level_nesting_is_refused() {
    let mut app = active_app();
    let parent = make_group(&mut app, &[10, 11]);
    let child = make_group(&mut app, &[20, 21]);
    let grandchild = make_group(&mut app, &[30, 31]);

    // child nests under parent (allowed, one level).
    drag_drop(
        &mut app,
        MembershipDragSubject::Group(child),
        MembershipDropTarget::IntoGroup(parent),
    );

    // Trying to nest grandchild under the already-nested child would be two
    // levels deep — refused, leaving grandchild at the top level.
    drag_drop(
        &mut app,
        MembershipDragSubject::Group(grandchild),
        MembershipDropTarget::IntoGroup(child),
    );
    assert_eq!(nesting_parent(&app, grandchild), None);

    // And nesting the parent (which already holds a child) under another
    // group would push its child two levels deep — also refused.
    let other = make_group(&mut app, &[40, 41]);
    drag_drop(
        &mut app,
        MembershipDragSubject::Group(parent),
        MembershipDropTarget::IntoGroup(other),
    );
    assert_eq!(nesting_parent(&app, parent), None);
    assert!(app.test_track_groups().validate_nesting());
}

#[test]
fn group_cannot_nest_into_itself() {
    let mut app = active_app();
    let g = make_group(&mut app, &[10, 11]);

    drag_drop(
        &mut app,
        MembershipDragSubject::Group(g),
        MembershipDropTarget::IntoGroup(g),
    );

    assert_eq!(nesting_parent(&app, g), None);
}

// ---- Drag lifecycle: transient, cancellable -----------------------------

#[test]
fn start_opens_drag_with_resolved_origin_group() {
    let mut app = active_app();
    let g = make_group(&mut app, &[10, 11]);

    let _ = app.update(Message::Group(GroupMessage::StartMembershipDrag(
        MembershipDragSubject::Track(10),
        42.0,
    )));

    let drag = app.test_membership_drag().expect("drag should be open");
    assert_eq!(drag.subject, MembershipDragSubject::Track(10));
    assert_eq!(drag.origin_group, Some(g));
    assert_eq!(drag.cursor_y, 42.0);
    assert!(drag.hover.is_none());
}

#[test]
fn cancel_leaves_membership_untouched() {
    let mut app = active_app();
    let g = make_group(&mut app, &[10, 11]);

    let _ = app.update(Message::Group(GroupMessage::StartMembershipDrag(
        MembershipDragSubject::Track(20),
        0.0,
    )));
    let _ = app.update(Message::Group(GroupMessage::UpdateMembershipDrag {
        target: Some(MembershipDropTarget::IntoGroup(g)),
        cursor_y: 0.0,
    }));
    let _ = app.update(Message::Group(GroupMessage::CancelMembershipDrag));

    assert!(app.test_membership_drag().is_none());
    assert_eq!(members(&app, g), vec![10, 11]);
}

#[test]
fn drop_without_a_target_is_a_noop() {
    let mut app = active_app();
    let g = make_group(&mut app, &[10, 11]);

    // Grab a member but release over nothing droppable (no hover set).
    let _ = app.update(Message::Group(GroupMessage::StartMembershipDrag(
        MembershipDragSubject::Track(10),
        0.0,
    )));
    let _ = app.update(Message::Group(GroupMessage::DropMembership));

    assert!(app.test_membership_drag().is_none());
    assert_eq!(members(&app, g), vec![10, 11]);
}

#[test]
fn drop_with_no_active_drag_is_inert() {
    let mut app = active_app();
    let g = make_group(&mut app, &[10, 11]);

    // A stray drop with nothing grabbed must not panic or mutate.
    let _ = app.update(Message::Group(GroupMessage::DropMembership));

    assert_eq!(members(&app, g), vec![10, 11]);
}

// ---- Undo classification ------------------------------------------------

#[test]
fn drop_is_recorded_for_undo_but_drag_phases_are_not() {
    use resonance_app::undo::{classify, UndoAction};

    assert!(matches!(
        classify(&Message::Group(GroupMessage::DropMembership)),
        UndoAction::Record
    ));
    assert!(matches!(
        classify(&Message::Group(GroupMessage::StartMembershipDrag(
            MembershipDragSubject::Track(1),
            0.0
        ))),
        UndoAction::Skip
    ));
    assert!(matches!(
        classify(&Message::Group(GroupMessage::UpdateMembershipDrag {
            target: None,
            cursor_y: 0.0
        })),
        UndoAction::Skip
    ));
    assert!(matches!(
        classify(&Message::Group(GroupMessage::CancelMembershipDrag)),
        UndoAction::Skip
    ));
}
