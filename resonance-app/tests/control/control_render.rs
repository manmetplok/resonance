//! `render.*` control handlers (ba doc #265, todo #1157): render.mixdown
//! WAV to an explicit path as a job — param validation, the overwrite
//! `needs_confirmation` guard and the recording/in-flight `busy` guards,
//! job completion keyed off the engine bounce events with the WAV's real
//! duration + sample rate, a ranged mixdown riding `ExportAudio`; and
//! render.stems as a job on the Export dialog's `ExportStems`, its names
//! and defaults, resolved off the `StemExport*` events.

use resonance_app::message::{ExportMessage, Message};
use resonance_app::state::{ExportPhase, ExportSource, Overlay};
use resonance_app::Resonance;
use resonance_audio::types::{
    AudioCommand, AudioEvent, EngineError, ExportErrorKind, StemBitDepth, StemSource, TrackType,
};
use resonance_control::job::{JobStarted, JobState, JobStatus};
use resonance_control::methods::render::{MixdownResult, StemsResult};
use resonance_control::{ErrorKind, Request, Response};
use serde_json::json;
use std::io::Write;
use std::path::Path;
use crate::common::roundtrip;
use crate::common::app_active_project as app;

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
fn mixdown_failure_fails_the_job_with_the_engine_message_and_kind() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("mix.wav");

    let mut app = app();
    let job = started_job(roundtrip(
        &mut app,
        request(1, "render.mixdown", json!({ "path": target.display().to_string() })),
    ));
    // A path-less BounceError still resolves the one in-flight export job.
    // `EncoderUnavailable` maps to `unsupported` (ARCH-05 / epic C, C-2).
    app.test_apply_engine_event(AudioEvent::BounceError {
        kind: ExportErrorKind::EncoderUnavailable,
        message: "no output device".to_owned(),
    });
    let status = job_status(&mut app, job);
    assert_eq!(status.state, JobState::Error);
    let error = status.error.expect("failed job carries an error");
    assert_eq!(error.message, "no output device");
    assert_eq!(error.kind, Some(ErrorKind::Unsupported));
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

/// The renderer writes WAV, so the target must say so: an agent cannot
/// point a mixdown at `~/.bashrc`, even with `overwrite` (code review
/// STATE2-10).
#[test]
fn mixdown_requires_a_wav_extension() {
    let mut app = app();
    let dir = tempfile::tempdir().expect("temp dir");
    let rc = dir.path().join(".bashrc");
    std::fs::write(&rc, b"export PATH").unwrap();
    for (i, target) in [
        rc.clone(),
        dir.path().join("mix"),
        dir.path().join("mix.flac"),
        dir.path().join("mix.wav.txt"),
    ]
    .into_iter()
    .enumerate()
    {
        let response = roundtrip(
            &mut app,
            request(
                i as i64,
                "render.mixdown",
                json!({ "path": target.display().to_string(), "overwrite": true }),
            ),
        );
        let err = response.error.expect("refused");
        assert_eq!(err.kind(), ErrorKind::InvalidParams, "{target:?}");
        assert!(err.message.contains(".wav"), "{}", err.message);
    }
    assert!(!app.test_is_bouncing(), "no bounce started");
    assert_eq!(std::fs::read(&rc).unwrap(), b"export PATH");
    // Case does not matter.
    let upper = dir.path().join("MIX.WAV").display().to_string();
    let response = roundtrip(&mut app, request(9, "render.mixdown", json!({ "path": upper })));
    assert!(response.error.is_none(), "{:?}", response.error);
}

// ---------------- render.mixdown range ----------------

/// An app whose engine commands are captured, with an active project.
fn capture_app() -> (Resonance, crossbeam_channel::Receiver<AudioCommand>) {
    let (mut app, _task, cmd_rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    while cmd_rx.try_recv().is_ok() {}
    (app, cmd_rx)
}

/// One bar at the test app's default 120 BPM in 4/4 on its 44.1 kHz
/// tempo map.
const BAR: u64 = 88_200;

#[test]
fn mixdown_renders_a_two_bar_range_as_an_export_with_that_window() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("chorus.wav");
    let (mut app, cmd_rx) = capture_app();
    let job = started_job(roundtrip(
        &mut app,
        request(
            1,
            "render.mixdown",
            json!({
                "path": target.display().to_string(),
                "range": { "start": { "bar": 3 }, "end": { "bar": 5 } }
            }),
        ),
    ));
    let sent: Vec<AudioCommand> = cmd_rx
        .try_iter()
        .filter(|c| {
            matches!(c, AudioCommand::ExportAudio { .. } | AudioCommand::BounceToWav { .. })
        })
        .collect();
    match sent.as_slice() {
        [AudioCommand::ExportAudio { path, range, settings }] => {
            assert_eq!(path, &target.display().to_string());
            // Bars 3-4: two bars, starting two bars in. The file's
            // length past the window is the engine's FX tail
            // (`resonance-audio/tests/bounce/export_range.rs`).
            assert_eq!(*range, Some((2 * BAR, 4 * BAR)));
            assert!(!settings.normalize.enabled, "a plain ranged mix");
        }
        other => panic!("one ranged ExportAudio, got {other:?}"),
    }
    assert!(app.test_is_bouncing());

    // Two bars plus the 2 s tail.
    write_wav(&target, 44_100, (2 * BAR + 88_200) as u32);
    app.test_apply_engine_event(AudioEvent::ExportComplete {
        path: target.display().to_string(),
        achieved_lufs: None,
        achieved_dbtp: 0.0,
        bytes: 0,
    });
    let status = job_status(&mut app, job);
    assert_eq!(status.state, JobState::Done);
    let result: MixdownResult = serde_json::from_value(status.result.unwrap()).unwrap();
    assert!((result.duration_s - 6.0).abs() < 1e-6, "{}", result.duration_s);
    assert!(result.normalize.is_none(), "not normalized");
    assert!(!app.test_is_bouncing());
}

#[test]
fn a_failed_ranged_mixdown_fails_its_job() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("verse.wav").display().to_string();
    let (mut app, _cmd_rx) = capture_app();
    let job = started_job(roundtrip(
        &mut app,
        request(1, "render.mixdown", json!({ "path": target, "range": { "end": { "bar": 2 } } })),
    ));
    app.test_apply_engine_event(AudioEvent::ExportError {
        kind: ExportErrorKind::Io,
        message: "disk full".to_owned(),
    });
    let status = job_status(&mut app, job);
    assert_eq!(status.state, JobState::Error);
    assert_eq!(status.error.unwrap().message, "disk full");
}

#[test]
fn mixdown_refuses_an_empty_range() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("mix.wav").display().to_string();
    let mut app = app();
    let response = roundtrip(
        &mut app,
        request(
            1,
            "render.mixdown",
            json!({ "path": target, "range": { "start": { "bar": 5 }, "end": { "bar": 5 } } }),
        ),
    );
    assert_eq!(response.error.expect("empty range").kind(), ErrorKind::InvalidParams);
    assert!(!app.test_is_bouncing());
}

#[test]
fn mixdown_accepts_an_explicitly_empty_whole_range() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("mix.wav").display().to_string();
    let (mut app, cmd_rx) = capture_app();

    // An empty range object means "the whole song": the plain bounce.
    let job = started_job(roundtrip(
        &mut app,
        request(1, "render.mixdown", json!({ "path": target, "range": {} })),
    ));
    assert_eq!(job_status(&mut app, job).state, JobState::Pending);
    assert!(cmd_rx.try_iter().any(|c| matches!(c, AudioCommand::BounceToWav { .. })));
}

// ---------------- render.stems ----------------

const VOX: u64 = 11;
const GTR: u64 = 12;

/// A two-track project (ids 11 "Lead Vox" and 12 "Gtr/DI").
fn stems_app() -> (Resonance, crossbeam_channel::Receiver<AudioCommand>) {
    let (mut app, cmd_rx) = capture_app();
    app.test_add_track(VOX, TrackType::Vocal);
    app.test_add_track(GTR, TrackType::Audio);
    for (id, name) in [(VOX, "Lead Vox"), (GTR, "Gtr/DI")] {
        let response = roundtrip(
            &mut app,
            request(1, "track.rename", json!({ "track_id": id, "name": name })),
        );
        assert!(response.error.is_none(), "{:?}", response.error);
    }
    while cmd_rx.try_recv().is_ok() {}
    (app, cmd_rx)
}

fn stems(app: &mut Resonance, params: serde_json::Value) -> Response {
    roundtrip(app, request(1, "render.stems", params))
}

/// The one `ExportStems` sent since the last drain.
fn sent_stems(cmd_rx: &crossbeam_channel::Receiver<AudioCommand>) -> AudioCommand {
    let sent: Vec<AudioCommand> = cmd_rx
        .try_iter()
        .filter(|c| matches!(c, AudioCommand::ExportStems { .. }))
        .collect();
    assert_eq!(sent.len(), 1, "{sent:?}");
    sent.into_iter().next().unwrap()
}

fn target_paths(command: &AudioCommand) -> Vec<(StemSource, String)> {
    match command {
        AudioCommand::ExportStems { targets, .. } => {
            targets.iter().map(|t| (t.source, t.path.clone())).collect()
        }
        other => panic!("not ExportStems: {other:?}"),
    }
}

#[test]
fn stems_exports_every_track_named_as_the_export_dialog_names_them() {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = dir.path().join("stems");
    let (mut app, cmd_rx) = stems_app();

    let job = started_job(stems(&mut app, json!({ "dir": out.display().to_string() })));
    assert!(out.is_dir(), "the directory is created, as the dialog creates it");
    let command = sent_stems(&cmd_rx);
    let paths = target_paths(&command);
    assert_eq!(
        paths,
        vec![
            (StemSource::Track(VOX), out.join("01-Lead Vox.wav").display().to_string()),
            (StemSource::Track(GTR), out.join("02-Gtr_DI.wav").display().to_string()),
        ]
    );
    // The Export dialog's defaults: whole song, 48 kHz 24-bit, FX tail.
    match &command {
        AudioCommand::ExportStems {
            range,
            sample_rate,
            bit_depth,
            include_fx_tail,
            ..
        } => {
            assert_eq!(*range, None);
            assert_eq!(*sample_rate, 48_000);
            assert_eq!(*bit_depth, StemBitDepth::Int24);
            assert!(*include_fx_tail);
        }
        _ => unreachable!(),
    }

    // The same tracks ticked in the GUI dialog give the same files.
    let (mut gui, gui_rx) = stems_app();
    gui.test_set_project_path(dir.path().to_path_buf());
    let _ = gui.update(Message::Export(ExportMessage::Open));
    for id in [VOX, GTR] {
        let _ = gui.update(Message::Export(ExportMessage::ToggleSource(ExportSource::Track(id))));
    }
    let _ = gui.update(Message::Export(ExportMessage::Confirm));
    assert_eq!(target_paths(&sent_stems(&gui_rx)), paths);

    // The engine renders both (stand-in WAVs: 3 s at 48 kHz).
    assert_eq!(job_status(&mut app, job).state, JobState::Pending);
    app.test_apply_engine_event(AudioEvent::StemExportProgress {
        target_index: 0,
        total: 2,
        fraction: 0.0,
    });
    write_wav(Path::new(&paths[0].1), 48_000, 144_000);
    app.test_apply_engine_event(AudioEvent::StemExportTargetDone {
        index: 0,
        path: paths[0].1.clone(),
    });
    let status = job_status(&mut app, job);
    assert_eq!(status.state, JobState::Running);
    assert_eq!(status.progress, Some(0.5));
    app.test_apply_engine_event(AudioEvent::StemExportProgress {
        target_index: 1,
        total: 2,
        fraction: 0.5,
    });
    write_wav(Path::new(&paths[1].1), 48_000, 144_000);
    app.test_apply_engine_event(AudioEvent::StemExportTargetDone {
        index: 1,
        path: paths[1].1.clone(),
    });
    app.test_apply_engine_event(AudioEvent::StemExportComplete {
        files: paths.iter().map(|(_, p)| p.clone()).collect(),
    });

    let status = job_status(&mut app, job);
    assert_eq!(status.state, JobState::Done);
    let result: StemsResult = serde_json::from_value(status.result.unwrap()).unwrap();
    assert_eq!(result.paths, paths.iter().map(|(_, p)| p.clone()).collect::<Vec<_>>());
    assert!((result.duration_s - 3.0).abs() < 1e-6, "{}", result.duration_s);
    for path in &result.paths {
        assert!(std::fs::metadata(path).unwrap().len() > 44, "{path} is non-empty");
    }
    // The job is over: nothing is rendering any more.
    assert!(app.root_overlay().is_none(), "{:?}", app.root_overlay());
}

#[test]
fn stems_track_ids_exports_only_those_tracks() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (mut app, cmd_rx) = stems_app();
    let _ = started_job(stems(
        &mut app,
        json!({ "dir": dir.path().display().to_string(), "track_ids": [VOX] }),
    ));
    assert_eq!(
        target_paths(&sent_stems(&cmd_rx)),
        vec![(
            StemSource::Track(VOX),
            dir.path().join("01-Lead Vox.wav").display().to_string()
        )]
    );
}

#[test]
fn stems_refuses_unknown_and_empty_track_ids() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (mut app, cmd_rx) = stems_app();
    for track_ids in [json!([VOX, 999]), json!([])] {
        let response = stems(
            &mut app,
            json!({ "dir": dir.path().display().to_string(), "track_ids": track_ids }),
        );
        assert_eq!(response.error.expect("refused").kind(), ErrorKind::InvalidParams);
    }
    assert!(!cmd_rx.try_iter().any(|c| matches!(c, AudioCommand::ExportStems { .. })));
}

#[test]
fn stems_over_an_existing_file_requires_overwrite() {
    let dir = tempfile::tempdir().expect("temp dir");
    let existing = dir.path().join("02-Gtr_DI.wav");
    std::fs::write(&existing, b"keep me").unwrap();
    let (mut app, cmd_rx) = stems_app();

    let response = stems(&mut app, json!({ "dir": dir.path().display().to_string() }));
    let error = response.error.expect("overwrite guard");
    assert_eq!(error.kind(), ErrorKind::NeedsConfirmation);
    assert!(error.message.contains("02-Gtr_DI.wav"), "{}", error.message);
    assert!(!cmd_rx.try_iter().any(|c| matches!(c, AudioCommand::ExportStems { .. })));
    assert_eq!(std::fs::read(&existing).unwrap(), b"keep me");

    // With overwrite the same names are used — no `_1` suffix.
    let _ = started_job(stems(
        &mut app,
        json!({ "dir": dir.path().display().to_string(), "overwrite": true }),
    ));
    let paths = target_paths(&sent_stems(&cmd_rx));
    assert_eq!(paths[1].1, existing.display().to_string());
}

#[test]
fn stems_takes_a_range() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (mut app, cmd_rx) = stems_app();
    let _ = started_job(stems(
        &mut app,
        json!({
            "dir": dir.path().display().to_string(),
            "range": { "start": { "bar": 2 }, "end": { "bar": 4 } }
        }),
    ));
    match sent_stems(&cmd_rx) {
        AudioCommand::ExportStems { range, .. } => assert_eq!(range, Some((BAR, 3 * BAR))),
        _ => unreachable!(),
    }
}

#[test]
fn stems_validates_dir() {
    let (mut app, _cmd_rx) = stems_app();
    let response = stems(&mut app, json!({ "dir": "relative/stems" }));
    assert_eq!(response.error.expect("relative").kind(), ErrorKind::InvalidParams);

    let file = tempfile::NamedTempFile::new().unwrap();
    let response = stems(&mut app, json!({ "dir": file.path().display().to_string() }));
    assert_eq!(response.error.expect("a file").kind(), ErrorKind::InvalidParams);
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

#[test]
fn a_stem_that_fails_fails_the_job_naming_what_was_written() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (mut app, cmd_rx) = stems_app();
    let job = started_job(stems(&mut app, json!({ "dir": dir.path().display().to_string() })));
    let paths = target_paths(&sent_stems(&cmd_rx));
    app.test_apply_engine_event(AudioEvent::StemExportTargetDone {
        index: 0,
        path: paths[0].1.clone(),
    });
    app.test_apply_engine_event(AudioEvent::StemExportTargetError {
        index: 1,
        message: "write failed".to_owned(),
    });
    app.test_apply_engine_event(AudioEvent::StemExportComplete {
        files: vec![paths[0].1.clone()],
    });
    let status = job_status(&mut app, job);
    assert_eq!(status.state, JobState::Error);
    let message = status.error.unwrap().message;
    assert!(message.contains("write failed"), "{message}");
    assert!(message.contains("01-Lead Vox.wav"), "{message}");
}

#[test]
fn a_stem_export_that_cannot_start_fails_with_its_kind() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (mut app, _cmd_rx) = stems_app();
    let job = started_job(stems(&mut app, json!({ "dir": dir.path().display().to_string() })));
    app.test_apply_engine_event(AudioEvent::StemExportError(EngineError::busy(
        "Stop transport before exporting stems",
    )));
    let status = job_status(&mut app, job);
    assert_eq!(status.state, JobState::Error);
    assert_eq!(status.error.unwrap().kind, Some(ErrorKind::Busy));
    // The modal never saw it.
    assert!(app.test_export_dialog().is_none());
}

#[test]
fn the_progress_overlay_cancels_a_control_stem_job() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (mut app, cmd_rx) = stems_app();
    let job = started_job(stems(&mut app, json!({ "dir": dir.path().display().to_string() })));
    let _ = sent_stems(&cmd_rx);
    assert_eq!(app.root_overlay(), Some(Overlay::StemsProgress));

    let _ = app.update(Message::Export(ExportMessage::CancelRender));
    assert!(cmd_rx.try_iter().any(|c| matches!(c, AudioCommand::CancelStemExport)));
    app.test_apply_engine_event(AudioEvent::StemExportCancelled { files: Vec::new() });
    let status = job_status(&mut app, job);
    assert_eq!(status.state, JobState::Error);
    assert!(status.error.unwrap().message.contains("cancelled"));
    assert!(app.root_overlay().is_none());
}

#[test]
fn the_export_dialog_and_a_control_stem_job_refuse_each_other() {
    let dir = tempfile::tempdir().expect("temp dir");

    // A control job is rendering: the dialog does not open, and a second
    // render.stems is busy.
    let (mut app, _cmd_rx) = stems_app();
    let _ = started_job(stems(&mut app, json!({ "dir": dir.path().display().to_string() })));
    let _ = app.update(Message::Export(ExportMessage::Open));
    assert!(app.test_export_dialog().is_none(), "the Export dialog is refused");
    let other = dir.path().join("other");
    let response = stems(&mut app, json!({ "dir": other.display().to_string() }));
    assert_eq!(response.error.expect("busy").kind(), ErrorKind::Busy);

    // The dialog is rendering: render.stems answers busy.
    let (mut gui, _gui_rx) = stems_app();
    gui.test_set_project_path(dir.path().to_path_buf());
    let _ = gui.update(Message::Export(ExportMessage::Open));
    let _ = gui.update(Message::Export(ExportMessage::ToggleSource(ExportSource::Track(VOX))));
    let _ = gui.update(Message::Export(ExportMessage::Confirm));
    assert!(gui.test_export_dialog().unwrap().is_rendering());
    let response = stems(&mut gui, json!({ "dir": other.display().to_string() }));
    assert_eq!(response.error.expect("busy").kind(), ErrorKind::Busy);
    // The dialog's events still go to the dialog.
    gui.test_apply_engine_event(AudioEvent::StemExportComplete { files: Vec::new() });
    assert_eq!(
        gui.test_export_dialog().unwrap().phase,
        ExportPhase::Done(Vec::new())
    );
}

#[test]
fn hello_lists_render_stems() {
    let mut app = app();
    let response = roundtrip(
        &mut app,
        request(1, "control.hello", json!({ "protocol_version": resonance_control::PROTOCOL_VERSION })),
    );
    let hello: resonance_control::methods::control::HelloResult = response.result().unwrap();
    assert!(hello.capabilities.iter().any(|m| m == "render.stems"), "{:?}", hello.capabilities);
}
