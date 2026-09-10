//! Control-endpoint job registry (ba doc #265, todo #1149): the
//! `JobBoard` ledger, `job.status` / `job.wait` over the control
//! dispatch, completion through the real update-loop path (correlation
//! tokens resolved by the existing `ProjectIoMessage` completion arms),
//! blocking-wait semantics on the socket side, and retention rules.

use resonance_app::control_jobs::{JobBoard, JobToken, MAX_RETAINED_JOBS};
use resonance_app::control_socket::ControlMessage;
use resonance_app::message::{Message, ProjectIoMessage};
use resonance_app::state::ViewMode;
use resonance_app::{Resonance};
use resonance_control::job::{JobState, JobStatus};
use resonance_control::{ErrorKind, Request, Response};
use std::sync::Arc;
use std::time::{Duration, Instant};
use crate::common::roundtrip;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-jobs-test.rprj"));
    app
}

fn status_request(id: i64, job_id: u64) -> Request {
    Request::new(id, "job.status", &serde_json::json!({ "job_id": job_id }))
        .expect("params serialize")
}

// ---------------- registry + job.status through the update loop ----------------

#[test]
fn save_job_completes_through_the_real_update_path() {
    let mut app = app();
    let started = app.start_control_job(
        "project.save",
        "Save project",
        JobToken::ProjectSave,
        None,
    );
    let job_id = u64::from(started.job_id);

    let response = roundtrip(&mut app, status_request(1, job_id));
    let status: JobStatus = response.result().expect("status succeeds");
    assert_eq!(status.state, JobState::Pending);

    // An *autosave* completing must not resolve the client's save job.
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectSaved(
        Ok(()),
        true,
    )));
    let status: JobStatus = roundtrip(&mut app, status_request(2, job_id))
        .result()
        .expect("status succeeds");
    assert_eq!(status.state, JobState::Pending);

    // The manual-save completion message resolves it, carrying the path.
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectSaved(
        Ok(()),
        false,
    )));
    let status: JobStatus = roundtrip(&mut app, status_request(3, job_id))
        .result()
        .expect("status succeeds");
    assert_eq!(status.state, JobState::Done);
    assert_eq!(status.progress, Some(1.0));
    let result = status.result.expect("done jobs carry a result");
    assert!(result["path"]
        .as_str()
        .expect("path echoed")
        .ends_with("control-jobs-test.rprj"));

    // Status stays queryable after the first fetch (retention is
    // LRU-bounded, not fetch-once).
    let again: JobStatus = roundtrip(&mut app, status_request(4, job_id))
        .result()
        .expect("status succeeds");
    assert_eq!(again.state, JobState::Done);
}

#[test]
fn save_failure_fails_the_job_with_the_message() {
    let mut app = app();
    let started =
        app.start_control_job("project.save", "Save project", JobToken::ProjectSave, None);
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectSaved(
        Err("disk full".to_owned()),
        false,
    )));
    let status: JobStatus = roundtrip(&mut app, status_request(1, u64::from(started.job_id)))
        .result()
        .expect("status succeeds");
    assert_eq!(status.state, JobState::Error);
    assert_eq!(status.error.as_deref(), Some("disk full"));
    assert!(status.result.is_none());
}

#[test]
fn completion_without_a_matching_job_is_a_no_op() {
    let mut app = app();
    // No control job in flight: the ordinary GUI save completion must
    // not invent one.
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectSaved(
        Ok(()),
        false,
    )));
    let response = roundtrip(&mut app, status_request(1, 1));
    assert_eq!(response.error.expect("no job").kind(), ErrorKind::NotFound);
}

#[test]
fn unknown_job_id_is_not_found_for_status_and_wait() {
    let mut app = app();
    let response = roundtrip(&mut app, status_request(1, 424242));
    assert_eq!(response.error.expect("unknown job").kind(), ErrorKind::NotFound);

    let wait = Request::new(
        2,
        "job.wait",
        &serde_json::json!({ "job_id": 424242, "timeout_ms": 10 }),
    )
    .unwrap();
    let response = roundtrip(&mut app, wait);
    assert_eq!(response.error.expect("unknown job").kind(), ErrorKind::NotFound);
}

#[test]
fn update_loop_wait_is_a_nonblocking_snapshot() {
    let mut app = app();
    let started =
        app.start_control_job("project.save", "Save project", JobToken::ProjectSave, None);
    let wait = Request::new(
        1,
        "job.wait",
        &serde_json::json!({ "job_id": u64::from(started.job_id), "timeout_ms": 60_000 }),
    )
    .unwrap();
    let begun = Instant::now();
    let status: JobStatus = roundtrip(&mut app, wait).result().expect("wait succeeds");
    // Direct dispatch answers immediately with the current status —
    // blocking waits live on the socket threads only.
    assert!(begun.elapsed() < Duration::from_secs(5));
    assert_eq!(status.state, JobState::Pending);
}

// ---------------- blocking wait semantics (socket-thread side) ----------------

#[test]
fn board_wait_returns_early_on_completion() {
    let board = Arc::new(JobBoard::default());
    let started = board.start("test", "fake op", None, None);
    let id = u64::from(started.job_id);

    let completer = Arc::clone(&board);
    let handle = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        completer.complete(id, serde_json::json!({ "ok": true }));
    });

    let begun = Instant::now();
    let status = board
        .wait(id, Some(Duration::from_secs(30)))
        .expect("job known");
    let waited = begun.elapsed();
    handle.join().unwrap();

    assert_eq!(status.state, JobState::Done);
    assert!(
        waited < Duration::from_secs(5),
        "wait must resolve on completion, not the timeout (waited {waited:?})"
    );
    assert!(waited >= Duration::from_millis(50), "wait actually blocked");
}

#[test]
fn board_wait_times_out_cleanly_with_current_status() {
    let board = JobBoard::default();
    let started = board.start("test", "fake op", None, None);
    let id = u64::from(started.job_id);
    board.set_running(id, Some(0.25));

    let begun = Instant::now();
    let status = board
        .wait(id, Some(Duration::from_millis(120)))
        .expect("job known");
    assert!(begun.elapsed() >= Duration::from_millis(100));
    assert_eq!(status.state, JobState::Running);
    assert_eq!(status.progress, Some(0.25));
}

// ---------------- socket transport serves job.wait itself ----------------

#[test]
fn socket_job_wait_never_crosses_the_bridge() {
    use std::os::unix::net::UnixStream;

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("control.sock");
    let (tx, _rx) = iced::futures::channel::mpsc::unbounded();
    let board = Arc::new(JobBoard::default());
    let server = resonance_app::control_socket::spawn(path.clone(), tx, Arc::clone(&board))
        .expect("bind control socket");

    let started = board.start("test", "fake op", None, None);
    let job_id = u64::from(started.job_id);

    let client = UnixStream::connect(&path).expect("connect");
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("read timeout");
    let mut writer = client.try_clone().expect("clone stream");
    let wait = Request::new(
        1,
        "job.wait",
        &serde_json::json!({ "job_id": job_id, "timeout_ms": 30_000 }),
    )
    .unwrap();
    resonance_control::write_message(&mut writer, &wait).expect("send wait");

    // Nobody pumps the bridge (`_rx` sits untouched): the reader thread
    // itself must resolve the wait once the job completes.
    std::thread::sleep(Duration::from_millis(100));
    board.complete(job_id, serde_json::json!({ "ok": true }));

    let mut reader = resonance_control::MessageReader::from_reader(client);
    let response: Response = reader
        .read_message()
        .expect("read wait reply")
        .expect("wait reply present");
    let status: JobStatus = response.result().expect("wait succeeds");
    assert_eq!(status.state, JobState::Done);

    drop(server);
}

// ---------------- retention ----------------

#[test]
fn disconnect_drops_the_connections_jobs() {
    let mut app = app();
    let _ = app.update(Message::Control(ControlMessage::Connected { conn: 9 }));
    let started =
        app.start_control_job("project.save", "Save project", JobToken::ProjectSave, Some(9));
    let job_id = u64::from(started.job_id);
    assert!(app.control_jobs().status(job_id).is_some());

    let _ = app.update(Message::Control(ControlMessage::Disconnected { conn: 9 }));
    let response = roundtrip(&mut app, status_request(1, job_id));
    assert_eq!(
        response.error.expect("dropped with its connection").kind(),
        ErrorKind::NotFound
    );
}

#[test]
fn retention_is_bounded_and_prefers_evicting_fetched_terminal_jobs() {
    let board = JobBoard::default();
    // A live job must survive any amount of churn.
    let live = u64::from(board.start("test", "live", None, None).job_id);

    let mut first_done = None;
    for i in 0..(MAX_RETAINED_JOBS + 20) {
        let id = u64::from(board.start("test", &format!("op {i}"), None, None).job_id);
        board.complete(id, serde_json::json!(i));
        first_done.get_or_insert(id);
        // Fetch every other job so eviction has fetched victims first.
        if i % 2 == 0 {
            let _ = board.status(id);
        }
    }

    assert!(
        board.status(live).is_some(),
        "live jobs are never evicted by the LRU cap"
    );
    assert!(
        board.status(u64::from(first_done.unwrap())).is_none(),
        "oldest terminal jobs get evicted once over the cap"
    );
}

// ---------------- poison containment ----------------

#[test]
fn a_reader_thread_panic_does_not_poison_the_board() {
    let board = Arc::new(JobBoard::default());
    let id = u64::from(board.start("test", "survives poisoning", None, None).job_id);

    // Panic while holding the table lock on another thread — exactly
    // what a crashing `job.wait` reader would do. The table is shared
    // between those reader threads and the main update loop, so the
    // poison must not cascade into the app.
    let poisoner = Arc::clone(&board);
    std::thread::spawn(move || poisoner.panic_holding_table_for_test())
        .join()
        .expect_err("the probe thread panics by design");

    // Every later access recovers the lock instead of panicking in turn.
    board.complete(id, serde_json::json!({ "ok": true }));
    let status = board.status(id).expect("the table still serves");
    assert_eq!(status.state, JobState::Done);
    let waited = board
        .wait(id, Some(Duration::from_millis(10)))
        .expect("wait still serves");
    assert_eq!(waited.state, JobState::Done);
}

// ---------------- duplicate tokens resolve FIFO ----------------

#[test]
fn duplicate_live_tokens_resolve_oldest_first() {
    // Two clients started the same operation, so two live jobs carry an
    // identical token. Completion events arrive in dispatch order, so
    // the first completion belongs to the FIRST job — resolving the
    // newest instead left the older client waiting out the full
    // `MAX_WAIT` on a job that had in fact finished.
    let board = JobBoard::default();
    let first = u64::from(
        board
            .start("project.save", "client A", Some(JobToken::ProjectSave), None)
            .job_id,
    );
    let second = u64::from(
        board
            .start("project.save", "client B", Some(JobToken::ProjectSave), None)
            .job_id,
    );

    assert!(board.complete_token(&JobToken::ProjectSave, serde_json::json!({ "n": 1 })));
    assert_eq!(board.status(first).unwrap().state, JobState::Done);
    assert_eq!(
        board.status(second).unwrap().state,
        JobState::Pending,
        "the newer duplicate stays live until its own completion arrives"
    );

    assert!(board.fail_token(&JobToken::ProjectSave, "disk full"));
    assert_eq!(board.status(second).unwrap().state, JobState::Error);
}

// ---------------- overlapping import batches ----------------

#[test]
fn one_import_event_ticks_only_the_oldest_batch_awaiting_that_path() {
    // Two overlapping `pool.import` batches name the same source file:
    // the engine imports it twice and reports twice. Each event must
    // tick ONE batch — ticking every batch resolved the second batch
    // `done` off the first batch's event, before its own copy of the
    // file had imported.
    let board = JobBoard::default();
    let path = "/tmp/shared-kick.wav";
    let first = u64::from(
        board
            .start(
                "pool.import",
                "batch A",
                Some(JobToken::PoolImport { paths: vec![path.to_owned()] }),
                None,
            )
            .job_id,
    );
    let second = u64::from(
        board
            .start(
                "pool.import",
                "batch B",
                Some(JobToken::PoolImport { paths: vec![path.to_owned()] }),
                None,
            )
            .job_id,
    );

    // First per-file event: exactly the oldest batch finishes.
    let finished = board.tick_import_path(path, None);
    assert_eq!(finished, vec![(first, None)]);
    board.complete(first, serde_json::json!({ "assets": [] }));
    assert_eq!(
        board.status(second).unwrap().state,
        JobState::Pending,
        "the overlapping batch still awaits its own event"
    );

    // Second event: now the second batch finishes.
    let finished = board.tick_import_path(path, None);
    assert_eq!(finished, vec![(second, None)]);

    // A third event has nobody left to tick.
    assert!(board.tick_import_path(path, None).is_empty());
}
