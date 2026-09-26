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

fn loaded_at(dir: &Path) -> Box<LoadedProject> {
    Box::new(LoadedProject {
        file: ProjectFile::default(),
        project_dir: dir.to_path_buf(),
        midi_notes: Default::default(),
        plugin_states: Default::default(),
    })
}

/// Two overlapping GUI opens (FU-A1a): `pending_open_path` used to be a
/// single slot, so the *first* load to land adopted the *second* path —
/// project A on screen, tied to B's folder. Each open's load now carries
/// the token it was started with; a load an overtaking open made stale
/// is dropped, whichever order the two finish in.
#[test]
fn overlapping_opens_adopt_only_the_latest_open() {
    for stale_lands_first in [true, false] {
        let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
        let current = PathBuf::from("/tmp/open-overlap-current.rproj");
        app.test_set_project_path(current.clone());
        let _ = rx.try_iter().count();

        let a = Path::new("/tmp/open-overlap-a.rproj");
        let b = std::env::temp_dir();
        let b = b.as_path();
        let _ = app.update(Message::ProjectIo(ProjectIoMessage::OpenPathSelected(Some(
            a.display().to_string(),
        ))));
        let token_a = app.test_pending_open_token();
        let _ = app.update(Message::ProjectIo(ProjectIoMessage::OpenRecent(
            b.to_path_buf(),
        )));
        let token_b = app.test_pending_open_token();
        assert_ne!(token_a, token_b, "each open gets its own token");

        let finish = |app: &mut Resonance, token, dir: &Path| {
            let _ = app.update(Message::ProjectIo(ProjectIoMessage::OpenLoadFinished(
                token,
                Ok(loaded_at(dir)),
            )));
        };
        if stale_lands_first {
            finish(&mut app, token_a, a);
            assert_eq!(
                app.test_project_path(),
                Some(current.as_path()),
                "the stale load of A must not adopt anything"
            );
            finish(&mut app, token_b, b);
        } else {
            finish(&mut app, token_b, b);
            finish(&mut app, token_a, a);
        }
        assert_eq!(app.test_project_path(), Some(b), "stale_first={stale_lands_first}");
        assert_eq!(
            project_dirs_sent(&rx),
            vec![b.to_path_buf()],
            "only B's load repoints the engine (stale_first={stale_lands_first})"
        );
    }
}

/// A stale open's *failure* must not clear the pending slot of the open
/// that overtook it, or B's later success would adopt no path at all.
#[test]
fn a_stale_failed_open_leaves_the_latest_open_pending() {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    let _ = rx.try_iter().count();
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::OpenPathSelected(Some(
        "/tmp/open-overlap-a.rproj".to_owned(),
    ))));
    let token_a = app.test_pending_open_token();
    let b = Path::new("/tmp/open-overlap-b.rproj");
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::OpenPathSelected(Some(
        b.display().to_string(),
    ))));
    let token_b = app.test_pending_open_token();

    let _ = app.update(Message::ProjectIo(ProjectIoMessage::OpenLoadFinished(
        token_a,
        Err("corrupt".to_owned()),
    )));
    assert!(app.test_error_message().is_none(), "a stale failure is not surfaced");
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::OpenLoadFinished(
        token_b,
        Ok(loaded_at(b)),
    )));
    assert_eq!(app.test_project_path(), Some(b));
}
