//! `render.mixdown` delivery normalization (warmth-width-depth.md §7.7,
//! W11): `normalize` and `platform` map to the engine's normalize stage,
//! the job resolves off `ExportComplete` with what the file achieved,
//! and a plain mixdown still takes the unchanged bounce path. The
//! loudness the stage actually reaches is pinned in
//! `resonance-audio/tests/bounce/delivery_normalize.rs`.

use resonance_app::Resonance;
use resonance_audio::types::{
    AudioCommand, AudioEvent, ExportErrorKind, NormalizeMode, NormalizeSpec,
};
use resonance_control::job::{JobStarted, JobState, JobStatus};
use resonance_control::methods::render::MixdownResult;
use resonance_control::{ErrorKind, Request, Response};
use serde_json::{json, Value};

use crate::common::roundtrip;

fn capture_app() -> (Resonance, crossbeam_channel::Receiver<AudioCommand>) {
    let (mut app, _task, cmd_rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    while cmd_rx.try_recv().is_ok() {}
    (app, cmd_rx)
}

fn out_path(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "resonance-render-normalize-{tag}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("master.wav").to_string_lossy().into_owned()
}

fn mixdown(app: &mut Resonance, params: Value) -> Response {
    roundtrip(app, Request::new(1, "render.mixdown", &params).unwrap())
}

fn started_job(response: Response) -> u64 {
    let started: JobStarted = response.result().expect("job started");
    u64::from(started.job_id)
}

fn status(app: &mut Resonance, job: u64) -> JobStatus {
    let request = Request::new(99, "job.status", &json!({ "job_id": job })).unwrap();
    roundtrip(app, request).result().expect("job.status succeeds")
}

/// The one export-ish command sent since the last drain.
fn sent(cmd_rx: &crossbeam_channel::Receiver<AudioCommand>) -> AudioCommand {
    let commands: Vec<AudioCommand> = cmd_rx
        .try_iter()
        .filter(|c| {
            matches!(c, AudioCommand::ExportAudio { .. } | AudioCommand::BounceToWav { .. })
        })
        .collect();
    assert_eq!(commands.len(), 1, "{commands:?}");
    commands.into_iter().next().unwrap()
}

fn normalize_of(command: AudioCommand) -> NormalizeSpec {
    match command {
        AudioCommand::ExportAudio { settings, .. } => settings.normalize,
        other => panic!("a normalized mixdown exports, got {other:?}"),
    }
}

#[test]
fn a_platform_maps_to_its_published_target_and_reports_what_the_file_achieved() {
    let (mut app, cmd_rx) = capture_app();
    let path = out_path("spotify");
    let job = started_job(mixdown(&mut app, json!({ "path": path, "platform": "spotify" })));
    assert_eq!(
        normalize_of(sent(&cmd_rx)),
        NormalizeSpec {
            enabled: true,
            mode: NormalizeMode::IntegratedLufs,
            target_db: -14.0,
            ceiling_dbtp: -1.0,
        }
    );
    assert_eq!(status(&mut app, job).state, JobState::Pending);

    // A second render is refused while this one runs.
    let busy = mixdown(&mut app, json!({ "path": out_path("second"), "overwrite": true }));
    assert_eq!(busy.error.expect("busy").kind(), ErrorKind::Busy);

    app.test_apply_engine_event(AudioEvent::ExportComplete {
        path: path.clone(),
        achieved_lufs: Some(-14.034),
        achieved_dbtp: -1.012,
        bytes: 1_234,
    });
    let status = status(&mut app, job);
    assert_eq!(status.state, JobState::Done, "{:?}", status.error);
    let wire = status.result.unwrap();
    assert_eq!(
        wire["normalize"],
        json!({
            "platform": "spotify",
            "target_lufs": -14.0,
            "ceiling_dbtp": -1.0,
            "achieved_lufs": -14.03,
            "achieved_dbtp": -1.01
        })
    );
    let result: MixdownResult = serde_json::from_value(wire).unwrap();
    assert_eq!(result.path, path);
}

#[test]
fn every_platform_has_its_documented_target() {
    for (platform, target, ceiling) in [
        ("spotify", -14.0, -1.0),
        ("apple", -16.0, -1.0),
        ("youtube", -14.0, -1.0),
        ("tidal", -14.0, -1.0),
        ("amazon", -14.0, -2.0),
        ("deezer", -15.0, -1.0),
        ("club", -8.0, -0.3),
    ] {
        let (mut app, cmd_rx) = capture_app();
        let path = out_path(platform);
        let _ = started_job(mixdown(
            &mut app,
            json!({ "path": path, "platform": platform, "overwrite": true }),
        ));
        let spec = normalize_of(sent(&cmd_rx));
        assert_eq!((spec.target_db, spec.ceiling_dbtp), (target, ceiling), "{platform}");
    }
}

#[test]
fn an_explicit_normalize_is_passed_through() {
    let (mut app, cmd_rx) = capture_app();
    let _ = started_job(mixdown(
        &mut app,
        json!({
            "path": out_path("explicit"),
            "normalize": { "target_lufs": -11.5, "ceiling_dbtp": -0.8 }
        }),
    ));
    let spec = normalize_of(sent(&cmd_rx));
    assert_eq!((spec.target_db, spec.ceiling_dbtp), (-11.5, -0.8));
}

#[test]
fn a_plain_mixdown_still_bounces_without_normalizing() {
    let (mut app, cmd_rx) = capture_app();
    let path = out_path("plain");
    let _ = started_job(mixdown(&mut app, json!({ "path": path })));
    assert!(matches!(sent(&cmd_rx), AudioCommand::BounceToWav { .. }));
}

#[test]
fn both_or_out_of_range_targets_are_invalid_and_start_nothing() {
    let (mut app, cmd_rx) = capture_app();
    let path = out_path("invalid");
    for params in [
        json!({
            "path": path,
            "platform": "apple",
            "normalize": { "target_lufs": -14, "ceiling_dbtp": -1 }
        }),
        json!({ "path": path, "normalize": { "target_lufs": -2, "ceiling_dbtp": -1 } }),
        json!({ "path": path, "normalize": { "target_lufs": -14, "ceiling_dbtp": 1 } }),
        json!({ "path": path, "platform": "radio" }),
    ] {
        let error = mixdown(&mut app, params.clone()).error.expect("refused");
        assert_eq!(error.kind(), ErrorKind::InvalidParams, "{params}: {error:?}");
    }
    assert!(!cmd_rx.try_iter().any(|c| matches!(c, AudioCommand::ExportAudio { .. })));
}

#[test]
fn an_export_error_fails_the_normalized_job() {
    let (mut app, _cmd_rx) = capture_app();
    let params = json!({ "path": out_path("err"), "platform": "tidal" });
    let job = started_job(mixdown(&mut app, params));
    app.test_apply_engine_event(AudioEvent::ExportError {
        kind: ExportErrorKind::Io,
        message: "disk full".into(),
    });
    let status = status(&mut app, job);
    assert_eq!(status.state, JobState::Error);
    assert_eq!(status.error.unwrap().message, "disk full");
    // And the renderer is free again.
    let _ = started_job(mixdown(&mut app, json!({ "path": out_path("after"), "overwrite": true })));
}
