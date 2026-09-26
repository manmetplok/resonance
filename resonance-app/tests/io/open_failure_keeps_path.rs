//! A failed project open must leave the open project's path — and the
//! engine's project dir — alone (code review STATE-01 / UPD-01). Before
//! the fix, `OpenPathSelected` / `OpenRecent` repointed both at the
//! target before the async load ran, so a load failure left project A
//! on screen but tied to project B's folder: Ctrl+S overwrote B,
//! recordings landed in `B/audio/`, and structural undos read A's clips
//! from B.

use resonance_app::message::{Message, ProjectIoMessage};
use resonance_app::project::{LoadedProject, ProjectFile};
use resonance_app::Resonance;
use resonance_audio::types::AudioCommand;
use std::path::{Path, PathBuf};

fn project_dirs_sent(
    rx: &resonance_audio::test_support::Receiver<AudioCommand>,
) -> Vec<PathBuf> {
    rx.try_iter()
        .filter_map(|cmd| match cmd {
            AudioCommand::SetProjectDir(dir) => Some(dir),
            _ => None,
        })
        .collect()
}

#[test]
fn failed_open_keeps_the_current_project_path_and_engine_dir() {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    let a = PathBuf::from("/tmp/open-failure-a.rproj");
    app.test_set_project_path(a.clone());
    app.test_set_active_project(true);
    let _ = rx.try_iter().count();

    let _ = app.update(Message::ProjectIo(ProjectIoMessage::OpenPathSelected(Some(
        "/tmp/open-failure-b.rproj".to_owned(),
    ))));
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectLoaded(Err(
        "No project.json found".to_owned(),
    ))));

    assert_eq!(app.test_project_path(), Some(a.as_path()));
    assert!(
        project_dirs_sent(&rx).is_empty(),
        "the engine's project dir must not be repointed at the failed target"
    );
}

#[test]
fn failed_open_recent_keeps_the_current_project_path() {
    let target = tempfile::tempdir().expect("temp dir");
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    let a = PathBuf::from("/tmp/open-failure-a.rproj");
    app.test_set_project_path(a.clone());
    let _ = rx.try_iter().count();

    let _ = app.update(Message::ProjectIo(ProjectIoMessage::OpenRecent(
        target.path().to_path_buf(),
    )));
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectLoaded(Err(
        "corrupt".to_owned(),
    ))));

    assert_eq!(app.test_project_path(), Some(a.as_path()));
    assert!(project_dirs_sent(&rx).is_empty());
}

#[test]
fn successful_open_adopts_the_target_path_and_engine_dir() {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_project_path(PathBuf::from("/tmp/open-failure-a.rproj"));
    let _ = rx.try_iter().count();

    let b = Path::new("/tmp/open-success-b.rproj");
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::OpenPathSelected(Some(
        b.display().to_string(),
    ))));
    // Nothing is repointed while the load is still in flight.
    assert_ne!(app.test_project_path(), Some(b));
    let loaded = LoadedProject {
        file: ProjectFile::default(),
        project_dir: b.to_path_buf(),
        midi_notes: Default::default(),
        plugin_states: Default::default(),
    };
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectLoaded(Ok(
        Box::new(loaded),
    ))));

    assert_eq!(app.test_project_path(), Some(b));
    assert_eq!(project_dirs_sent(&rx), vec![b.to_path_buf()]);
}
