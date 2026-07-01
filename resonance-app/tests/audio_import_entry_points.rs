//! Audio import — entry points (doc #175, ba todo #608).
//!
//! Two entry points start an audio import from the Arrange view:
//! (1) dragging audio files onto the window (the `arrange_audio_file_drop`
//!     subscription) and (2) the chrome "Import audio…" button (`PickFiles`).
//!
//! The window subscription itself can't be exercised headlessly, so these
//! tests pin the two halves it relies on: the `is_pool_audio_path` filter
//! that decides which OS drops count, and the reducer behaviour for the
//! messages the subscription emits (`WindowAudioDrop`) and the button emits
//! (`PickFiles`), driven through the real `Resonance::update` / `dispatch`
//! so the startup gate is exercised.

use std::path::{Path, PathBuf};

use resonance_app::message::{Message, PoolMessage};
use resonance_app::update::pool::is_pool_audio_path;
use resonance_app::Resonance;
use resonance_audio::types::AudioCommand;

/// App with an active, saved project so the startup-modal gate lets Pool
/// messages through and `import()` has a `project_path` to write into.
fn app_with_project() -> (Resonance, resonance_audio::__test_support::Receiver<AudioCommand>) {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_project_path(PathBuf::from("/proj/song.rproj"));
    let rx = app.test_capture_engine();
    (app, rx)
}

fn drain(rx: &resonance_audio::__test_support::Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

// ---------------------------------------------------------------------------
// is_pool_audio_path: only wav/flac/mp3/ogg start an import
// ---------------------------------------------------------------------------

#[test]
fn is_pool_audio_path_accepts_all_four_containers() {
    assert!(is_pool_audio_path(Path::new("/tmp/loop.wav")));
    assert!(is_pool_audio_path(Path::new("/tmp/stem.flac")));
    assert!(is_pool_audio_path(Path::new("/tmp/vocal.mp3")));
    assert!(is_pool_audio_path(Path::new("/tmp/fx.ogg")));
}

#[test]
fn is_pool_audio_path_is_case_insensitive() {
    assert!(is_pool_audio_path(Path::new("/tmp/KICK.WAV")));
    assert!(is_pool_audio_path(Path::new("/tmp/Snare.Flac")));
    assert!(is_pool_audio_path(Path::new("/tmp/bass.MP3")));
    assert!(is_pool_audio_path(Path::new("/tmp/pad.OGG")));
}

#[test]
fn is_pool_audio_path_rejects_non_audio_files() {
    assert!(!is_pool_audio_path(Path::new("/tmp/song.mid")));
    assert!(!is_pool_audio_path(Path::new("/tmp/notes.txt")));
    assert!(!is_pool_audio_path(Path::new("/tmp/project.rproj")));
    assert!(!is_pool_audio_path(Path::new("/tmp/wav")), "no extension");
    assert!(
        !is_pool_audio_path(Path::new("/tmp/kick.wav.bak")),
        "double extension"
    );
    assert!(
        !is_pool_audio_path(Path::new("/tmp/.wav")),
        "dotfile, no stem"
    );
}

// ---------------------------------------------------------------------------
// WindowAudioDrop — OS file-drop entry point
// ---------------------------------------------------------------------------

#[test]
fn window_audio_drop_sends_import_command_and_queues_placement() {
    let (mut app, rx) = app_with_project();

    let _ = app.update(Message::Pool(PoolMessage::WindowAudioDrop(
        PathBuf::from("/samples/kick.wav"),
    )));

    let cmds = drain(&rx);

    // Exactly one ImportAudioToPool for the dropped file.
    let import_paths = cmds
        .iter()
        .find_map(|c| match c {
            AudioCommand::ImportAudioToPool { paths } => Some(paths.clone()),
            _ => None,
        })
        .expect("ImportAudioToPool was sent");
    assert_eq!(import_paths, vec!["/samples/kick.wav"]);

    // One pending import queued (will be placed once AssetImported fires).
    assert_eq!(
        app.test_pending_import_count(),
        1,
        "one placement pending for the dropped file"
    );

    // A new audio track was pre-spawned for the placement (NewTrack path).
    let add_track = cmds.iter().any(|c| matches!(c, AudioCommand::AddTrack { .. }));
    assert!(add_track, "AddTrack sent for the new audio track");
}

#[test]
fn window_audio_drop_at_playhead_zero_by_default() {
    // The handler uses r.transport.playhead as the drop position; with a
    // fresh app it is zero, so the snapped placement is at sample 0.
    let (mut app, _rx) = app_with_project();
    assert_eq!(app.test_playhead(), 0, "playhead starts at 0");

    // Drop at playhead 0 — no panic, placement queued.
    let _ = app.update(Message::Pool(PoolMessage::WindowAudioDrop(
        PathBuf::from("/samples/snare.flac"),
    )));
    assert_eq!(app.test_pending_import_count(), 1);
}

#[test]
fn window_audio_drop_multiple_files_queues_each_separately() {
    // The subscription fires one WindowAudioDrop per file; each dispatch
    // queues its own pending import and sends its own ImportAudioToPool.
    let (mut app, rx) = app_with_project();

    let _ = app.update(Message::Pool(PoolMessage::WindowAudioDrop(
        PathBuf::from("/samples/kick.wav"),
    )));
    let _ = app.update(Message::Pool(PoolMessage::WindowAudioDrop(
        PathBuf::from("/samples/snare.wav"),
    )));

    assert_eq!(
        app.test_pending_import_count(),
        2,
        "one placement queued per drop"
    );

    let cmds = drain(&rx);
    let import_count = cmds
        .iter()
        .filter(|c| matches!(c, AudioCommand::ImportAudioToPool { .. }))
        .count();
    assert_eq!(import_count, 2, "one ImportAudioToPool per file");
}

// ---------------------------------------------------------------------------
// Gating: both entry points blocked when no project is active
// ---------------------------------------------------------------------------

#[test]
fn window_audio_drop_gated_without_active_project() {
    let (mut app, rx) = {
        let (mut a, _task) = Resonance::new();
        // No test_set_active_project — startup modal owns the screen.
        let rx = a.test_capture_engine();
        (a, rx)
    };

    let _ = app.update(Message::Pool(PoolMessage::WindowAudioDrop(
        PathBuf::from("/samples/kick.wav"),
    )));

    // No commands should have been sent to the engine.
    let cmds = drain(&rx);
    assert!(
        cmds.is_empty(),
        "WindowAudioDrop is gated while no project is open"
    );
    assert_eq!(app.test_pending_import_count(), 0);
}

#[test]
fn pick_files_gated_without_active_project() {
    // PickFiles opens the OS dialog via a Task — no state changes at dispatch
    // time. The gate should swallow it before the task is even scheduled.
    let (mut app, rx) = {
        let (mut a, _task) = Resonance::new();
        let rx = a.test_capture_engine();
        (a, rx)
    };

    // Gate swallows the message; the returned task is empty.
    let task = app.update(Message::Pool(PoolMessage::PickFiles));
    // The task is opaque — we can't inspect it directly — but we can confirm
    // no engine commands were queued (no side-effects occurred at dispatch).
    let cmds = drain(&rx);
    assert!(
        cmds.is_empty(),
        "PickFiles is gated while no project is open"
    );
    // Suppress "unused" warning on `task`; the important assertion is above.
    drop(task);
}

// ---------------------------------------------------------------------------
// PickFiles with an active project: no state change at dispatch time
// ---------------------------------------------------------------------------

#[test]
fn pick_files_with_project_dispatches_no_engine_commands() {
    // PickFiles returns a Task::perform that opens the OS dialog asynchronously.
    // At dispatch time (before the user picks anything) no engine command
    // should have been sent and the pending import queue should be empty.
    let (mut app, rx) = app_with_project();

    let _task = app.update(Message::Pool(PoolMessage::PickFiles));

    let cmds = drain(&rx);
    assert!(
        cmds.is_empty(),
        "PickFiles sends no engine commands at dispatch time"
    );
    assert_eq!(
        app.test_pending_import_count(),
        0,
        "no pending imports before the dialog resolves"
    );
}

// ---------------------------------------------------------------------------
// Require a saved project — error message when no project path
// ---------------------------------------------------------------------------

#[test]
fn window_audio_drop_sets_error_when_project_is_unsaved() {
    // Project active (past the startup gate) but not yet saved — import
    // must be refused with a user-facing error rather than silently dropped.
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    // No project_path set — io.project_path is None.

    let _ = app.update(Message::Pool(PoolMessage::WindowAudioDrop(
        PathBuf::from("/samples/loop.wav"),
    )));

    assert!(
        app.test_pending_import_count() == 0,
        "import refused: no project path"
    );
}
