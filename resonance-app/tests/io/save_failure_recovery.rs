//! A save the engine could not collect must not wedge the app (code
//! review STATE2-02), and autosave failures must reach the user once they
//! persist (UX-13).
//!
//! Before STATE2-02 a failed clip copy or transcode during save (a full
//! disk) sent only `AudioEvent::Error`: the `SaveCollector` waited forever
//! for `ClipsSavedToProjectDir`, so every later manual save queued behind
//! it, autosave backed off forever, and the control `project.*` methods
//! answered `busy` for the rest of the session.

use resonance_app::message::{Message, ProjectIoMessage};
use resonance_app::Resonance;
use resonance_audio::types::{AudioCommand, AudioEvent, EngineError};
use resonance_control::job::{JobStarted, JobState, JobStatus};
use resonance_control::{ErrorKind, Request, Response};
use serde_json::json;

use crate::common::roundtrip;

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

fn dispatch(app: &mut Resonance, m: ProjectIoMessage) {
    let _ = app.update(Message::ProjectIo(m));
}

/// An app with a titled project in `dir`.
fn titled_app(dir: &tempfile::TempDir) -> Resonance {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_set_project_path(dir.path().join("song.rproj"));
    app
}

const DISK_FULL: &str = "Transcode clip 7 to WAV: No space left on device (os error 28)";

#[test]
fn a_failed_clip_save_fails_the_save_and_frees_the_collector() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut app = titled_app(&dir);

    dispatch(&mut app, ProjectIoMessage::SaveProject);
    assert!(app.test_save_in_flight().is_some());
    app.test_apply_engine_event(AudioEvent::ClipsSaveFailed { error: DISK_FULL.to_owned() });
    app.test_apply_engine_event(AudioEvent::AllPluginStatesSaved { states: Vec::new() });

    assert_eq!(app.test_save_in_flight(), None, "the collector is gone");
    assert!(!app.is_saving(), "the save is over");
    let banner = app.test_error_message().expect("the failure is shown");
    assert!(banner.contains("Save failed") && banner.contains("No space left"), "{banner}");

    // The user frees space and saves again: a fresh round-trip starts.
    dispatch(&mut app, ProjectIoMessage::SaveProject);
    assert!(app.test_save_in_flight().is_some(), "the next save runs");
}

#[test]
fn a_failed_clip_save_fails_the_control_job_and_the_next_save_is_not_busy() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut app = titled_app(&dir);

    let job = started_job(roundtrip(&mut app, request(1, "project.save", json!({}))));
    assert_eq!(job_status(&mut app, job).state, JobState::Pending);
    app.test_apply_engine_event(AudioEvent::ClipsSaveFailed { error: DISK_FULL.to_owned() });
    app.test_apply_engine_event(AudioEvent::AllPluginStatesSaved { states: Vec::new() });

    let status = job_status(&mut app, job);
    assert_eq!(status.state, JobState::Error, "{status:?}");

    let response = roundtrip(&mut app, request(2, "project.save", json!({})));
    assert!(
        response.error.as_ref().is_none_or(|e| e.kind() != ErrorKind::Busy),
        "the next save must not be busy: {:?}",
        response.error
    );
    let job = started_job(response);
    assert_eq!(job_status(&mut app, job).state, JobState::Pending);
}

#[test]
fn a_manual_save_queued_behind_a_failed_autosave_runs() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (mut app, _task, cmds) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    let saved = dir.path().join("song.rproj");
    app.test_set_project_path(saved.clone());

    dispatch(&mut app, ProjectIoMessage::Autosave);
    dispatch(&mut app, ProjectIoMessage::SaveProject);
    app.test_apply_engine_event(AudioEvent::ClipsSaveFailed { error: DISK_FULL.to_owned() });
    app.test_apply_engine_event(AudioEvent::AllPluginStatesSaved { states: Vec::new() });

    assert_eq!(
        app.test_save_in_flight(),
        Some((saved, false)),
        "the queued manual save started its own round-trip"
    );
    let clip_saves = cmds
        .try_iter()
        .filter(|c| matches!(c, AudioCommand::SaveClipsToProjectDir))
        .count();
    assert_eq!(clip_saves, 2, "one engine round-trip per save");
    assert!(
        app.test_error_message().is_none(),
        "an autosave failure never raises the error banner"
    );
}

/// The safety net: a save whose engine reply never comes (the engine
/// raised only `AudioEvent::Error`, or died) is abandoned by the tick
/// watchdog instead of blocking every later save.
#[test]
fn the_watchdog_abandons_a_save_the_engine_never_answers() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut app = titled_app(&dir);

    let job = started_job(roundtrip(&mut app, request(1, "project.save", json!({}))));
    app.test_apply_engine_event(AudioEvent::Error(EngineError::io("Copy clip 3 WAV: EIO")));

    // Young: the tick leaves it collecting.
    let _ = app.update(Message::Tick);
    assert!(app.test_save_in_flight().is_some(), "a young save is left alone");

    app.test_age_save_in_flight(std::time::Duration::from_secs(10 * 60));
    let _ = app.update(Message::Tick);
    assert_eq!(app.test_save_in_flight(), None, "the watchdog dropped the collector");
    assert!(!app.is_saving());
    assert_eq!(job_status(&mut app, job).state, JobState::Error);

    let response = roundtrip(&mut app, request(2, "project.save", json!({})));
    let job = started_job(response);
    assert_eq!(job_status(&mut app, job).state, JobState::Pending);
}

// ---- UX-13: autosave failures reach the user --------------------------

#[test]
fn repeated_autosave_failures_raise_a_persistent_indicator_until_one_succeeds() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut app = titled_app(&dir);
    let fail = || ProjectIoMessage::ProjectSaved(Err("Disk quota exceeded".to_owned()), true);

    dispatch(&mut app, fail());
    dispatch(&mut app, fail());
    assert_eq!(app.test_autosave_failing(), None, "two misses are not yet a pattern");

    dispatch(&mut app, fail());
    let shown = app.test_autosave_failing().expect("third miss raises it");
    assert!(shown.contains("Disk quota exceeded"), "{shown}");
    assert!(app.test_error_message().is_none(), "not on the dismissable error banner");
    iced_test::simulator(app.view())
        .find(shown.as_str())
        .expect("the indicator is rendered");

    // An unrelated error lands and is dismissed: the indicator stays.
    app.test_apply_engine_event(AudioEvent::Error(EngineError::internal("preset star failed")));
    let _ = app.update(Message::Ui(resonance_app::message::UiMessage::DismissError));
    assert_eq!(app.test_autosave_failing(), Some(shown));

    dispatch(&mut app, ProjectIoMessage::ProjectSaved(Ok(()), true));
    assert_eq!(app.test_autosave_failing(), None, "a successful autosave clears it");
}

#[test]
fn an_autosave_clip_failure_counts_toward_the_indicator() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut app = titled_app(&dir);
    for _ in 0..3 {
        dispatch(&mut app, ProjectIoMessage::Autosave);
        app.test_apply_engine_event(AudioEvent::ClipsSaveFailed { error: DISK_FULL.to_owned() });
        app.test_apply_engine_event(AudioEvent::AllPluginStatesSaved { states: Vec::new() });
    }
    let shown = app.test_autosave_failing().expect("indicator raised");
    assert!(shown.contains("No space left"), "{shown}");
}
