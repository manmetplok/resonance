//! Control-endpoint job registry (ba doc #265, todo #1149): the
//! `JobBoard` ledger, `job.status` / `job.wait` over the control
//! dispatch, completion through the real update-loop path (correlation
//! tokens resolved by the existing `ProjectIoMessage` completion arms),
//! blocking-wait semantics on the socket side, and retention rules.

use resonance_app::compose::messages::VocalAudioReadyData;
use resonance_app::compose::ComposeMessage;
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
fn disconnect_orphans_live_jobs_and_they_complete_normally() {
    // The MCP client tears its connection down on ANY transport failure
    // (a read timeout on an unrelated call included) and then tells the
    // model to keep polling `job.status` with the same job id. The
    // operation behind a live job is still running in the app, so the
    // disconnect must orphan the job, not delete it — deleting it turned
    // that documented recovery path into `not_found` mid-render.
    let mut app = app();
    let _ = app.update(Message::Control(ControlMessage::Connected { conn: 9 }));
    let started =
        app.start_control_job("project.save", "Save project", JobToken::ProjectSave, Some(9));
    let job_id = u64::from(started.job_id);

    let _ = app.update(Message::Control(ControlMessage::Disconnected { conn: 9 }));

    // The reconnected client (`roundtrip` speaks as conn 1) still finds
    // the job, live.
    let status: JobStatus = roundtrip(&mut app, status_request(1, job_id))
        .result()
        .expect("a live job survives its connection");
    assert_eq!(status.state, JobState::Pending);

    // ... and the orphan completes through the normal update-loop path.
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectSaved(
        Ok(()),
        false,
    )));
    let status: JobStatus = roundtrip(&mut app, status_request(2, job_id))
        .result()
        .expect("status succeeds");
    assert_eq!(status.state, JobState::Done);
    assert_eq!(status.progress, Some(1.0));
}

#[test]
fn disconnect_drops_the_connections_terminal_jobs() {
    // Terminal jobs are a different matter: their result was for the
    // departed client alone, so the disconnect reaps them as before.
    let mut app = app();
    let _ = app.update(Message::Control(ControlMessage::Connected { conn: 9 }));
    let started =
        app.start_control_job("project.save", "Save project", JobToken::ProjectSave, Some(9));
    let job_id = u64::from(started.job_id);
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectSaved(
        Ok(()),
        false,
    )));
    assert_eq!(
        app.control_jobs().status(job_id).expect("job known").state,
        JobState::Done
    );

    let _ = app.update(Message::Control(ControlMessage::Disconnected { conn: 9 }));
    let response = roundtrip(&mut app, status_request(1, job_id));
    assert_eq!(
        response.error.expect("dropped with its connection").kind(),
        ErrorKind::NotFound
    );
}

#[test]
fn a_terminal_orphan_is_still_evicted_by_the_retention_cap() {
    // An orphan must not dodge the LRU bound once it goes terminal, or
    // flaky connections would grow the table forever.
    let board = JobBoard::default();
    let orphan = u64::from(
        board
            .start("project.save", "orphan-to-be", None, Some(9))
            .job_id,
    );
    board.on_disconnect(9);
    board.complete(orphan, serde_json::json!({ "ok": true }));

    for i in 0..(MAX_RETAINED_JOBS + 20) {
        let id = u64::from(board.start("test", &format!("op {i}"), None, None).job_id);
        board.complete(id, serde_json::json!(i));
    }
    assert!(
        board.status(orphan).is_none(),
        "a terminal orphan is prunable like any other terminal job"
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

// ---------------- superseded vocal renders ----------------

/// One update-loop `job.status` read, for the vocal tests below where
/// the interesting part is the interleaving, not the wire shape.
fn job_state(app: &mut Resonance, job_id: u64) -> JobState {
    roundtrip(app, status_request(99, job_id))
        .result::<JobStatus>()
        .expect("job.status succeeds")
        .state
}

/// Register a `vocal.render` job covering `lanes`, as the control
/// endpoint does once its renders have dispatched.
fn vocal_render_job(app: &mut Resonance, description: &str, lanes: Vec<(u64, u64)>) -> u64 {
    u64::from(
        app.start_control_job(
            "vocal.render",
            description,
            JobToken::VocalRender { lanes },
            None,
        )
        .job_id,
    )
}

/// The completion the background SVS task dispatches for a finished
/// render of `(definition_id, track_id)`, carrying the epoch snapshot
/// the render was queued with. No placements, so the install has no
/// engine work to do — the epoch check is the part under test.
fn vocal_audio_ready(definition_id: u64, track_id: u64, render_epoch: u64) -> Message {
    Message::Compose(ComposeMessage::VocalAudioReady(Box::new(
        VocalAudioReadyData {
            definition_id,
            track_id,
            wav_path: std::path::PathBuf::from("/tmp/nonexistent-control-jobs-vocal.wav"),
            placements: Vec::new(),
            clip_name: "Vocal".to_owned(),
            trim_start_frames: 0,
            trim_end_frames: 0,
            render_epoch,
        },
    )))
}

/// The failure the background SVS task dispatches when a render of
/// `(definition_id, track_id)` errors.
fn vocal_audio_failed(
    definition_id: u64,
    track_id: u64,
    render_epoch: u64,
    error: &str,
) -> Message {
    Message::Compose(ComposeMessage::VocalAudioFailed {
        definition_id,
        track_id,
        render_epoch,
        error: error.to_owned(),
    })
}

#[test]
fn a_superseded_render_resolves_no_job() {
    // The vocal sibling of the overlapping-import race: J1's render was
    // still in flight when J2 re-rendered the same lane, bumping its
    // epoch. J1's audio will be DISCARDED as stale when it arrives, so
    // its event must resolve nothing — ticking the lane off every batch
    // before the epoch check let J2 report `done` with a revision while
    // the install then threw J1's audio away, and a client that waited
    // on J2 and immediately read the track back saw old/no audio.
    let (def, track) = (7, 50);
    let mut app = app();

    app.test_set_vocal_render_epoch(def, track, 1);
    let j1 = vocal_render_job(&mut app, "client A", vec![(def, track)]);
    // J2 supersedes: its dispatch bumped the lane's epoch synchronously.
    app.test_set_vocal_render_epoch(def, track, 2);
    let j2 = vocal_render_job(&mut app, "client B", vec![(def, track)]);

    let _ = app.update(vocal_audio_ready(def, track, 1));
    assert_eq!(
        job_state(&mut app, j2),
        JobState::Pending,
        "the second job resolved off the first epoch's (discarded) render"
    );
    assert_eq!(
        job_state(&mut app, j1),
        JobState::Pending,
        "a discarded render must not resolve even the job that queued it"
    );

    // The current render lands: the audio every waiter asked for is now
    // installed, and there is no further event coming for the lane (the
    // superseded render's was discarded above) — so BOTH jobs resolve
    // off the one surviving install.
    let _ = app.update(vocal_audio_ready(def, track, 2));
    assert_eq!(job_state(&mut app, j2), JobState::Done);
    assert_eq!(
        job_state(&mut app, j1),
        JobState::Done,
        "the superseded job resolves off the accepted install, not never"
    );
}

#[test]
fn a_failed_lane_fails_only_the_jobs_waiting_on_it() {
    // Control job renders lane A; independently (say a GUI regeneration)
    // lane B renders and FAILS. The failure used to carry no lane
    // identity and was inferred onto the oldest live vocal-render job —
    // killing lane A's job over an error it had nothing to do with.
    let (def_a, track_a) = (7, 50);
    let (def_b, track_b) = (8, 51);
    let mut app = app();

    app.test_set_vocal_render_epoch(def_a, track_a, 1);
    app.test_set_vocal_render_epoch(def_b, track_b, 1);
    let job_a = vocal_render_job(&mut app, "lane A", vec![(def_a, track_a)]);
    let job_b = vocal_render_job(&mut app, "lane B", vec![(def_b, track_b)]);

    let _ = app.update(vocal_audio_failed(def_b, track_b, 1, "onnx session exploded"));
    assert_eq!(
        job_state(&mut app, job_a),
        JobState::Pending,
        "lane B's failure killed a job covering only lane A"
    );
    assert_eq!(
        job_state(&mut app, job_b),
        JobState::Error,
        "the job actually waiting on lane B fails"
    );

    // Lane A's own failure still reaches its job, message intact.
    let _ = app.update(vocal_audio_failed(def_a, track_a, 1, "voicebank missing"));
    let status = app.control_jobs().status(job_a).expect("job A known");
    assert_eq!(status.state, JobState::Error);
    assert_eq!(status.error.as_deref(), Some("voicebank missing"));
}

#[test]
fn a_superseded_renders_failure_fails_nothing() {
    // The failure-side epoch gate: the lane was re-rendered while the
    // failing render was in flight, so the newer render's outcome is
    // what the job gets — a stale failure must not pre-empt it.
    let (def, track) = (7, 50);
    let mut app = app();

    app.test_set_vocal_render_epoch(def, track, 2);
    let job = vocal_render_job(&mut app, "client", vec![(def, track)]);

    let _ = app.update(vocal_audio_failed(def, track, 1, "torn down mid-render"));
    assert_eq!(
        job_state(&mut app, job),
        JobState::Pending,
        "a superseded render's failure killed the job waiting on the current render"
    );

    // The current render then succeeds, and the job completes normally.
    let _ = app.update(vocal_audio_ready(def, track, 2));
    assert_eq!(job_state(&mut app, job), JobState::Done);
}
