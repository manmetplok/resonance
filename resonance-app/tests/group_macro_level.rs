//! Group level trim with macro scaling (epic #36, doc #200, todo #689).
//!
//! The group header's compact trim slider emits
//! `GroupMessage::SetMacroLevel`, whose reducer writes the multiplicative
//! macro gain onto the persisted group. The trim *scales* members'
//! contribution (`member_volume * group.macro_level`); it never overwrites
//! the members' own faders, so returning the trim to unity restores each
//! member exactly. Because the slider is a continuous control, its message
//! burst coalesces into a single undo entry keyed by group — mirroring how a
//! track fader drag is recorded.

use resonance_app::message::{GroupMessage, Message};
use resonance_app::state::TrackState;
use resonance_app::undo::{classify, CoalesceKey, UndoAction};
use resonance_app::Resonance;
use resonance_common::automation::TrackId;
use resonance_common::track_group::MACRO_LEVEL_UNITY;

fn active_app() -> Resonance {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app
}

/// Fold `members` into a fresh group and return its allocated id.
fn make_group(app: &mut Resonance, members: &[TrackId]) -> TrackId {
    let before: Vec<TrackId> = app.test_track_groups().group_ids().collect();
    app.test_set_selected_tracks(members.to_vec());
    let _ = app.update(Message::Group(GroupMessage::CreateGroupFromSelection));
    app.test_track_groups()
        .group_ids()
        .find(|id| !before.contains(id))
        .expect("a new group should have been created")
}

fn macro_level(app: &Resonance, group: TrackId) -> f32 {
    app.test_track_groups()
        .get_group(group)
        .map(|g| g.macro_level)
        .expect("group should exist")
}

fn set_level(app: &mut Resonance, group: TrackId, level: f32) {
    let _ = app.update(Message::Group(GroupMessage::SetMacroLevel(group, level)));
}

// ---- Reducer ------------------------------------------------------------

#[test]
fn fresh_group_starts_at_unity() {
    let mut app = active_app();
    let g = make_group(&mut app, &[10, 11]);
    assert_eq!(macro_level(&app, g), MACRO_LEVEL_UNITY);
}

#[test]
fn set_macro_level_writes_the_group_gain() {
    let mut app = active_app();
    let g = make_group(&mut app, &[10, 11]);

    set_level(&mut app, g, 1.5);
    assert_eq!(macro_level(&app, g), 1.5);

    // A later adjustment overwrites, not accumulates.
    set_level(&mut app, g, 0.25);
    assert_eq!(macro_level(&app, g), 0.25);
}

#[test]
fn set_macro_level_does_not_touch_member_faders() {
    let mut app = active_app();
    // Real member tracks with distinct fader values.
    let mut t10 = TrackState::new_audio(10, 0);
    t10.volume = -6.0;
    let mut t11 = TrackState::new_audio(11, 1);
    t11.volume = 3.0;
    app.test_push_track(t10);
    app.test_push_track(t11);

    let g = make_group(&mut app, &[10, 11]);
    set_level(&mut app, g, 0.5);

    // The group gain is the only thing that moved — the per-track faders are
    // left exactly as the user set them (the trim scales, never overwrites).
    let faders: Vec<f32> = app
        .test_registry()
        .sorted_tracks()
        .iter()
        .filter(|t| t.id == 10 || t.id == 11)
        .map(|t| t.volume)
        .collect();
    assert_eq!(faders, vec![-6.0, 3.0]);
}

#[test]
fn set_macro_level_on_unknown_group_is_inert() {
    let mut app = active_app();
    let g = make_group(&mut app, &[10, 11]);

    // A stray id (no such group) must not panic or disturb the real group.
    set_level(&mut app, 99_999, 0.3);
    assert_eq!(macro_level(&app, g), MACRO_LEVEL_UNITY);
}

// ---- Undo ---------------------------------------------------------------

#[test]
fn set_macro_level_coalesces_per_group_for_undo() {
    // A continuous slider drag collapses into one entry, keyed by group so
    // two groups' trims never coalesce into each other.
    assert!(matches!(
        classify(&Message::Group(GroupMessage::SetMacroLevel(7, 0.5))),
        UndoAction::RecordCoalesced(CoalesceKey::GroupMacroLevel(7))
    ));
    assert!(matches!(
        classify(&Message::Group(GroupMessage::SetMacroLevel(8, 0.5))),
        UndoAction::RecordCoalesced(CoalesceKey::GroupMacroLevel(8))
    ));
}
