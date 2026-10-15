//! Save As onto a different existing project replaces it, after asking
//! (BUG-62).
//!
//! Saving over another `.rproj` used to write the new project into it and
//! keep everything else that was there: its `backups/` (which the clip GC
//! then counted as references, keeping its clips alive), its autosave, and
//! its old media. Now the GUI asks first ("Replace project X? Its history
//! and media will be deleted."), and on confirm the target's history and
//! media are cleared before the save; Cancel saves nothing. The control
//! API's `confirm: true` is that answer.

use std::path::{Path, PathBuf};

use resonance_app::message::{ClipMessage, Message, ProjectIoMessage};
use resonance_app::state::Overlay;
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, ClipId, TrackType};
use resonance_control::{ErrorKind, Request};
use serde_json::json;

use crate::common::{roundtrip, run_task};

/// What the replaced project held besides its `project.json`.
const HISTORY_AND_MEDIA: &[&str] = &[
    "project.autosave.json",
    "autosave/midi/clip_2.mid",
    "backups/project-2026-01-01T00:00:00Z.json",
    "audio/clip_1.wav",
    "midi/clip_2.mid",
    "plugins/plugin_3.bin",
];

/// A project bundle on disk with a history and media of its own, and a
/// file of the user's that is not the project's.
fn other_project(root: &Path, name: &str) -> PathBuf {
    let b = root.join(name);
    for dir in ["audio", "midi", "plugins", "backups", "autosave/midi"] {
        std::fs::create_dir_all(b.join(dir)).unwrap();
    }
    std::fs::write(
        b.join("project.json"),
        br#"{"clips":[{"id":1,"audio_file":"audio/clip_1.wav"}]}"#,
    )
    .unwrap();
    for file in HISTORY_AND_MEDIA {
        std::fs::write(b.join(file), b"old").unwrap();
    }
    std::fs::write(b.join("notes.txt"), b"mine").unwrap();
    b
}

fn imported(clip_id: ClipId) -> AudioEvent {
    AudioEvent::ClipImported {
        clip_id,
        track_id: 1,
        start_sample: 0,
        duration_samples: 48_000,
        name: format!("clip {clip_id}"),
        waveform_peaks: Vec::new(),
    }
}

/// A titled project at `root/A.rproj` whose undo history alone names clip
/// 7 (imported, then deleted).
fn titled_app(root: &Path) -> (Resonance, PathBuf) {
    let a = root.join("A.rproj");
    std::fs::create_dir_all(a.join("audio")).unwrap();
    std::fs::write(a.join("audio/clip_7.wav"), b"RIFF seven").unwrap();
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_set_project_path(a.clone());
    app.test_add_track(1, TrackType::Audio);
    app.test_apply_engine_event(imported(7));
    let _ = app.update(Message::Clip(ClipMessage::DeleteClip(7)));
    (app, a)
}

fn save_as(app: &mut Resonance, target: &Path) {
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::SavePathSelected(Some(
        target.to_string_lossy().into_owned(),
    ))));
}

/// Run `task` and feed what it produced back through `update`.
fn drive(app: &mut Resonance, task: iced::Task<Message>) {
    for message in run_task(task) {
        let _ = app.update(message);
    }
}

fn answer(app: &mut Resonance, replace: bool) {
    let task = app.update(Message::ProjectIo(ProjectIoMessage::ReplaceProjectChoice(replace)));
    drive(app, task);
}

/// Answer the collecting save's engine round-trip and run its write.
fn complete_save(app: &mut Resonance) {
    let request = app.test_save_request();
    let _ = app.test_engine_event_task(AudioEvent::ClipsSavedToProjectDir {
        request,
        clip_files: Vec::new(),
    });
    let write = app.test_engine_event_task(AudioEvent::AllPluginStatesSaved {
        request,
        states: Vec::new(),
    });
    drive(app, write);
    assert_eq!(app.test_save_in_flight(), None, "the save completed");
}

/// The dot-prefixed sibling a replace moves the old files into.
fn trash_dirs(root: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(root)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().contains(".replaced-"))
        .collect()
}

fn assert_untouched(b: &Path) {
    assert!(b.join("project.json").exists());
    for file in HISTORY_AND_MEDIA {
        assert!(b.join(file).exists(), "{file} is still there");
    }
}

#[test]
fn save_as_onto_another_project_asks_before_touching_it() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, a) = titled_app(root.path());
    let b = other_project(root.path(), "B.rproj");

    save_as(&mut app, &b);

    assert_eq!(app.test_save_in_flight(), None, "nothing is saved before the answer");
    assert_eq!(app.test_project_path(), Some(a.as_path()), "still the open project");
    assert_eq!(app.test_confirm_replace_project(), Some(b.as_path()));
    assert_eq!(app.root_overlay(), Some(Overlay::ConfirmReplaceProject));
    iced_test::simulator(app.view())
        .find("Replace project \u{201c}B\u{201d}? Its history and media will be deleted.")
        .expect("the dialog names the project and what goes");
    assert_untouched(&b);
}

#[test]
fn cancelling_the_replace_saves_nothing() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, a) = titled_app(root.path());
    let b = other_project(root.path(), "B.rproj");

    save_as(&mut app, &b);
    answer(&mut app, false);

    assert_eq!(app.root_overlay(), None, "the dialog is gone");
    assert_eq!(app.test_save_in_flight(), None);
    assert_eq!(app.test_project_path(), Some(a.as_path()));
    assert_untouched(&b);
}

#[test]
fn replacing_clears_the_old_projects_history_and_media_then_saves() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, a) = titled_app(root.path());
    let b = other_project(root.path(), "B.rproj");

    save_as(&mut app, &b);
    answer(&mut app, true);

    assert_eq!(app.test_project_path(), Some(b.as_path()));
    assert!(app.test_save_in_flight().is_some(), "the save started");
    for file in HISTORY_AND_MEDIA {
        assert!(!b.join(file).exists(), "{file} went with the replaced project");
    }
    assert!(!b.join("project.json").exists(), "until the save writes the new one");
    assert_eq!(std::fs::read(b.join("notes.txt")).unwrap(), b"mine", "not the project's");
    assert!(trash_dirs(root.path()).is_empty(), "the moved files were deleted");

    complete_save(&mut app);
    assert!(b.join("project.json").exists());
    let backups: Vec<_> = std::fs::read_dir(b.join("backups")).unwrap().flatten().collect();
    assert_eq!(backups.len(), 1, "only this save's backup: {backups:?}");
    assert!(!b.join("audio/clip_1.wav").exists(), "no media of the old project");
    // The open project's own files are untouched, and its undo history's
    // clip came along (BUG-60).
    assert!(a.join("audio/clip_7.wav").exists());
    assert_eq!(std::fs::read(b.join("audio/clip_7.wav")).unwrap(), b"RIFF seven");
}

/// Saving onto the open project's own folder is a plain save: no dialog,
/// nothing cleared — also when the path reaches it through a symlink.
#[cfg(unix)]
#[test]
fn saving_onto_the_open_project_does_not_ask() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, a) = titled_app(root.path());
    std::fs::write(a.join("project.json"), b"{}").unwrap();
    std::fs::create_dir_all(a.join("backups")).unwrap();
    std::fs::write(a.join("backups/project-x.json"), b"{}").unwrap();
    let link = root.path().join("Link.rproj");
    std::os::unix::fs::symlink(&a, &link).unwrap();

    save_as(&mut app, &link);

    assert_eq!(app.test_confirm_replace_project(), None);
    assert!(app.test_save_in_flight().is_some());
    assert!(a.join("backups/project-x.json").exists());
}

/// A folder with no `project.json` holds no project to replace.
#[test]
fn saving_into_a_folder_without_a_project_does_not_ask() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, _a) = titled_app(root.path());
    let c = root.path().join("C.rproj");
    std::fs::create_dir_all(c.join("audio")).unwrap();

    save_as(&mut app, &c);

    assert_eq!(app.test_confirm_replace_project(), None);
    assert_eq!(app.test_project_path(), Some(c.as_path()));
    assert!(app.test_save_in_flight().is_some());
}

/// The open project lives inside the target's `audio/`: clearing it would
/// delete the open project. Nothing moves and nothing is saved.
#[test]
fn a_replace_that_would_delete_the_open_project_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let b = other_project(root.path(), "B.rproj");
    let (mut app, a) = titled_app(&b.join("audio"));

    save_as(&mut app, &b);
    answer(&mut app, true);

    let banner = app.test_error_message().expect("the refusal is shown");
    assert!(banner.contains("Could not replace B"), "{banner}");
    assert_eq!(app.test_save_in_flight(), None);
    assert_eq!(app.test_project_path(), Some(a.as_path()));
    assert!(a.join("audio/clip_7.wav").exists());
    assert_untouched(&b);
}

// ---- control API ------------------------------------------------------

fn save_as_request(path: &Path, confirm: bool) -> Request {
    Request::new(
        1,
        "project.save_as",
        &json!({ "path": path.to_string_lossy(), "confirm": confirm }),
    )
    .unwrap()
}

#[test]
fn control_save_as_over_another_project_needs_confirm_and_then_replaces_it() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, _a) = titled_app(root.path());
    let b = other_project(root.path(), "B.rproj");

    let refused = roundtrip(&mut app, save_as_request(&b, false));
    let error = refused.error.expect("refused without confirm");
    assert_eq!(error.kind(), ErrorKind::NeedsConfirmation);
    assert!(error.message.contains("deletes its history"), "{}", error.message);
    assert_untouched(&b);

    let accepted = roundtrip(&mut app, save_as_request(&b, true));
    assert!(accepted.error.is_none(), "{:?}", accepted.error);
    assert_eq!(app.test_confirm_replace_project(), None, "no dialog for a client");
    assert!(app.test_save_in_flight().is_some());
    for file in HISTORY_AND_MEDIA {
        assert!(!b.join(file).exists(), "{file} went with the replaced project");
    }
}
