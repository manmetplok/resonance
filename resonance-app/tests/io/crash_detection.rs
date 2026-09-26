//! Crash detection for autosave recovery (code review FU-M12a): the
//! session marker's on-disk rule, and the marker following the project
//! through open, Save As, untitled autosave and a clean quit.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use resonance_app::message::{Message, ProjectIoMessage};
use resonance_app::project::session::{
    self, gc_scratch_root, probe, recoverable_from_state, retire_stale_autosave,
    scan_scratch_root, SessionMarker, SCRATCH_GC_AGE, SESSION_MARKER,
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
    let token = app.test_pending_open_token();
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::OpenLoadFinished(
        token,
        Ok(Box::new(loaded)),
    )));
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

// ---- Autosave hygiene (FU-R1a) ----------------------------------------

fn set_dir_mtime(dir: &Path, at: SystemTime) {
    std::fs::File::open(dir)
        .and_then(|f| f.set_modified(at))
        .expect("set dir mtime");
}

/// A project dir with a `project.json` at `saved` and an autosave (JSON +
/// `autosave/` side files) at `autosaved`.
fn project_with_autosave(tag: &str, saved: SystemTime, autosaved: SystemTime) -> PathBuf {
    let dir = temp_dir(tag);
    touch(&dir.join(PROJECT_JSON), saved);
    let side = dir.join("autosave");
    std::fs::create_dir_all(side.join("midi")).unwrap();
    std::fs::write(side.join("midi").join("clip_1.mid"), b"x").unwrap();
    touch(&dir.join(AUTOSAVE_JSON), autosaved);
    dir
}

#[test]
fn an_autosave_older_than_the_save_is_retired_with_its_side_files() {
    let dir = project_with_autosave("retire_old", t(20), t(10));
    retire_stale_autosave(&dir);
    assert!(!dir.join(AUTOSAVE_JSON).exists(), "stale autosave JSON must go");
    assert!(!dir.join("autosave").exists(), "its side files must go too");
    assert!(dir.join(PROJECT_JSON).exists(), "the save itself is untouched");

    // Same instant: the save holds everything the snapshot did.
    let same = project_with_autosave("retire_same", t(20), t(20));
    retire_stale_autosave(&same);
    assert!(!same.join(AUTOSAVE_JSON).exists());
}

#[test]
fn an_autosave_newer_than_the_save_is_kept() {
    let dir = project_with_autosave("retire_newer", t(10), t(20));
    retire_stale_autosave(&dir);
    assert!(dir.join(AUTOSAVE_JSON).exists(), "a newer autosave is not stale");
    assert!(dir.join("autosave").join("midi").join("clip_1.mid").exists());
}

#[test]
fn orphaned_side_files_without_their_json_are_retired() {
    let dir = project_with_autosave("retire_orphan", t(20), t(10));
    std::fs::remove_file(dir.join(AUTOSAVE_JSON)).unwrap();
    retire_stale_autosave(&dir);
    assert!(!dir.join("autosave").exists());
}

#[test]
fn a_never_saved_dir_keeps_its_autosave() {
    let dir = temp_dir("retire_unsaved");
    touch(&dir.join(AUTOSAVE_JSON), t(10));
    retire_stale_autosave(&dir);
    assert!(dir.join(AUTOSAVE_JSON).exists(), "no save: the autosave is the only copy");
}

#[test]
fn a_manual_save_retires_the_projects_stale_autosave() {
    let dir = saved_project("save_retires");
    let (mut app, _task) = Resonance::new_for_test();
    open(&mut app, &dir);
    // An autosave from before this save (a minute older than project.json).
    let saved_at = std::fs::metadata(dir.join(PROJECT_JSON)).unwrap().modified().unwrap();
    std::fs::create_dir_all(dir.join("autosave")).unwrap();
    touch(&dir.join(AUTOSAVE_JSON), saved_at - Duration::from_secs(60));

    // An autosave completing never retires anything.
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectSaved(Ok(()), true)));
    assert!(dir.join(AUTOSAVE_JSON).exists(), "an autosave must not retire itself");

    let _ = app.update(Message::ProjectIo(ProjectIoMessage::SaveProject));
    finish_collect(&mut app);
    // The write task is not run here; `project.json` already stands in
    // for what it writes (newer than the autosave).
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectSaved(Ok(()), false)));
    assert!(!dir.join(AUTOSAVE_JSON).exists(), "the save supersedes the autosave");
    assert!(!dir.join("autosave").exists());
}

#[test]
fn scratch_gc_drops_only_old_markerless_dirs() {
    let root = temp_dir("gc");
    let now = SystemTime::now();
    let old = now - SCRATCH_GC_AGE - Duration::from_secs(3600);
    let recent = now - Duration::from_secs(3600);
    let mk = |name: &str, at: SystemTime, marker: bool| {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        touch(&dir.join(AUTOSAVE_JSON), at);
        if marker {
            write_marker_as(&dir, 0, name);
        }
        set_dir_mtime(&dir, at);
        dir
    };
    let orphan_old = mk("orphan-old", old, false);
    let orphan_recent = mk("orphan-recent", recent, false);
    let crashed_old = mk("crashed-old", old, true);
    let mine_old = mk("mine", old, false);
    // An old dir whose autosave was rewritten recently is still active.
    let touched = mk("touched", old, false);
    touch(&touched.join(AUTOSAVE_JSON), recent);
    set_dir_mtime(&touched, old);

    assert_eq!(gc_scratch_root(&root, "mine", SCRATCH_GC_AGE, now), 1);
    assert!(!orphan_old.exists(), "an old marker-less scratch dir is collected");
    assert!(orphan_recent.exists(), "a recent one may still be in use");
    assert!(crashed_old.exists(), "a marked dir is recovery evidence, never collected");
    assert!(mine_old.exists(), "this session's own dir is never collected");
    assert!(touched.exists(), "recent autosave activity keeps a dir");
    assert_eq!(gc_scratch_root(&root.join("missing"), "mine", SCRATCH_GC_AGE, now), 0);
}
