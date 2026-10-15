//! A save collector takes only the engine replies to its own requests
//! (BUG-61).
//!
//! Saves are serialized, but the engine's `ClipsSavedToProjectDir` /
//! `AllPluginStatesSaved` carried no tag, so whatever collector was
//! current took them: a reply that arrived after the watchdog abandoned
//! its save completed the next one with the old save's clip list and
//! plugin blobs. Each save now sends a request id the engine echoes, and
//! a reply carrying another id is left alone.

use resonance_app::message::{Message, ProjectIoMessage};
use resonance_app::Resonance;
use resonance_audio::types::{AudioCommand, AudioEvent};

fn dispatch(app: &mut Resonance, m: ProjectIoMessage) {
    let _ = app.update(Message::ProjectIo(m));
}

/// The request ids the save sent with its two engine commands.
fn sent_requests(cmds: &resonance_audio::test_support::Receiver<AudioCommand>) -> Vec<u64> {
    cmds.try_iter()
        .filter_map(|c| match c {
            AudioCommand::SaveClipsToProjectDir { request }
            | AudioCommand::SaveAllPluginStates { request } => Some(request),
            _ => None,
        })
        .collect()
}

#[test]
fn a_late_reply_to_an_abandoned_save_does_not_complete_the_next_one() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (mut app, _task, cmds) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    app.test_set_project_path(dir.path().join("song.rproj"));

    dispatch(&mut app, ProjectIoMessage::SaveProject);
    let first = app.test_save_request();
    assert_eq!(sent_requests(&cmds), vec![first, first], "one id per save, on both commands");
    // The engine never answers in time: the watchdog abandons the save.
    app.test_age_save_in_flight(std::time::Duration::from_secs(10 * 60));
    let _ = app.update(Message::Tick);
    assert_eq!(app.test_save_in_flight(), None);

    // The user saves again; the first save's replies only now land.
    dispatch(&mut app, ProjectIoMessage::SaveProject);
    let second = app.test_save_request();
    assert_ne!(first, second);
    app.test_apply_engine_event(AudioEvent::ClipsSavedToProjectDir {
        request: first,
        clip_files: Vec::new(),
    });
    app.test_apply_engine_event(AudioEvent::AllPluginStatesSaved {
        request: first,
        states: vec![(9, vec![0xDE, 0xAD])],
    });
    assert!(
        app.test_save_in_flight().is_some(),
        "the second save still waits for its own replies"
    );
    // The stale blob still refreshed the undo cache: it is the plugin's
    // live state, whoever asked for it.
    assert_eq!(app.test_cached_plugin_state(9), Some(vec![0xDE, 0xAD]));

    // Its own replies complete it.
    app.test_apply_engine_event(AudioEvent::ClipsSavedToProjectDir {
        request: second,
        clip_files: Vec::new(),
    });
    app.test_apply_engine_event(AudioEvent::AllPluginStatesSaved {
        request: second,
        states: Vec::new(),
    });
    assert_eq!(app.test_save_in_flight(), None, "the second save took its own replies");
}

/// A late `ClipsSaveFailed` from an abandoned save must not fail the next.
#[test]
fn a_late_clip_failure_does_not_fail_the_next_save() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_set_project_path(dir.path().join("song.rproj"));

    dispatch(&mut app, ProjectIoMessage::SaveProject);
    let first = app.test_save_request();
    app.test_age_save_in_flight(std::time::Duration::from_secs(10 * 60));
    let _ = app.update(Message::Tick);
    let _ = app.update(Message::Ui(resonance_app::message::UiMessage::DismissError));

    dispatch(&mut app, ProjectIoMessage::SaveProject);
    app.test_apply_engine_event(AudioEvent::ClipsSaveFailed {
        request: first,
        error: "No space left on device".to_owned(),
    });
    app.test_apply_engine_event(AudioEvent::AllPluginStatesSaved {
        request: first,
        states: Vec::new(),
    });
    assert!(app.test_save_in_flight().is_some(), "the second save is untouched");
    assert!(app.test_error_message().is_none(), "{:?}", app.test_error_message());
}
