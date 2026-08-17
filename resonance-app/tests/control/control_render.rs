//! `render.*` control handlers (ba doc #265, todo #1157): render.mixdown
//! WAV to an explicit path as a job — param validation, the overwrite
//! `needs_confirmation` guard and the recording/in-flight `busy` guards,
//! job completion keyed off the engine bounce events with the WAV's real
//! duration + sample rate, and render.stems reported `unsupported`.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance};
use resonance_audio::types::AudioEvent;
use resonance_control::job::{JobStarted, JobState, JobStatus};
use resonance_control::methods::render::MixdownResult;
use resonance_control::{ErrorKind, Request, Response};
use serde_json::json;
use std::io::Write;
use std::path::Path;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app
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

/// Write a minimal but valid 16-bit stereo WAV of `frames` silent frames
/// at `sample_rate`, so completion-path parsing sees a real, playable
/// (non-empty) file at the render target.
fn write_wav(path: &Path, sample_rate: u32, frames: u32) {
    let channels: u16 = 2;
    let bits: u16 = 16;
    let block_align = channels * bits / 8;
    let byte_rate = sample_rate * block_align as u32;
    let data_len = frames * block_align as u32;

    let mut f = std::fs::File::create(path).expect("create wav");
    f.write_all(b"RIFF").unwrap();
    f.write_all(&(36 + data_len).to_le_bytes()).unwrap();
    f.write_all(b"WAVE").unwrap();
    f.write_all(b"fmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap(); // PCM
    f.write_all(&channels.to_le_bytes()).unwrap();
    f.write_all(&sample_rate.to_le_bytes()).unwrap();
    f.write_all(&byte_rate.to_le_bytes()).unwrap();
    f.write_all(&block_align.to_le_bytes()).unwrap();
    f.write_all(&bits.to_le_bytes()).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&data_len.to_le_bytes()).unwrap();
    f.write_all(&vec![0u8; data_len as usize]).unwrap();
}

// ---------------- render.mixdown happy path ----------------

#[test]
fn mixdown_completes_with_the_written_wavs_duration_and_rate() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("mix.wav");

    let mut app = app();
    let job = started_job(roundtrip(
        &mut app,
        request(1, "render.mixdown", json!({ "path": target.display().to_string() })),
    ));
    // The bounce is in flight (the engine command fired synchronously).
    assert_eq!(job_status(&mut app, job).state, JobState::Pending);
    assert!(app.test_is_bouncing());

    // The engine renders and writes the file, then reports completion by
    // echoing the requested path. Stand in for the real render with a
    // valid 1-second 44.1k stereo WAV.
    write_wav(&target, 44_100, 44_100);
    app.test_apply_engine_event(AudioEvent::BounceComplete {
        path: target.display().to_string(),
    });

    let status = job_status(&mut app, job);
    assert_eq!(status.state, JobState::Done);
    let result: MixdownResult =
        serde_json::from_value(status.result.expect("done carries a result")).unwrap();
    assert_eq!(result.path, target.display().to_string());
    assert_eq!(result.sample_rate, 44_100);
    assert!(
        (result.duration_s - 1.0).abs() < 1e-6,
        "1s of 44.1k frames -> ~1.0s, got {}",
        result.duration_s
    );
    // The DoD's "playable non-empty WAV at the requested path".
    assert!(target.is_file());
    assert!(std::fs::metadata(&target).unwrap().len() > 44);
}

#[test]
fn mixdown_failure_fails_the_job_with_the_engine_message() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("mix.wav");

    let mut app = app();
    let job = started_job(roundtrip(
        &mut app,
        request(1, "render.mixdown", json!({ "path": target.display().to_string() })),
    ));
    // A path-less BounceError still resolves the one in-flight export job.
    app.test_apply_engine_event(AudioEvent::BounceError("no output device".to_owned()));
    let status = job_status(&mut app, job);
    assert_eq!(status.state, JobState::Error);
    assert_eq!(status.error.as_deref(), Some("no output device"));
}

// ---------------- guards ----------------

#[test]
fn mixdown_over_an_existing_file_requires_overwrite() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("mix.wav");
    write_wav(&target, 48_000, 10);

    let mut app = app();
    let response = roundtrip(
        &mut app,
        request(1, "render.mixdown", json!({ "path": target.display().to_string() })),
    );
    assert_eq!(
        response.error.expect("overwrite guard").kind(),
        ErrorKind::NeedsConfirmation
    );
    // Nothing started.
    assert!(!app.test_is_bouncing());

    // With overwrite the render starts.
    let job = started_job(roundtrip(
        &mut app,
        request(
            2,
            "render.mixdown",
            json!({ "path": target.display().to_string(), "overwrite": true }),
        ),
    ));
    assert_eq!(job_status(&mut app, job).state, JobState::Pending);
}

#[test]
fn mixdown_while_recording_is_busy() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("mix.wav");

    let mut app = app();
    app.test_set_transport_recording(true);
    let response = roundtrip(
        &mut app,
        request(1, "render.mixdown", json!({ "path": target.display().to_string() })),
    );
    assert_eq!(
        response.error.expect("recording guard").kind(),
        ErrorKind::Busy
    );
}

#[test]
fn a_second_mixdown_while_one_is_in_flight_is_busy() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut app = app();

    let first = dir.path().join("a.wav").display().to_string();
    let _ = started_job(roundtrip(
        &mut app,
        request(1, "render.mixdown", json!({ "path": first })),
    ));
    assert!(app.test_is_bouncing());

    let second = dir.path().join("b.wav").display().to_string();
    let response = roundtrip(&mut app, request(2, "render.mixdown", json!({ "path": second })));
    assert_eq!(
        response.error.expect("in-flight guard").kind(),
        ErrorKind::Busy
    );
}

#[test]
fn mixdown_validates_the_path() {
    let mut app = app();

    let response = roundtrip(
        &mut app,
        request(1, "render.mixdown", json!({ "path": "relative/mix.wav" })),
    );
    assert_eq!(
        response.error.expect("relative path").kind(),
        ErrorKind::InvalidParams
    );

    let response = roundtrip(
        &mut app,
        request(2, "render.mixdown", json!({ "path": "/definitely/not/here/mix.wav" })),
    );
    assert_eq!(
        response.error.expect("missing parent").kind(),
        ErrorKind::InvalidParams
    );
}

#[test]
fn mixdown_rejects_a_partial_range_as_unsupported() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("mix.wav").display().to_string();
    let mut app = app();

    let response = roundtrip(
        &mut app,
        request(
            1,
            "render.mixdown",
            json!({ "path": target, "range": { "start": { "bar": 2 }, "end": { "bar": 5 } } }),
        ),
    );
    assert_eq!(
        response.error.expect("partial range").kind(),
        ErrorKind::Unsupported
    );
    assert!(!app.test_is_bouncing());
}

#[test]
fn mixdown_accepts_an_explicitly_empty_whole_range() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("mix.wav").display().to_string();
    let mut app = app();

    // An empty range object means "the whole song" and must not be
    // rejected as a partial range.
    let job = started_job(roundtrip(
        &mut app,
        request(1, "render.mixdown", json!({ "path": target, "range": {} })),
    ));
    assert_eq!(job_status(&mut app, job).state, JobState::Pending);
}

// ---------------- render.stems ----------------

#[test]
fn stems_is_unsupported_on_this_build() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut app = app();
    let response = roundtrip(
        &mut app,
        request(1, "render.stems", json!({ "dir": dir.path().display().to_string() })),
    );
    assert_eq!(
        response.error.expect("stems unsupported").kind(),
        ErrorKind::Unsupported
    );
}

#[test]
fn stems_with_malformed_params_is_invalid_params() {
    let mut app = app();
    // Missing the required `dir`.
    let response = roundtrip(&mut app, request(1, "render.stems", json!({})));
    assert_eq!(
        response.error.expect("bad params").kind(),
        ErrorKind::InvalidParams
    );
}
