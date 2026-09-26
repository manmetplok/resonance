//! Autosave recovery after a crash (code review FU-M12a; ba todo #467's
//! actions, rebuilt): the GUI prompt on open, its three answers, the
//! control `project.open` path that never blocks on a modal, the startup
//! offer of a crashed untitled session, and the prompt's golden images.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, ProjectIoMessage, RecoveryChoice};
use resonance_app::project::session::{self, RecoveryOffer, SessionMarker, SESSION_MARKER};
use resonance_app::project::{
    load_project, save_autosave, save_project, AUTOSAVE_JSON, PROJECT_JSON,
};
use resonance_app::state::{RecoveryPrompt, ViewMode};
use resonance_app::{theme, Resonance};
use resonance_audio::types::{AudioEvent, TrackType};
use resonance_control::job::{JobStarted, JobState, JobStatus};
use serde_json::json;

use crate::common;

fn temp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "resonance_autosave_recovery_{}_{tag}_{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn set_mtime(path: &Path, at: SystemTime) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .and_then(|f| f.set_modified(at))
        .expect("set mtime");
}

fn crashed_marker(dir: &Path) {
    let marker = SessionMarker {
        pid: 0,
        session_id: "crashed-session".to_owned(),
    };
    std::fs::write(dir.join(SESSION_MARKER), serde_json::to_vec(&marker).unwrap()).unwrap();
}

/// A project whose last save has no tracks and whose autosave has two,
/// left behind by a crashed session.
fn crashed_project(tag: &str) -> PathBuf {
    let dir = temp_dir(tag).join("song.rproj");
    std::fs::create_dir_all(&dir).unwrap();
    let (mut app, _task) = Resonance::new_for_test();
    save_project(&dir, &app.test_build_project_file(), &[], &[]).unwrap();
    app.test_add_track(1, TrackType::Audio);
    app.test_add_track(2, TrackType::Audio);
    save_autosave(&dir, &app.test_build_project_file(), &[], &[]).unwrap();
    let now = SystemTime::now();
    set_mtime(&dir.join(PROJECT_JSON), now - Duration::from_secs(600));
    set_mtime(&dir.join(AUTOSAVE_JSON), now - Duration::from_secs(60));
    crashed_marker(&dir);
    dir
}

fn track_count(app: &Resonance) -> usize {
    app.test_build_project_file().tracks.len()
}

fn update(app: &mut Resonance, m: ProjectIoMessage) {
    let _ = app.update(Message::ProjectIo(m));
}

/// Stand in for the async load the returned `Task` would run, then for the
/// engine confirming the clear.
fn land_load(app: &mut Resonance, target: &Path) {
    let loaded = load_project(target).expect("load");
    update(app, ProjectIoMessage::ProjectLoaded(Ok(Box::new(loaded))));
    app.test_apply_engine_event(AudioEvent::AllCleared);
}

fn open_via_gui(app: &mut Resonance, dir: &Path) {
    update(
        app,
        ProjectIoMessage::OpenPathSelected(Some(dir.display().to_string())),
    );
}

// ---- The GUI prompt ------------------------------------------------------

#[test]
fn opening_a_crashed_project_asks_before_loading() {
    let dir = crashed_project("asks");
    let (mut app, _task) = Resonance::new_for_test();
    open_via_gui(&mut app, &dir);

    let prompt = app.test_recovery_prompt().expect("the recovery prompt is open");
    assert!(!prompt.untitled);
    assert_eq!(prompt.offer.dir, dir);
    // Nothing was loaded or adopted yet.
    assert_eq!(app.test_project_path(), None);
    assert!(!app.test_has_active_project());
}

#[test]
fn opening_a_cleanly_closed_project_loads_straight_away() {
    let dir = crashed_project("clean");
    std::fs::remove_file(dir.join(SESSION_MARKER)).unwrap();
    let (mut app, _task) = Resonance::new_for_test();
    open_via_gui(&mut app, &dir);
    assert!(app.test_recovery_prompt().is_none());
}

#[test]
fn recover_loads_the_autosave_dirty_and_keeps_it() {
    let dir = crashed_project("recover");
    let (mut app, _task) = Resonance::new_for_test();
    open_via_gui(&mut app, &dir);
    update(&mut app, ProjectIoMessage::RecoveryChoice(RecoveryChoice::RecoverAutosave));
    assert!(app.test_recovery_prompt().is_none());
    land_load(&mut app, &dir.join(AUTOSAVE_JSON));

    assert_eq!(track_count(&app), 2, "the autosave's state is the working state");
    assert_eq!(app.test_project_path(), Some(dir.as_path()), "it is still this project");
    assert!(app.test_dirty(), "recovered work is unsaved until the next save");
    assert!(dir.join(AUTOSAVE_JSON).exists(), "the autosave is kept");
    assert_eq!(
        session::read_marker(&dir).map(|m| m.session_id).as_deref(),
        Some(app.session_id()),
        "this session now holds the project"
    );
}

#[test]
fn open_last_saved_loads_the_canonical_project_clean() {
    let dir = crashed_project("lastsaved");
    let (mut app, _task) = Resonance::new_for_test();
    open_via_gui(&mut app, &dir);
    update(&mut app, ProjectIoMessage::RecoveryChoice(RecoveryChoice::OpenLastSaved));
    land_load(&mut app, &dir);

    assert_eq!(track_count(&app), 0);
    assert!(!app.test_dirty());
    assert!(dir.join(AUTOSAVE_JSON).exists(), "the autosave is kept");
}

#[test]
fn cancel_opens_nothing_and_leaves_the_crash_evidence() {
    let dir = crashed_project("cancel");
    let (mut app, _task) = Resonance::new_for_test();
    open_via_gui(&mut app, &dir);
    update(&mut app, ProjectIoMessage::RecoveryChoice(RecoveryChoice::Cancel));

    assert!(app.test_recovery_prompt().is_none());
    assert_eq!(app.test_project_path(), None);
    let marker = session::read_marker(&dir).expect("marker untouched");
    assert_eq!(marker.session_id, "crashed-session");
}

// ---- Control: never a modal ----------------------------------------------

fn open_job(app: &mut Resonance, params: serde_json::Value) -> u64 {
    let started: JobStarted = common::call(app, "project.open", params)
        .result()
        .expect("job started");
    u64::from(started.job_id)
}

fn job_result(app: &mut Resonance, job: u64) -> serde_json::Value {
    let status: JobStatus = common::call(app, "job.status", json!({ "job_id": job }))
        .result()
        .expect("job.status");
    assert_eq!(status.state, JobState::Done);
    status.result.expect("result")
}

#[test]
fn control_open_loads_the_last_save_and_reports_the_autosave() {
    let dir = crashed_project("control_default");
    let (mut app, _task) = Resonance::new_for_test();
    let job = open_job(&mut app, json!({ "path": dir.display().to_string() }));
    assert!(app.test_recovery_prompt().is_none(), "no modal for a client");
    land_load(&mut app, &dir);

    let result = job_result(&mut app, job);
    assert_eq!(result["autosave_available"], json!(true));
    assert!(result.get("recovered_autosave").is_none());
    assert_eq!(track_count(&app), 0);
    assert!(!app.test_dirty());
}

#[test]
fn control_open_can_recover_the_autosave() {
    let dir = crashed_project("control_recover");
    let (mut app, _task) = Resonance::new_for_test();
    let job = open_job(
        &mut app,
        json!({ "path": dir.display().to_string(), "recover_autosave": true }),
    );
    land_load(&mut app, &dir.join(AUTOSAVE_JSON));

    let result = job_result(&mut app, job);
    assert_eq!(result["recovered_autosave"], json!(true));
    assert_eq!(result["autosave_available"], json!(true));
    assert_eq!(result["path"], json!(dir.display().to_string()));
    assert_eq!(track_count(&app), 2);
    assert!(app.test_dirty());
}

#[test]
fn control_open_of_a_clean_project_reports_nothing_extra() {
    let dir = crashed_project("control_clean");
    std::fs::remove_file(dir.join(SESSION_MARKER)).unwrap();
    let (mut app, _task) = Resonance::new_for_test();
    let job = open_job(
        &mut app,
        json!({ "path": dir.display().to_string(), "recover_autosave": true }),
    );
    land_load(&mut app, &dir);

    let result = job_result(&mut app, job);
    assert!(result.get("autosave_available").is_none());
    assert!(result.get("recovered_autosave").is_none());
    assert!(!app.test_dirty());
}

// ---- A crashed untitled session, at startup --------------------------------

/// A crashed untitled session's scratch dir under the hermetic scratch
/// root, dated `ahead` into the future so the startup scan prefers it over
/// any other test's scratch dir.
fn crashed_untitled(ahead: Duration) -> PathBuf {
    let root = resonance_app::user_dirs::hermetic_root()
        .expect("a test app has been built")
        .join("data/resonance/autosave");
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = root.join(format!("crashed-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (mut app, _task) = Resonance::new_for_test();
    app.test_add_track(1, TrackType::Audio);
    save_autosave(&dir, &app.test_build_project_file(), &[], &[]).unwrap();
    set_mtime(&dir.join(AUTOSAVE_JSON), SystemTime::now() + ahead);
    crashed_marker(&dir);
    dir
}

#[test]
fn a_crashed_untitled_session_is_offered_at_startup() {
    // One test, run in sequence: the scratch root is process-wide.
    let (mut app, _task) = Resonance::new_for_test();

    // Discard deletes it.
    let discarded = crashed_untitled(Duration::from_secs(86_400 * 365));
    app.test_offer_orphaned_session();
    let prompt = app.test_recovery_prompt().expect("offered at startup");
    assert!(prompt.untitled);
    assert_eq!(prompt.offer.dir, discarded);
    update(&mut app, ProjectIoMessage::RecoveryChoice(RecoveryChoice::Discard));
    assert!(!discarded.exists());

    // Recover opens it untitled and dirty; its files stay until a save.
    let recovered = crashed_untitled(Duration::from_secs(86_400 * 366));
    app.test_offer_orphaned_session();
    assert_eq!(app.test_recovery_prompt().map(|p| p.offer.dir.clone()), Some(recovered.clone()));
    update(&mut app, ProjectIoMessage::RecoveryChoice(RecoveryChoice::RecoverAutosave));
    land_load(&mut app, &recovered.join(AUTOSAVE_JSON));
    assert_eq!(app.test_project_path(), None, "a recovered untitled session stays untitled");
    assert_eq!(track_count(&app), 1);
    assert!(app.test_dirty());
    assert_eq!(
        session::read_marker(&recovered).map(|m| m.session_id).as_deref(),
        Some(app.session_id()),
        "claimed: a second crash offers it again"
    );

    // Saving it somewhere real retires the crashed scratch dir.
    let target = temp_dir("untitled_save").join("song.rproj");
    update(
        &mut app,
        ProjectIoMessage::SavePathSelected(Some(target.display().to_string())),
    );
    app.test_apply_engine_event(AudioEvent::ClipsSavedToProjectDir { clip_files: Vec::new() });
    app.test_apply_engine_event(AudioEvent::AllPluginStatesSaved { states: Vec::new() });
    update(&mut app, ProjectIoMessage::ProjectSaved(Ok(()), false));
    assert!(!recovered.exists());
}

// ---- Golden images -------------------------------------------------------

const WINDOW: (f32, f32) = (1440.0, 900.0);

fn sim_settings() -> iced::Settings {
    let mut fonts: Vec<std::borrow::Cow<'static, [u8]>> = Vec::new();
    fonts.push(theme::ICON_FONT_BYTES.into());
    for face in theme::UI_FONT_FACES {
        fonts.push((*face).into());
    }
    iced::Settings {
        fonts,
        default_font: theme::UI_FONT,
        ..iced::Settings::default()
    }
}

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

fn fixed_offer(untitled: bool) -> RecoveryPrompt {
    let saved = SystemTime::UNIX_EPOCH + Duration::from_secs(1_750_000_000);
    RecoveryPrompt {
        offer: RecoveryOffer {
            dir: PathBuf::from("/home/user/Music/Night Drive.rproj"),
            autosave_at: saved + Duration::from_secs(12 * 60),
            saved_at: (!untitled).then_some(saved),
        },
        untitled,
    }
}

#[test]
fn recovery_prompt_saved_project() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_recovery_prompt(fixed_offer(false));
    {
        let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
        ui.find("Open last saved").expect("the saved-project choice");
        ui.find(
            "Resonance didn't close cleanly while \u{201c}Night Drive\u{201d} was open. Its \
             autosave has work the saved project doesn't, 12 minutes newer than the last save.",
        )
        .expect("the explanation names the project and the gap");
    }
    snapshot_to(&app, "tests/snapshots/recovery_prompt_saved_project.png");
}

#[test]
fn recovery_prompt_untitled_at_startup() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_recovery_prompt(fixed_offer(true));
    {
        let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
        ui.find("Discard").expect("an untitled session can be discarded");
        assert!(ui.find("Open last saved").is_err(), "there is no saved version");
    }
    snapshot_to(&app, "tests/snapshots/recovery_prompt_untitled_at_startup.png");
}

#[test]
fn the_prompt_buttons_carry_their_choices() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_recovery_prompt(fixed_offer(false));
    for (label, choice) in [
        ("Recover autosave", RecoveryChoice::RecoverAutosave),
        ("Open last saved", RecoveryChoice::OpenLastSaved),
        ("Cancel", RecoveryChoice::Cancel),
    ] {
        let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
        ui.click(label).expect("button present");
        let messages: Vec<Message> = ui.into_messages().collect();
        assert!(
            messages.iter().any(|m| matches!(
                m,
                Message::ProjectIo(ProjectIoMessage::RecoveryChoice(c)) if *c == choice
            )),
            "{label} → {choice:?}: {messages:?}"
        );
    }
}
