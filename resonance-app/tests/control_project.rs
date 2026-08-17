//! `project.*` control handlers (ba doc #265, todo #1151): new / open /
//! save / save-as with explicit paths as jobs — param validation, the
//! dirty-project and overwrite `needs_confirmation` guards, the `busy`
//! guard, and job completion through the real update-loop / engine-event
//! paths.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::{Message, ProjectIoMessage};
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::AudioEvent;
use resonance_control::job::{JobStarted, JobState, JobStatus};
use resonance_control::{ErrorKind, Request, Response};
use serde_json::json;

fn app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    Resonance::new_for_test().0
}

fn roundtrip(app: &mut Resonance, req: Request) -> Response {
    let (reply, rx) = ReplySender::test_pair();
    let _ = app.update(Message::Control(ControlMessage::Request(ControlRequest {
        conn: 1,
        request: req,
        reply,
    })));
    rx.try_recv().expect("one reply per request")
}

fn request(id: i64, method: &str, params: serde_json::Value) -> Request {
    Request::new(id, method, &params).expect("params serialize")
}

fn job_status(app: &mut Resonance, job_id: u64) -> JobStatus {
    roundtrip(app, request(99, "job.status", json!({ "job_id": job_id })))
        .result()
        .expect("job.status succeeds")
}

fn started_job(response: Response) -> u64 {
    let started: JobStarted = response.result().expect("job started");
    u64::from(started.job_id)
}

// ---------------- project.new ----------------

#[test]
fn new_completes_when_the_engine_confirms_the_clear() {
    let mut app = app();
    // Omitted params entirely: defaults to the empty built-in template.
    let job = started_job(roundtrip(
        &mut app,
        Request::without_params(1, "project.new"),
    ));
    assert_eq!(job_status(&mut app, job).state, JobState::Pending);

    // The engine confirms the clear → the fresh project replays → done.
    app.test_apply_engine_event(AudioEvent::AllCleared);
    let status = job_status(&mut app, job);
    assert_eq!(status.state, JobState::Done);
    let result = status.result.expect("done jobs carry a result");
    assert!(result["path"].is_null(), "a fresh project is untitled");
    assert!(result["revision"].is_u64(), "result carries the revision");

    // Compose-ready: active project, untitled (next save is a save-as).
    assert!(app.test_has_active_project());
    assert_eq!(app.test_project_path(), None);
}

#[test]
fn new_accepts_builtin_slugs_via_the_template_id_alias() {
    let mut app = app();
    let job = started_job(roundtrip(
        &mut app,
        request(1, "project.new", json!({ "template_id": "beatmaking" })),
    ));
    app.test_apply_engine_event(AudioEvent::AllCleared);
    assert_eq!(job_status(&mut app, job).state, JobState::Done);
    // The Beatmaking starter replayed (its tempo is 90 BPM).
    assert_eq!(app.test_transport_bpm(), 90.0);
    assert_eq!(app.test_project_path(), None);
}

#[test]
fn new_requires_confirm_when_the_project_is_dirty() {
    let mut app = app();
    app.test_set_active_project(true);
    app.test_set_dirty(true);

    let response = roundtrip(&mut app, request(1, "project.new", json!({})));
    assert_eq!(
        response.error.expect("dirty guard").kind(),
        ErrorKind::NeedsConfirmation
    );

    // With confirm the instantiation proceeds.
    let response = roundtrip(
        &mut app,
        request(2, "project.new", json!({ "confirm": true })),
    );
    let job = started_job(response);
    assert_eq!(job_status(&mut app, job).state, JobState::Pending);
}

#[test]
fn new_with_an_unknown_template_is_invalid_params() {
    let mut app = app();
    let response = roundtrip(
        &mut app,
        request(1, "project.new", json!({ "template": "no-such-template" })),
    );
    let error = response.error.expect("unknown template");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert!(
        error.message.contains("empty"),
        "error lists the built-in slugs: {}",
        error.message
    );
}

#[test]
fn new_while_an_instantiation_is_pending_is_busy() {
    let mut app = app();
    let _ = started_job(roundtrip(&mut app, request(1, "project.new", json!({}))));
    // The engine has not confirmed the clear yet: a second lifecycle op
    // would clobber the pending load.
    let response = roundtrip(&mut app, request(2, "project.new", json!({})));
    assert_eq!(
        response.error.expect("pending load").kind(),
        ErrorKind::Busy
    );
}

// ---------------- project.open ----------------

#[test]
fn open_validates_the_path() {
    let mut app = app();

    let response = roundtrip(
        &mut app,
        request(1, "project.open", json!({ "path": "relative/song.rproj" })),
    );
    assert_eq!(
        response.error.expect("relative path").kind(),
        ErrorKind::InvalidParams
    );

    let response = roundtrip(
        &mut app,
        request(
            2,
            "project.open",
            json!({ "path": "/definitely/not/here.rproj" }),
        ),
    );
    assert_eq!(
        response.error.expect("missing project").kind(),
        ErrorKind::NotFound
    );
}

#[test]
fn open_requires_confirm_when_the_project_is_dirty() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("song.rproj");
    std::fs::create_dir(&target).expect("existing project dir");

    let mut app = app();
    app.test_set_active_project(true);
    app.test_set_dirty(true);

    let response = roundtrip(
        &mut app,
        request(
            1,
            "project.open",
            json!({ "path": target.display().to_string() }),
        ),
    );
    assert_eq!(
        response.error.expect("dirty guard").kind(),
        ErrorKind::NeedsConfirmation
    );
}

#[test]
fn open_starts_a_job_that_fails_with_the_load_error() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("song.rproj");
    std::fs::create_dir(&target).expect("existing project dir");

    let mut app = app();
    let job = started_job(roundtrip(
        &mut app,
        request(
            1,
            "project.open",
            json!({ "path": target.display().to_string() }),
        ),
    ));
    assert_eq!(job_status(&mut app, job).state, JobState::Pending);
    // The open routed through the path-carrying message: the path is set.
    assert_eq!(app.test_project_path(), Some(target.as_path()));

    // The async load fails → the job fails with the message.
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectLoaded(Err(
        "corrupt project.json".to_owned(),
    ))));
    let status = job_status(&mut app, job);
    assert_eq!(status.state, JobState::Error);
    assert_eq!(status.error.as_deref(), Some("corrupt project.json"));
}

// ---------------- project.save / project.save_as ----------------

#[test]
fn save_without_any_path_is_invalid_params() {
    let mut app = app();
    let response = roundtrip(&mut app, request(1, "project.save", json!({})));
    let error = response.error.expect("never-saved project needs a path");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert!(error.message.contains("path"), "{}", error.message);
}

#[test]
fn save_establishes_the_path_and_completes_on_project_saved() {
    let dir = tempfile::tempdir().expect("temp dir");
    let raw = dir.path().join("song"); // no extension on purpose

    let mut app = app();
    app.test_set_active_project(true);
    let job = started_job(roundtrip(
        &mut app,
        request(
            1,
            "project.save",
            json!({ "path": raw.display().to_string() }),
        ),
    ));

    // The `.rproj` extension was applied exactly like the GUI save path.
    let expected = dir.path().join("song.rproj");
    assert_eq!(app.test_project_path(), Some(expected.as_path()));
    assert_eq!(job_status(&mut app, job).state, JobState::Pending);

    // The save collector finishing resolves the job with path + revision.
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectSaved(
        Ok(()),
        false,
    )));
    let status = job_status(&mut app, job);
    assert_eq!(status.state, JobState::Done);
    let result = status.result.expect("done jobs carry a result");
    assert_eq!(
        result["path"].as_str().expect("path echoed"),
        expected.display().to_string()
    );
    assert!(result["revision"].is_u64());
    // The project directory itself was created on disk.
    assert!(expected.is_dir());
}

#[test]
fn save_as_over_a_foreign_existing_path_requires_confirm() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("other.rproj");
    std::fs::create_dir(&target).expect("existing project dir");

    let mut app = app();
    app.test_set_active_project(true);
    app.test_set_project_path(dir.path().join("mine.rproj"));

    let response = roundtrip(
        &mut app,
        request(
            1,
            "project.save_as",
            json!({ "path": target.display().to_string() }),
        ),
    );
    assert_eq!(
        response.error.expect("overwrite guard").kind(),
        ErrorKind::NeedsConfirmation
    );

    // With confirm the save starts.
    let response = roundtrip(
        &mut app,
        request(
            2,
            "project.save_as",
            json!({ "path": target.display().to_string(), "confirm": true }),
        ),
    );
    let job = started_job(response);
    assert_eq!(job_status(&mut app, job).state, JobState::Pending);
    assert_eq!(app.test_project_path(), Some(target.as_path()));
}

#[test]
fn saving_over_the_projects_own_path_needs_no_confirm() {
    let dir = tempfile::tempdir().expect("temp dir");
    let own = dir.path().join("mine.rproj");
    std::fs::create_dir(&own).expect("existing project dir");

    let mut app = app();
    app.test_set_active_project(true);
    app.test_set_project_path(own.clone());

    // Both the pathless in-place save…
    let job = started_job(roundtrip(&mut app, request(1, "project.save", json!({}))));
    assert_eq!(job_status(&mut app, job).state, JobState::Pending);
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectSaved(
        Ok(()),
        false,
    )));
    assert_eq!(job_status(&mut app, job).state, JobState::Done);

    // …and an explicit save to the same path skip the overwrite guard.
    let response = roundtrip(
        &mut app,
        request(
            2,
            "project.save",
            json!({ "path": own.display().to_string() }),
        ),
    );
    let job = started_job(response);
    assert_eq!(job_status(&mut app, job).state, JobState::Pending);
}

#[test]
fn save_with_a_missing_parent_directory_is_invalid_params() {
    let mut app = app();
    let response = roundtrip(
        &mut app,
        request(
            1,
            "project.save",
            json!({ "path": "/definitely/not/here/song.rproj" }),
        ),
    );
    assert_eq!(
        response.error.expect("missing parent").kind(),
        ErrorKind::InvalidParams
    );
}

#[test]
fn a_second_save_while_one_is_in_flight_is_busy() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut app = app();
    app.test_set_active_project(true);

    let path = dir.path().join("song.rproj").display().to_string();
    let _ = started_job(roundtrip(
        &mut app,
        request(1, "project.save", json!({ "path": path })),
    ));

    // The collector is still gathering: a second save (and an open, and a
    // new) must not clobber it.
    for (id, method, params) in [
        (2, "project.save", json!({})),
        (
            3,
            "project.open",
            json!({ "path": dir.path().display().to_string() }),
        ),
        (4, "project.new", json!({})),
    ] {
        let response = roundtrip(&mut app, request(id, method, params));
        assert_eq!(
            response.error.expect("busy while saving").kind(),
            ErrorKind::Busy,
            "{method} while a save is in flight"
        );
    }
}
