//! `clip.set_warp` / `clip.set_warp_markers` / `clip.detect_tempo`
//! through the real control dispatch: validation, the mirror the reply
//! reports, one undo entry per call, and the detect-tempo job resolved by
//! the engine's `ClipTempoDetected`.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::{ClipState, ClipWarpState, MidiClipState, ViewMode};
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, FadeCurve, TrackType, WarpAlgorithm};
use resonance_control::job::{JobStarted, JobState, JobStatus};
use resonance_control::methods::clip::{self as proto, DetectTempoResult, WarpResult};
use resonance_control::{ErrorKind, Request, Response};

const SR: u32 = 48_000;
const TRACK: u64 = 1;
const CLIP: u64 = 7;
const MIDI_CLIP: u64 = 8;

fn app() -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    let rx = app.test_capture_engine();
    app.test_set_sample_rate(SR);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-clip-warp.rprj"));
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_push_clip(ClipState {
        id: CLIP,
        track_id: TRACK,
        start_sample: 0,
        duration_samples: 4 * SR as u64,
        name: "loop".into(),
        total_frames: 4 * SR as u64,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        waveform_peaks: Vec::new(),
        vocal_tuning: None,
        asset_ref: None,
        warp: ClipWarpState::default(),
    });
    app.test_push_midi_clip(MidiClipState {
        id: MIDI_CLIP,
        track_id: TRACK,
        start_sample: 0,
        duration_ticks: 960,
        name: "midi".into(),
        notes: Default::default(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    (app, rx)
}

fn call(app: &mut Resonance, method: &str, params: serde_json::Value) -> Response {
    let (reply, rx) = ReplySender::test_pair();
    let request = Request::new(1, method, &params).expect("params serialize");
    let _ = app.update(Message::Control(ControlMessage::Request(ControlRequest {
        conn: 1,
        request,
        reply,
    })));
    rx.try_recv().expect("one reply per request")
}

fn expect_error(response: Response, kind: ErrorKind) -> String {
    let error = response.error.expect("expected an error reply");
    assert_eq!(error.kind(), kind, "unexpected error kind: {}", error.message);
    error.message
}

fn warp(app: &Resonance) -> ClipWarpState {
    app.test_clips()[0].warp.clone()
}

fn entries(app: &Resonance) -> usize {
    app.test_undo_history().test_undo_entries().len()
}

fn job_status(app: &mut Resonance, job_id: u64) -> JobStatus {
    call(app, "job.status", serde_json::json!({ "job_id": job_id }))
        .result()
        .expect("job.status succeeds")
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        if !matches!(cmd, AudioCommand::PersistClipWavs) {
            cmds.push(cmd);
        }
    }
    cmds
}

#[test]
fn set_warp_applies_given_fields_and_keeps_the_rest() {
    let (mut app, rx) = app();
    let _ = drain(&rx);
    let result: WarpResult = call(
        &mut app,
        proto::SET_WARP,
        serde_json::json!({ "clip_id": CLIP, "enabled": true, "original_bpm": 92.0 }),
    )
    .result()
    .expect("set_warp succeeds");
    assert!(result.enabled);
    assert_eq!(result.original_bpm, Some(92.0));
    assert_eq!(result.algorithm, proto::WarpAlgorithm::Transient);
    assert_eq!(entries(&app), 1);
    assert!(matches!(
        drain(&rx).as_slice(),
        [AudioCommand::SetClipWarp { clip_id: CLIP, warp_enabled: true, original_bpm: Some(b), .. }]
            if *b == 92.0
    ));

    let result: WarpResult = call(
        &mut app,
        proto::SET_WARP,
        serde_json::json!({ "clip_id": CLIP, "algorithm": "tonal", "transpose_semitones": 5.0 }),
    )
    .result()
    .expect("set_warp succeeds");
    assert!(result.enabled, "enabled kept");
    assert_eq!(result.original_bpm, Some(92.0), "tempo kept");
    assert_eq!(warp(&app).algorithm, WarpAlgorithm::Tonal);
    assert_eq!(warp(&app).transpose_semitones, 5.0);

    let result: WarpResult = call(
        &mut app,
        proto::SET_WARP,
        serde_json::json!({ "clip_id": CLIP, "clear_original_bpm": true }),
    )
    .result()
    .expect("clearing succeeds");
    assert_eq!(result.original_bpm, None);
    assert_eq!(entries(&app), 3, "one undo entry per call");
}

#[test]
fn set_warp_rejects_bad_input() {
    let (mut app, _rx) = app();
    let bad = [
        serde_json::json!({ "clip_id": CLIP }),
        serde_json::json!({ "clip_id": CLIP, "original_bpm": 5.0 }),
        serde_json::json!({ "clip_id": CLIP, "original_bpm": 100.0, "clear_original_bpm": true }),
        serde_json::json!({ "clip_id": CLIP, "transpose_semitones": 60.0 }),
        serde_json::json!({ "clip_id": CLIP, "algorithm": "granular" }),
    ];
    for params in bad {
        expect_error(call(&mut app, proto::SET_WARP, params.clone()), ErrorKind::InvalidParams);
    }
    let msg = expect_error(
        call(&mut app, proto::SET_WARP, serde_json::json!({ "clip_id": MIDI_CLIP, "enabled": true })),
        ErrorKind::NotFound,
    );
    assert!(msg.contains("MIDI"), "{msg}");
    assert_eq!(warp(&app), ClipWarpState::default(), "nothing landed");
    assert_eq!(entries(&app), 0);
}

#[test]
fn set_warp_markers_sorts_and_validates() {
    let (mut app, _rx) = app();
    let result: WarpResult = call(
        &mut app,
        proto::SET_WARP_MARKERS,
        serde_json::json!({ "clip_id": CLIP, "markers": [
            { "source_frame": 96000, "beat": 4.0 },
            { "source_frame": 0, "beat": 0.0 },
        ]}),
    )
    .result()
    .expect("set_warp_markers succeeds");
    assert_eq!(result.markers.len(), 2);
    assert_eq!(result.markers[0].beat, 0.0, "sorted by beat");
    assert_eq!(warp(&app).markers[1].source_frame, 96_000);

    for markers in [
        // Runs the source backwards.
        serde_json::json!([{ "source_frame": 96000, "beat": 0.0 }, { "source_frame": 0, "beat": 2.0 }]),
        // Too close together.
        serde_json::json!([{ "source_frame": 0, "beat": 1.0 }, { "source_frame": 10, "beat": 1.01 }]),
        // Negative beat.
        serde_json::json!([{ "source_frame": 0, "beat": -1.0 }]),
        // Past the source.
        serde_json::json!([{ "source_frame": 10000000, "beat": 1.0 }]),
    ] {
        expect_error(
            call(&mut app, proto::SET_WARP_MARKERS, serde_json::json!({ "clip_id": CLIP, "markers": markers })),
            ErrorKind::InvalidParams,
        );
    }
    assert_eq!(warp(&app).markers.len(), 2, "a refused call changes nothing");

    let result: WarpResult = call(
        &mut app,
        proto::SET_WARP_MARKERS,
        serde_json::json!({ "clip_id": CLIP, "markers": [] }),
    )
    .result()
    .expect("clearing succeeds");
    assert!(result.markers.is_empty());
    assert_eq!(entries(&app), 2);
}

#[test]
fn detect_tempo_is_a_job_resolved_by_the_engine() {
    let (mut app, rx) = app();
    let _ = drain(&rx);
    let started: JobStarted = call(&mut app, proto::DETECT_TEMPO, serde_json::json!({ "clip_id": CLIP }))
        .result()
        .expect("detect_tempo starts a job");
    assert!(matches!(
        drain(&rx).as_slice(),
        [AudioCommand::DetectClipTempo { clip_id: CLIP }]
    ));
    let job = u64::from(started.job_id);
    assert!(!job_status(&mut app, job).state.is_terminal());
    assert_eq!(entries(&app), 0, "analysis is not an edit");

    app.test_apply_engine_event(AudioEvent::ClipTempoDetected {
        clip_id: CLIP,
        bpm: 128.0,
        confidence: 0.9,
    });
    let status = job_status(&mut app, job);
    assert_eq!(status.state, JobState::Done, "{:?}", status.error);
    let result: DetectTempoResult =
        serde_json::from_value(status.result.expect("result")).expect("decodes");
    assert_eq!(result.bpm, 128.0);
    assert_eq!(warp(&app).original_bpm, None, "the clip is unchanged");
}

#[test]
fn detect_tempo_fails_when_no_tempo_is_found_or_the_clip_goes() {
    let (mut app, _rx) = app();
    let started: JobStarted = call(&mut app, proto::DETECT_TEMPO, serde_json::json!({ "clip_id": CLIP }))
        .result()
        .expect("starts");
    app.test_apply_engine_event(AudioEvent::ClipTempoDetected {
        clip_id: CLIP,
        bpm: 0.0,
        confidence: 0.0,
    });
    assert_eq!(job_status(&mut app, u64::from(started.job_id)).state, JobState::Error);

    let started: JobStarted = call(&mut app, proto::DETECT_TEMPO, serde_json::json!({ "clip_id": CLIP }))
        .result()
        .expect("starts");
    let _ = call(
        &mut app,
        proto::DELETE,
        serde_json::json!({ "clip_id": CLIP, "confirm": true }),
    );
    assert_eq!(
        job_status(&mut app, u64::from(started.job_id)).state,
        JobState::Error,
        "a deleted clip never replies, so its job must not hang"
    );
}
