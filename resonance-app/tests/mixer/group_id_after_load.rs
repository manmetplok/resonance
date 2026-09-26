//! Group ids share the app's sub-track id counter, which a fresh session
//! starts at 1_000_000_000. A project load must advance that counter past
//! the loaded group ids too, or the next Cmd-G reuses a saved group's id
//! and silently replaces it (code review STATE-04).

use resonance_app::message::{GroupMessage, Message};
use resonance_app::project::ProjectFile;
use resonance_app::state::TrackState;
use resonance_app::Resonance;
use resonance_common::group_identity::GroupIdentityColor;
use resonance_common::track_group::TrackGroup;

const SAVED_GROUP: u64 = 1_000_000_000;

#[test]
fn new_group_after_load_does_not_reuse_a_loaded_group_id() {
    let (mut app, _task) = Resonance::new_for_test();
    let mut saved = TrackGroup::new(SAVED_GROUP, "Drums", GroupIdentityColor::Drum);
    saved.ordered_members = vec![1, 2];
    saved.macro_mute = true;
    app.test_replay_loaded_project(ProjectFile {
        track_groups: vec![saved.clone()],
        ..ProjectFile::default()
    });
    app.test_set_active_project(true);

    app.test_push_track(TrackState::new_instrument(3, 3));
    app.test_push_track(TrackState::new_instrument(4, 4));
    app.test_set_selected_tracks(vec![3, 4]);
    let _ = app.update(Message::Group(GroupMessage::CreateGroupFromSelection));

    let groups = app.test_track_groups();
    assert_eq!(groups.len(), 2, "the new group must not replace the loaded one");
    assert_eq!(groups.get_group(SAVED_GROUP), Some(&saved));
    let fresh = groups
        .get_all_groups()
        .into_iter()
        .find(|g| g.id != SAVED_GROUP)
        .expect("the new group");
    assert_eq!(fresh.ordered_members, vec![3, 4]);
}
