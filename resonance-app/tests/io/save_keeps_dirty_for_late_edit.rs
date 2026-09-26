//! A save only cleans what it captured (code review STATE-09).
//!
//! The project JSON, MIDI and plugin blobs are captured when both engine
//! branches of the save have reported (`try_finish_save`); the files are
//! then written asynchronously, with an fsync per file. An edit landing
//! between that capture and `ProjectSaved(Ok)` is not in the files, yet the
//! completion cleared `dirty` unconditionally — so closing asked nothing and
//! the edit was lost. The completion now clears `dirty` only when no edit
//! happened since the capture.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use resonance_app::message::{Message, ProjectIoMessage, TrackMessage};
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, TrackType};

const TRACK: u64 = 1;

/// A per-test project dir, with the user config pointed away from the
/// developer's real `recent.json` (the save completion adds to recents).
fn project_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "resonance_save_dirty_{tag}_{}_{n}",
        std::process::id()
    ));
    std::env::set_var("XDG_CONFIG_HOME", root.join("config"));
    let dir = root.join("song.rproj");
    std::fs::create_dir_all(&dir).expect("create project dir");
    dir
}

/// An app mid-save: the save has started and both engine branches have
/// reported, so its contents are captured and the write is in flight.
fn app_with_captured_save(tag: &str) -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(project_dir(tag));
    app.test_add_track(TRACK, TrackType::Audio);
    let _ = app.update(Message::Track(TrackMessage::SetTrackVolume(TRACK, -3.0)));
    assert!(app.is_dirty());

    let _ = app.update(Message::ProjectIo(ProjectIoMessage::SaveProject));
    app.test_apply_engine_event(AudioEvent::ClipsSavedToProjectDir { clip_files: Vec::new() });
    app.test_apply_engine_event(AudioEvent::AllPluginStatesSaved { states: Vec::new() });
    app
}

#[test]
fn an_edit_during_the_write_keeps_the_project_dirty() {
    let mut app = app_with_captured_save("late_edit");

    let _ = app.update(Message::Track(TrackMessage::SetTrackVolume(TRACK, -9.0)));
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectSaved(Ok(()), false)));

    assert!(
        app.is_dirty(),
        "the edit after the capture is not on disk, so the project stays dirty"
    );
}

#[test]
fn a_save_with_no_edit_during_the_write_cleans_the_project() {
    let mut app = app_with_captured_save("no_edit");

    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectSaved(Ok(()), false)));

    assert!(!app.is_dirty(), "everything was captured, so the save cleans");
}
