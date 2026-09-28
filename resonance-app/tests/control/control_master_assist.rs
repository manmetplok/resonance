//! `master.assist` (warmth-width-depth.md §7.4): the mastering assistant
//! over an offline render of the master, answered WITHOUT applying
//! anything.
//!
//! Capture-mode app: the engine commands the handler sends are asserted,
//! and the engine's answers are injected as `MixMeasured` events built
//! from a known assistant spectrum. The payload must be exactly what the
//! assistant's decision engine (`resonance-mastering-assist`, which the
//! plugin's panel runs too) makes of that analysis — the same stages, and
//! param writes that replay to exactly the writes the panel's Apply
//! makes.

use resonance_app::state::{PluginSlotState, PoolAsset};
use resonance_app::Resonance;
use resonance_audio::types::{
    AudioCommand, AudioEvent, AudioMeasureSource, DetailSet, MeasureSource, MeasurementDetail,
    MixMeasurement, StemSource, TrackType,
};
use resonance_common::AudioFormat;
use resonance_control::job::{JobStarted, JobState, JobStatus};
use resonance_control::methods::master::{AssistMode, AssistResult};
use resonance_control::{ErrorKind, Request, Response};
use resonance_mastering_assist::analyze::AnalysisResult;
use resonance_mastering_assist::decide::{build, ParamSink, Target, HIGH_BAND_HZ};
use resonance_mastering_assist::targets::{target_band, target_curve, Genre};
use resonance_mastering_assist::ReferenceTrack;
use resonance_metering::offline::BandShares;
use serde_json::json;

use crate::common::roundtrip;

const SR: u32 = 48_000;
const ASSET: u64 = 9;

fn capture_app() -> (Resonance, crossbeam_channel::Receiver<AudioCommand>) {
    let (mut app, _task, cmd_rx) = Resonance::new_for_test_with_capture();
    app.test_set_sample_rate(SR);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-master-assist"));
    app.test_add_track(1, TrackType::Audio);
    app.test_add_pool_asset(PoolAsset {
        id: ASSET,
        project_relative_path: "audio/asset_9.wav".to_owned(),
        original_path: "/refs/Commercial Master.flac".to_owned(),
        format: AudioFormat::Flac,
        channels: 2,
        source_sample_rate: 44_100,
        duration_frames: 30 * SR as u64,
        thumbnail_peaks: Vec::new(),
        missing: false,
    });
    while cmd_rx.try_recv().is_ok() {}
    (app, cmd_rx)
}

fn request(params: serde_json::Value) -> Request {
    Request::new(1, "master.assist", &params).expect("params serialize")
}

fn started_job(response: Response) -> u64 {
    let started: JobStarted = response.result().expect("job started");
    u64::from(started.job_id)
}

fn status(app: &mut Resonance, job: u64) -> JobStatus {
    roundtrip(app, Request::new(9, "job.status", &json!({ "job_id": job })).unwrap())
        .result()
        .expect("job.status succeeds")
}

fn done(app: &mut Resonance, job: u64) -> AssistResult {
    let status = status(app, job);
    assert_eq!(status.state, JobState::Done, "job errored: {:?}", status.error);
    serde_json::from_value(status.result.expect("done carries a result")).expect("AssistResult")
}

/// A master whose low band sits 3 dB over the rock band and whose top
/// sits 2 dB under it, with loose dynamics and a narrow image.
fn assistant_spectrum() -> Vec<f32> {
    let (lo, hi) = target_band(Genre::Rock);
    let mut s = target_curve(Genre::Rock).to_vec();
    for (i, v) in s.iter_mut().enumerate() {
        let f = resonance_mastering_assist::targets::band_center_hz(i);
        if f <= 100.0 {
            *v = hi[i] + 3.0;
        } else if f >= HIGH_BAND_HZ.0 {
            *v = lo[i] - 2.0;
        }
        // Absolute level must not matter.
        *v -= 30.0;
    }
    s
}

fn measurement(source: MeasureSource, lufs: f32, ltas: Vec<f32>) -> MixMeasurement {
    let frames = u64::from(SR) * 30;
    MixMeasurement {
        target: StemSource::Master,
        source,
        range_start: 0,
        range_end: frames,
        frames,
        lufs_integrated: lufs,
        lufs_short_term_max: lufs + 2.0,
        lufs_momentary_max: lufs + 4.0,
        lra_lu: 6.0,
        true_peak_dbtp: -3.0,
        sample_peak_db: -3.2,
        crest_db: 16.5,
        clipped_samples: 0,
        correlation: 0.95,
        mono_penalty_db: -0.2,
        bands: BandShares {
            low: 0.4,
            mid: 0.35,
            high: 0.2,
            air: 0.05,
        },
        detail: MeasurementDetail {
            assist_ltas: Some(ltas),
            ..MeasurementDetail::default()
        },
    }
}

/// What the plugin's assistant makes of a measurement.
fn analysis(m: &MixMeasurement) -> AnalysisResult {
    AnalysisResult {
        sample_rate: SR as f32,
        duration_s: m.frames as f32 / SR as f32,
        integrated_lufs: m.lufs_integrated,
        short_term_lufs: m.lufs_short_term_max,
        true_peak_dbtp: m.true_peak_dbtp,
        crest_db: m.crest_db,
        correlation: m.correlation,
        spectrum_db: m.detail.assist_ltas.clone().unwrap(),
    }
}

fn sent(cmd_rx: &crossbeam_channel::Receiver<AudioCommand>) -> Vec<AudioCommand> {
    cmd_rx
        .try_iter()
        .filter(|c| {
            matches!(
                c,
                AudioCommand::MeasureMix { .. } | AudioCommand::MeasureAudio { .. }
            )
        })
        .collect()
}

/// Param writes by key, as the panel's Apply makes them (the mastering
/// plugin applies through the same `ParamSink`; its own tests pin that
/// every key resolves and that Apply writes exactly these).
#[derive(Default)]
struct Writes(std::cell::RefCell<std::collections::BTreeMap<String, f64>>);

impl ParamSink for Writes {
    fn set_param(&self, key: &str, value: f32) {
        self.0.borrow_mut().insert(key.to_owned(), f64::from(value));
    }
}

/// The wire's param writes, replayed in order, give exactly the writes
/// the panel's Apply makes.
fn assert_replays_like_apply(result: &AssistResult, target: &Target, analysis: &AnalysisResult) {
    let mut wire = std::collections::BTreeMap::new();
    for suggestion in &result.suggestions {
        for write in &suggestion.params {
            wire.insert(write.key.clone(), write.value);
        }
    }
    let panel = Writes::default();
    build(analysis, target).apply_to(&panel);
    let panel = panel.0.into_inner();
    assert_eq!(
        wire.keys().collect::<Vec<_>>(),
        panel.keys().collect::<Vec<_>>(),
        "the wire and Apply write different params"
    );
    for (key, a) in &wire {
        let b = panel[key];
        assert!((a - b).abs() < 2e-3, "{key}: wire replay {a} vs Apply {b}");
    }
}

#[test]
fn genre_mode_renders_the_master_with_the_assistant_ltas_and_applies_nothing() {
    let (mut app, cmd_rx) = capture_app();
    app.test_push_master_plugin(PluginSlotState::new(
        40,
        "Resonance Mastering".into(),
        "com.resonance.mastering".into(),
        "/plugins/mastering.clap".into(),
        Vec::new(),
        false,
    ));
    let revision = app.revision();
    let job = started_job(roundtrip(
        &mut app,
        request(json!({ "mode": "genre", "genre": "rock" })),
    ));
    let commands = sent(&cmd_rx);
    assert_eq!(commands.len(), 1, "genre mode renders the master only");
    let AudioCommand::MeasureMix {
        measure_id,
        targets,
        source,
        detail,
        ..
    } = &commands[0]
    else {
        panic!("expected MeasureMix, got {:?}", commands[0]);
    };
    assert_eq!(*measure_id, job);
    assert_eq!(targets, &vec![StemSource::Master]);
    assert_eq!(*source, MeasureSource::Render);
    assert_eq!(
        *detail,
        DetailSet {
            assist: true,
            ..DetailSet::default()
        }
    );

    let mix = measurement(MeasureSource::Render, -21.0, assistant_spectrum());
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![mix.clone()],
    });
    let result = done(&mut app, job);
    assert_eq!(app.revision(), revision, "master.assist changes nothing");

    assert_eq!(result.target.mode, AssistMode::Genre);
    assert_eq!(result.target.label, "Rock");
    assert_eq!(result.target.target_lufs, -11.0);
    assert_eq!(result.plugin_id, "com.resonance.mastering");
    assert_eq!(result.master_slot, Some(0));
    assert_eq!(result.measured.lufs_integrated, Some(-21.0));
    assert_eq!(result.measured.measured_seconds, 30.0);
    assert_eq!(result.deviations.len(), 31);

    let stages: Vec<&str> = result.suggestions.iter().map(|s| s.stage.as_str()).collect();
    assert_eq!(
        stages,
        vec![
            "input_trim",
            "tonal_low_shelf",
            "tonal_high_shelf",
            "glue",
            "imager",
            "limiter",
            "target_lufs",
            "diagnostic"
        ]
    );
    let low = &result.suggestions[1];
    let gain = low.params.iter().find(|p| p.key == "tone_b0_gain").unwrap();
    assert!((gain.value - -3.0).abs() < 0.05, "cut the 3 dB over the band: {gain:?}");
    let high = &result.suggestions[2];
    let gain = high.params.iter().find(|p| p.key == "tone_b3_gain").unwrap();
    assert!((gain.value - 2.0).abs() < 0.05, "lift the 2 dB under the band: {gain:?}");
    // A shelf is a stereo move: it puts its band's M/S selector back on
    // Stereo (index 0), whatever an earlier edit left there.
    for (stage, key) in [(low, "tone_b0_ms"), (high, "tone_b3_ms")] {
        let ms = stage.params.iter().find(|p| p.key == key);
        assert_eq!(ms.map(|p| p.value), Some(0.0), "{key} in {:?}", stage.params);
    }
    let sub = result.deviations.iter().find(|d| (d.hz - 50.0).abs() < 2.0).unwrap();
    assert!((sub.deviation_db - 3.0).abs() < 0.15, "{sub:?}");
    let mid = result.deviations.iter().find(|d| (d.hz - 1_000.0).abs() < 1.0).unwrap();
    assert_eq!(mid.deviation_db, 0.0);

    assert_replays_like_apply(&result, &Target::Genre(Genre::Rock), &analysis(&mix));
}

#[test]
fn reference_mode_measures_the_pooled_file_and_targets_it() {
    let (mut app, cmd_rx) = capture_app();
    let job = started_job(roundtrip(
        &mut app,
        request(json!({ "mode": "reference", "pool_asset_id": ASSET })),
    ));
    let commands = sent(&cmd_rx);
    assert_eq!(commands.len(), 1, "the reference is decoded first: {commands:?}");
    let AudioCommand::MeasureAudio {
        measure_id,
        source: AudioMeasureSource::File(path),
        detail,
    } = &commands[0]
    else {
        panic!("expected MeasureAudio of a file, got {:?}", commands[0]);
    };
    assert_eq!(*measure_id, job);
    assert!(detail.assist);
    assert_eq!(
        path,
        &std::path::PathBuf::from("/tmp/control-master-assist/audio/asset_9.wav"),
        "the pooled engine-format file a clip plays, not the original"
    );

    // The reference: flat on the rock midline, at -9 LUFS.
    let reference_ltas = target_curve(Genre::Rock).to_vec();
    let reference = measurement(MeasureSource::Decoded, -9.0, reference_ltas);
    let mix = measurement(MeasureSource::Render, -21.0, assistant_spectrum());
    // The reference arrives first; only then is the master rendered.
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![reference.clone()],
    });
    assert_ne!(status(&mut app, job).state, JobState::Done, "still waiting on the mix");
    let commands = sent(&cmd_rx);
    assert_eq!(commands.len(), 1, "{commands:?}");
    let AudioCommand::MeasureMix {
        measure_id,
        targets,
        source,
        detail,
        ..
    } = &commands[0]
    else {
        panic!("expected the master render, got {:?}", commands[0]);
    };
    assert_eq!(*measure_id, job);
    assert_eq!(targets, &vec![StemSource::Master]);
    assert_eq!(*source, MeasureSource::Render);
    assert!(detail.assist);
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![mix.clone()],
    });
    let result = done(&mut app, job);
    assert_eq!(result.target.mode, AssistMode::Reference);
    assert_eq!(result.target.label, "Commercial Master");
    assert_eq!(result.target.target_lufs, -9.0);
    assert_eq!(result.master_slot, None, "no mastering plugin on the master yet");

    let track = ReferenceTrack {
        display_name: "Commercial Master".into(),
        sample_rate: SR as f32,
        analysis: analysis(&reference),
    };
    assert_replays_like_apply(&result, &Target::Reference(track), &analysis(&mix));
}

#[test]
fn a_silent_master_fails_the_job_instead_of_suggesting() {
    let (mut app, _cmd_rx) = capture_app();
    let job = started_job(roundtrip(
        &mut app,
        request(json!({ "mode": "genre", "genre": "pop" })),
    ));
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![measurement(
            MeasureSource::Render,
            f32::NEG_INFINITY,
            vec![-120.0; 60],
        )],
    });
    let status = status(&mut app, job);
    assert_eq!(status.state, JobState::Error);
    assert!(status.error.unwrap().message.contains("silent"));
}

#[test]
fn an_engine_failure_fails_the_job() {
    let (mut app, _cmd_rx) = capture_app();
    let job = started_job(roundtrip(
        &mut app,
        request(json!({ "mode": "reference", "pool_asset_id": ASSET })),
    ));
    app.test_apply_engine_event(AudioEvent::MixMeasureError {
        measure_id: job,
        message: "cannot read the file".into(),
    });
    let status = status(&mut app, job);
    assert_eq!(status.state, JobState::Error);
    assert!(status.error.unwrap().message.contains("cannot read"));
}

/// A failed reference decode ends the job, and with it the offline-render
/// guard the job holds — so no master render may still be running under
/// it, or a bounce could start on top of that render. The master render
/// is only ever sent after the reference succeeded, and while the
/// reference decodes the guard is held.
#[test]
fn a_failed_reference_leaves_no_render_running_behind_the_released_guard() {
    let (mut app, cmd_rx) = capture_app();
    let job = started_job(roundtrip(
        &mut app,
        request(json!({ "mode": "reference", "pool_asset_id": ASSET })),
    ));
    let (kind, _) = refused(&mut app, json!({ "mode": "genre", "genre": "rock" }));
    assert_eq!(kind, ErrorKind::Busy, "the guard is held while the reference decodes");
    app.test_apply_engine_event(AudioEvent::MixMeasureError {
        measure_id: job,
        message: "cannot read the file".into(),
    });
    assert_eq!(status(&mut app, job).state, JobState::Error);
    let renders: Vec<_> = sent(&cmd_rx)
        .into_iter()
        .filter(|c| matches!(c, AudioCommand::MeasureMix { .. }))
        .collect();
    assert!(
        renders.is_empty(),
        "the guard is released, so no master render may be in flight: {renders:?}"
    );
}

fn refused(app: &mut Resonance, params: serde_json::Value) -> (ErrorKind, String) {
    let error = roundtrip(app, request(params)).error.expect("refused");
    (error.kind(), error.message)
}

#[test]
fn each_mode_takes_exactly_its_own_field() {
    let (mut app, cmd_rx) = capture_app();
    let (kind, message) = refused(&mut app, json!({ "mode": "genre" }));
    assert_eq!(kind, ErrorKind::InvalidParams, "{message}");
    let (kind, _) = refused(
        &mut app,
        json!({ "mode": "genre", "genre": "rock", "pool_asset_id": ASSET }),
    );
    assert_eq!(kind, ErrorKind::InvalidParams);
    let (kind, _) = refused(&mut app, json!({ "mode": "reference" }));
    assert_eq!(kind, ErrorKind::InvalidParams);
    let (kind, _) = refused(
        &mut app,
        json!({ "mode": "reference", "pool_asset_id": ASSET, "genre": "jazz" }),
    );
    assert_eq!(kind, ErrorKind::InvalidParams);
    let (kind, message) = refused(&mut app, json!({ "mode": "reference", "pool_asset_id": 404 }));
    assert_eq!(kind, ErrorKind::NotFound, "{message}");
    let (kind, _) = refused(&mut app, json!({ "mode": "genre", "genre": "polka" }));
    assert_eq!(kind, ErrorKind::InvalidParams);
    assert!(sent(&cmd_rx).is_empty(), "a refused call starts nothing");
}

#[test]
fn a_rolling_transport_is_busy() {
    let (mut app, _cmd_rx) = capture_app();
    app.test_set_transport_playing(true);
    let (kind, _) = refused(&mut app, json!({ "mode": "genre", "genre": "rock" }));
    assert_eq!(kind, ErrorKind::Busy);
}

/// The panel and the wire analyse identically: the engine's measurement
/// of a rendered buffer, read the way `master.assist` reads it, is the
/// plugin's own `analyze::run` of the same buffer — same LTAS bit for bit,
/// same crest, correlation and true peak, and the same gated loudness.
#[test]
fn the_engine_measurement_is_the_panels_analysis() {
    use resonance_audio::test_support::measure_rendered_buffer_detailed;
    let frames = 6 * SR as usize;
    let mut state = 0x1234_5678_u32;
    let mut noise = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        (state as f32 / u32::MAX as f32) * 2.0 - 1.0
    };
    let (mut left, mut right) = (Vec::with_capacity(frames), Vec::with_capacity(frames));
    let mut interleaved = Vec::with_capacity(frames * 2);
    for n in 0..frames {
        let t = n as f32 / SR as f32;
        let tone = 0.3 * (std::f32::consts::TAU * 110.0 * t).sin();
        let l = tone + 0.1 * noise();
        let r = 0.8 * tone + 0.1 * noise();
        left.push(l);
        right.push(r);
        interleaved.extend_from_slice(&[l, r]);
    }
    let detail = DetailSet {
        assist: true,
        ..DetailSet::default()
    };
    let m = measure_rendered_buffer_detailed(
        StemSource::Master,
        0,
        frames as u64,
        &interleaved,
        SR,
        detail,
    );
    let wire = analysis(&m);
    let panel = resonance_mastering_assist::analyze::run(SR as f32, &left, &right);
    assert_eq!(wire.spectrum_db, panel.spectrum_db);
    assert_eq!(wire.crest_db, panel.crest_db);
    assert_eq!(wire.correlation, panel.correlation);
    assert_eq!(wire.true_peak_dbtp, panel.true_peak_dbtp);
    assert!(
        (wire.integrated_lufs - panel.integrated_lufs).abs() < 0.01,
        "{} vs {}",
        wire.integrated_lufs,
        panel.integrated_lufs
    );
    let a = build(&wire, &Target::Genre(Genre::Indie));
    let b = build(&panel, &Target::Genre(Genre::Indie));
    assert_eq!(a.stages().len(), b.stages().len());
    assert_eq!(a.tonal_low_shelf_gain_db, b.tonal_low_shelf_gain_db);
    assert_eq!(a.tonal_high_shelf_gain_db, b.tonal_high_shelf_gain_db);
}
