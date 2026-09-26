//! Crash detection for autosave recovery (code review FU-M12a): the
//! session marker's on-disk rule, and the marker following the project
//! through open, Save As, untitled autosave and a clean quit.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use resonance_app::message::{Message, ProjectIoMessage};
use resonance_app::project::session::{
    self, probe, recoverable_from_state, scan_scratch_root, SessionMarker, SESSION_MARKER,
};
use resonance_app::project::{load_project, save_project, AUTOSAVE_JSON, PROJECT_JSON};
use resonance_app::Resonance;
use resonance_audio::types::AudioEvent;

fn temp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "resonance_crash_detection_{}_{tag}_{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn touch(path: &Path, at: SystemTime) {
    if !path.exists() {
        std::fs::write(path, b"{}").expect("write file");
    }
    std::fs::File::options()
        .write(true)
        .open(path)
        .and_then(|f| f.set_modified(at))
        .expect("set mtime");
}

fn write_marker_as(dir: &Path, pid: u32, session_id: &str) {
    let marker = SessionMarker {
        pid,
        session_id: session_id.to_owned(),
    };
    std::fs::write(dir.join(SESSION_MARKER), serde_json::to_vec(&marker).unwrap()).unwrap();
}

fn t(secs: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 + secs)
}

// ---- The pure rule ----------------------------------------------------

#[test]
fn only_an_unclean_exit_with_a_newer_autosave_is_recoverable() {
    assert!(recoverable_from_state(true, Some(t(10)), Some(t(5))));
    assert!(recoverable_from_state(true, Some(t(10)), None), "never saved: autosave is all there is");
    assert!(!recoverable_from_state(true, Some(t(5)), Some(t(10))), "saved after the autosave");
    assert!(!recoverable_from_state(true, Some(t(5)), Some(t(5))), "same instant: nothing newer");
    assert!(!recoverable_from_state(true, None, Some(t(5))), "no autosave");
    assert!(!recoverable_from_state(false, Some(t(10)), Some(t(5))), "clean exit");
}

// ---- Probing a directory ----------------------------------------------

#[test]
fn a_dead_sessions_marker_and_a_newer_autosave_offer_recovery() {
    let dir = temp_dir("probe");
    touch(&dir.join(PROJECT_JSON), t(5));
    touch(&dir.join(AUTOSAVE_JSON), t(10));
    write_marker_as(&dir, 0, "crashed-session");

    let offer = probe(&dir, "this-session").expect("recoverable");
    assert_eq!(offer.dir, dir);
    assert_eq!(offer.autosave_at, t(10));
    assert_eq!(offer.saved_at, Some(t(5)));
    assert_eq!(offer.autosave_json(), dir.join(AUTOSAVE_JSON));

    // Our own marker is not a crash.
    assert_eq!(probe(&dir, "crashed-session"), None);
    // A later manual save supersedes the autosave.
    touch(&dir.join(PROJECT_JSON), t(20));
    assert_eq!(probe(&dir, "this-session"), None);
}

#[test]
fn no_marker_means_a_clean_exit() {
    let dir = temp_dir("nomarker");
    touch(&dir.join(PROJECT_JSON), t(5));
    touch(&dir.join(AUTOSAVE_JSON), t(10));
    assert_eq!(probe(&dir, "this-session"), None);
}

#[test]
fn a_corrupt_marker_reads_as_a_crash() {
    let dir = temp_dir("corrupt");
    touch(&dir.join(AUTOSAVE_JSON), t(10));
    std::fs::write(dir.join(SESSION_MARKER), b"{ half a marker").unwrap();
    assert!(probe(&dir, "this-session").is_some());
}

#[cfg(target_os = "linux")]
#[test]
fn a_marker_held_by_a_running_process_is_not_a_crash() {
    let dir = temp_dir("live");
    touch(&dir.join(PROJECT_JSON), t(5));
    touch(&dir.join(AUTOSAVE_JSON), t(10));
    // pid 1 is always running: another instance has the project open.
    write_marker_as(&dir, 1, "other-instance");
    assert_eq!(probe(&dir, "this-session"), None);
}

#[test]
fn the_scratch_scan_offers_the_newest_crashed_untitled_session() {
    let root = temp_dir("scan");
    for (name, at) in [("a", 10), ("b", 30), ("c", 20)] {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        touch(&dir.join(AUTOSAVE_JSON), t(at));
        write_marker_as(&dir, 0, name);
    }
    // Newest of all, but ours.
    let mine = root.join("mine");
    std::fs::create_dir_all(&mine).unwrap();
    touch(&mine.join(AUTOSAVE_JSON), t(99));
    write_marker_as(&mine, 0, "mine");

    let offer = scan_scratch_root(&root, "mine").expect("an orphan");
    assert_eq!(offer.dir, root.join("b"));
    assert_eq!(scan_scratch_root(&root.join("missing"), "mine"), None);
}

// ---- The marker follows the project -----------------------------------

fn saved_project(tag: &str) -> PathBuf {
    let dir = temp_dir(tag).join("song.rproj");
    std::fs::create_dir_all(&dir).unwrap();
    let (app, _task) = Resonance::new_for_test();
    save_project(&dir, &app.test_build_project_file(), &[], &[]).expect("save project");
    dir
}

fn open(app: &mut Resonance, dir: &Path) {
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::OpenPathSelected(Some(
        dir.display().to_string(),
    ))));
    let loaded = load_project(dir).expect("load project");
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectLoaded(Ok(Box::new(loaded)))));
}

fn finish_collect(app: &mut Resonance) {
    app.test_apply_engine_event(AudioEvent::ClipsSavedToProjectDir { clip_files: Vec::new() });
    app.test_apply_engine_event(AudioEvent::AllPluginStatesSaved { states: Vec::new() });
}

fn marker_owner(dir: &Path) -> Option<String> {
    session::read_marker(dir).map(|m| m.session_id)
}

#[test]
fn opening_a_project_claims_it_and_a_clean_quit_releases_it() {
    let dir = saved_project("open");
    let (mut app, _task) = Resonance::new_for_test();
    open(&mut app, &dir);
    assert_eq!(marker_owner(&dir).as_deref(), Some(app.session_id()));

    let _ = app.update(Message::WindowCloseRequested(iced::window::Id::unique()));
    assert_eq!(marker_owner(&dir), None, "a clean quit is not a crash");
}

#[test]
fn opening_another_project_releases_the_first() {
    let first = saved_project("first");
    let second = saved_project("second");
    let (mut app, _task) = Resonance::new_for_test();
    open(&mut app, &first);
    open(&mut app, &second);
    assert_eq!(marker_owner(&first), None);
    assert_eq!(marker_owner(&second).as_deref(), Some(app.session_id()));
}

#[test]
fn save_as_moves_the_marker_to_the_new_location() {
    let old = saved_project("saveas_old");
    let new = old.parent().unwrap().join("copy.rproj");
    let (mut app, _task) = Resonance::new_for_test();
    open(&mut app, &old);

    let _ = app.update(Message::ProjectIo(ProjectIoMessage::SavePathSelected(Some(
        new.display().to_string(),
    ))));
    finish_collect(&mut app);
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectSaved(Ok(()), false)));

    assert_eq!(marker_owner(&old), None);
    assert_eq!(marker_owner(&new).as_deref(), Some(app.session_id()));
}

#[test]
fn an_untitled_autosave_marks_its_scratch_dir_until_a_clean_quit() {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::Autosave));
    let (scratch, is_autosave) = app.test_save_in_flight().expect("autosave collecting");
    assert!(is_autosave);
    finish_collect(&mut app);
    // The write task is not run here; stand in for what it writes.
    std::fs::write(scratch.join(AUTOSAVE_JSON), b"{}").unwrap();
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectSaved(Ok(()), true)));
    assert_eq!(marker_owner(&scratch).as_deref(), Some(app.session_id()));

    let _ = app.update(Message::WindowCloseRequested(iced::window::Id::unique()));
    assert!(!scratch.exists(), "a clean quit drops the untitled scratch dir");
}

#[test]
fn saving_an_untitled_project_retires_its_scratch_autosave() {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::Autosave));
    let (scratch, _) = app.test_save_in_flight().expect("autosave collecting");
    finish_collect(&mut app);
    std::fs::write(scratch.join(AUTOSAVE_JSON), b"{}").unwrap();
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectSaved(Ok(()), true)));

    let target = temp_dir("untitled_saveas").join("song.rproj");
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::SavePathSelected(Some(
        target.display().to_string(),
    ))));
    finish_collect(&mut app);
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectSaved(Ok(()), false)));

    assert!(!scratch.exists(), "the scratch autosave is superseded by the real save");
    assert_eq!(marker_owner(&target).as_deref(), Some(app.session_id()));
}
