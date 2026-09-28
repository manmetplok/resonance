//! Reference tracks over the control API (warmth-width-depth.md §7.5):
//! `reference.load {pool_asset_id}` puts the pooled asset on the A/B
//! reference list, and `meter.measure {target: {reference: N}}` measures
//! it from its decoded audio under its own target.
//!
//! Capture-mode app: the engine commands are asserted and the engine's
//! answers injected. That the numbers match the same file placed as a clip
//! is `resonance-audio/tests/mixer/measure_reference.rs`.

use resonance_app::state::PoolAsset;
use resonance_app::Resonance;
use resonance_audio::types::{
    AudioCommand, AudioEvent, AudioMeasureSource, MeasureSource, MeasurementDetail,
    MixMeasurement, ReferenceId as EngineReferenceId, StemSource, TrackType,
};
use resonance_common::AudioFormat;
use resonance_control::ids::ReferenceId;
use resonance_control::job::{JobStarted, JobState, JobStatus};
use resonance_control::methods::meter::{MeasureResult, MeasureTarget};
use resonance_control::methods::reference::LoadResult;
use resonance_control::{ErrorKind, Request, Response};
use resonance_metering::offline::BandShares;
use serde_json::json;

use crate::common::roundtrip;

const SR: u32 = 48_000;
const ASSET: u64 = 5;
const SOLOED: u64 = 1;

fn capture_app() -> (Resonance, crossbeam_channel::Receiver<AudioCommand>) {
    let (mut app, _task, cmd_rx) = Resonance::new_for_test_with_capture();
    app.test_set_sample_rate(SR);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-reference"));
    app.test_add_track(SOLOED, TrackType::Audio);
    app.test_add_pool_asset(PoolAsset {
        id: ASSET,
        project_relative_path: "audio/asset_5.wav".to_owned(),
        original_path: "/refs/Their Single.mp3".to_owned(),
        format: AudioFormat::Mp3,
        channels: 2,
        source_sample_rate: 44_100,
        duration_frames: 20 * SR as u64,
        thumbnail_peaks: Vec::new(),
        missing: false,
    });
    while cmd_rx.try_recv().is_ok() {}
    (app, cmd_rx)
}

fn call(app: &mut Resonance, method: &str, params: serde_json::Value) -> Response {
    roundtrip(app, Request::new(1, method, &params).expect("params serialize"))
}

fn job_of(response: Response) -> u64 {
    let started: JobStarted = response.result().expect("job started");
    u64::from(started.job_id)
}

fn status(app: &mut Resonance, job: u64) -> JobStatus {
    call(app, "job.status", json!({ "job_id": job }))
        .result()
        .expect("job.status succeeds")
}

fn decoded(frames: u64) -> MixMeasurement {
    MixMeasurement {
        target: StemSource::Master,
        source: MeasureSource::Decoded,
        range_start: 0,
        range_end: frames,
        frames,
        lufs_integrated: -8.7,
        lufs_short_term_max: -7.9,
        lufs_momentary_max: -6.8,
        lra_lu: 4.1,
        true_peak_dbtp: -0.6,
        sample_peak_db: -0.8,
        crest_db: 9.4,
        clipped_samples: 0,
        correlation: 0.71,
        mono_penalty_db: -0.9,
        bands: BandShares {
            low: 0.45,
            mid: 0.33,
            high: 0.18,
            air: 0.04,
        },
        detail: MeasurementDetail::default(),
    }
}

/// Load the fixture asset and let the engine finish decoding it.
fn loaded_reference(
    app: &mut Resonance,
    cmd_rx: &crossbeam_channel::Receiver<AudioCommand>,
) -> ReferenceId {
    let loaded: LoadResult = call(app, "reference.load", json!({ "pool_asset_id": ASSET }))
        .result()
        .expect("reference.load succeeds");
    let sent: Vec<_> = cmd_rx
        .try_iter()
        .filter_map(|c| match c {
            AudioCommand::LoadReferenceTrack { id, path } => Some((id, path)),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 1);
    let (engine_id, path) = sent[0].clone();
    assert_eq!(u64::from(engine_id.0), loaded.reference_id.0);
    assert_eq!(
        path,
        std::path::PathBuf::from("/tmp/control-reference/audio/asset_5.wav"),
        "the pooled engine-format file a clip plays"
    );
    app.test_apply_engine_event(AudioEvent::ReferenceLoaded {
        id: engine_id,
        name: "asset_5".into(),
        path: path.to_string_lossy().into_owned(),
        integrated_lufs: -8.7,
        waveform_peaks: Vec::new(),
        length_samples: 20 * u64::from(SR),
    });
    loaded.reference_id
}

#[test]
fn reference_load_lists_the_asset_under_its_name_and_is_undoable() {
    let (mut app, cmd_rx) = capture_app();
    let before = app.revision();
    let id = loaded_reference(&mut app, &cmd_rx);
    assert!(app.revision() > before, "a load is an edit");
    let entry = app
        .test_reference()
        .entries
        .iter()
        .find(|e| u64::from(e.id.0) == id.0)
        .expect("listed");
    assert_eq!(entry.name, "Their Single", "the asset's name survives the engine echo");

    let undo = call(&mut app, "edit.undo", json!({}));
    assert!(undo.error.is_none(), "{:?}", undo.error);
    assert!(
        app.test_reference().entries.iter().all(|e| u64::from(e.id.0) != id.0),
        "undo drops the loaded reference"
    );
}

#[test]
fn a_reference_measures_from_its_decoded_audio_under_its_own_target() {
    let (mut app, cmd_rx) = capture_app();
    let solo = call(&mut app, "mixer.set_solo", json!({ "track_id": SOLOED, "soloed": true }));
    assert!(solo.error.is_none(), "{:?}", solo.error);
    let id = loaded_reference(&mut app, &cmd_rx);
    let job = job_of(call(
        &mut app,
        "meter.measure",
        json!({ "target": { "reference": id.0 }, "detail": ["stereo", "dynamics"] }),
    ));
    let sent: Vec<_> = cmd_rx
        .try_iter()
        .filter_map(|c| match c {
            AudioCommand::MeasureAudio {
                measure_id,
                source,
                detail,
            } => Some((measure_id, source, detail)),
            AudioCommand::MeasureMix { .. } => panic!("a reference renders nothing"),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 1);
    let (measure_id, source, detail) = sent[0].clone();
    assert_eq!(measure_id, job);
    assert_eq!(
        source,
        AudioMeasureSource::Reference(EngineReferenceId(id.0 as u32))
    );
    assert!(detail.stereo && detail.dynamics && !detail.spectrum);

    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![decoded(20 * u64::from(SR))],
    });
    let status = status(&mut app, job);
    assert_eq!(status.state, JobState::Done, "{:?}", status.error);
    let wire = status.result.unwrap();
    assert_eq!(wire["target"], json!({ "reference": id.0 }));
    assert!(
        wire.get("soloed_track_ids").is_none(),
        "solo has nothing to do with a reference: {wire}"
    );
    let result: MeasureResult = serde_json::from_value(wire).unwrap();
    assert_eq!(result.target, MeasureTarget::Reference(id));
    assert_eq!(result.lufs_integrated, Some(-8.7));
    assert_eq!(result.measured_seconds, Some(20.0));
}

#[test]
fn a_reference_still_decoding_is_busy() {
    let (mut app, cmd_rx) = capture_app();
    let loaded: LoadResult = call(&mut app, "reference.load", json!({ "pool_asset_id": ASSET }))
        .result()
        .unwrap();
    let error = call(
        &mut app,
        "meter.measure",
        json!({ "target": { "reference": loaded.reference_id.0 } }),
    )
    .error
    .expect("refused");
    assert_eq!(error.kind(), ErrorKind::Busy, "{}", error.message);
    assert!(!cmd_rx
        .try_iter()
        .any(|c| matches!(c, AudioCommand::MeasureAudio { .. })));
}

#[test]
fn reference_targets_are_refused_where_they_do_not_apply() {
    let (mut app, cmd_rx) = capture_app();
    let id = loaded_reference(&mut app, &cmd_rx);
    let target = json!({ "reference": id.0 });
    let kind = |app: &mut Resonance, method: &str, params: serde_json::Value| {
        call(app, method, params).error.expect("refused").kind()
    };
    assert_eq!(
        kind(&mut app, "meter.measure", json!({ "target": target, "source": "live" })),
        ErrorKind::InvalidParams
    );
    assert_eq!(
        kind(
            &mut app,
            "meter.measure",
            json!({ "target": target, "range": { "start": { "bar": 2 } } })
        ),
        ErrorKind::InvalidParams
    );
    assert_eq!(
        kind(&mut app, "meter.measure", json!({ "target": { "reference": 999 } })),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(&mut app, "meter.snapshot", json!({ "target": target })),
        ErrorKind::InvalidParams
    );
    assert_eq!(
        kind(&mut app, "meter.probe", json!({ "target": target })),
        ErrorKind::InvalidParams
    );
    assert_eq!(
        kind(&mut app, "reference.load", json!({ "pool_asset_id": 404 })),
        ErrorKind::NotFound
    );
}

#[test]
fn a_failed_reference_measurement_fails_its_job() {
    let (mut app, cmd_rx) = capture_app();
    let id = loaded_reference(&mut app, &cmd_rx);
    let job = job_of(call(
        &mut app,
        "meter.measure",
        json!({ "target": { "reference": id.0 } }),
    ));
    app.test_apply_engine_event(AudioEvent::MixMeasureError {
        measure_id: job,
        message: "reference 1 is still decoding".into(),
    });
    assert_eq!(status(&mut app, job).state, JobState::Error);
}
