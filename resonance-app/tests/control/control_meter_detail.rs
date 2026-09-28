//! `meter.*` opt-in `detail` (warmth-width-depth.md §7.1): the wire
//! option reaches the engine as the right flags, the engine's detail
//! blocks come back as their own objects, and — the compatibility
//! promise — a request without `detail` produces exactly the payload it
//! always did, with no new keys.

use resonance_app::Resonance;
use resonance_audio::types::{
    AudioCommand, AudioEvent, DepthDetail, DepthSend, DetailSet, MeasureSource as EngineSource,
    MeasurementDetail, MixMeasurement, StemSource, TrackType,
};
use resonance_control::job::{JobStarted, JobState};
use resonance_control::methods::meter::{MeasureResult, StemsResult, THIRD_OCTAVE_HZ};
use resonance_control::{ErrorKind, Request, Response};
use resonance_metering::detail::stereo::STEREO_BAND_EDGES_HZ;
use resonance_metering::detail::{
    CorrelationWindows, SpectralPeak, SpectrumDetail, StereoBand, StereoDetail,
};
use resonance_metering::offline::BandShares;
use resonance_metering::RangeDynamics;
use serde_json::json;

use crate::common::roundtrip;

const DRUMS: u64 = 1;
const BASS: u64 = 3;

fn capture_app() -> (Resonance, crossbeam_channel::Receiver<AudioCommand>) {
    let (mut app, _task, cmd_rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    app.test_add_track(DRUMS, TrackType::Instrument);
    app.test_add_track(BASS, TrackType::Instrument);
    while cmd_rx.try_recv().is_ok() {}
    (app, cmd_rx)
}

fn request(id: i64, method: &str, params: serde_json::Value) -> Request {
    Request::new(id, method, &params).expect("params serialize")
}

fn started_job(response: Response) -> u64 {
    let started: JobStarted = response.result().expect("job started");
    u64::from(started.job_id)
}

fn result_json(app: &mut Resonance, job: u64) -> serde_json::Value {
    let status: resonance_control::job::JobStatus =
        roundtrip(app, request(99, "job.status", json!({ "job_id": job })))
            .result()
            .expect("job.status succeeds");
    assert_eq!(status.state, JobState::Done, "job errored: {:?}", status.error);
    status.result.expect("done carries a result")
}

/// The `detail` the one `MeasureMix` sent since the last drain carried.
fn sent_detail(cmd_rx: &crossbeam_channel::Receiver<AudioCommand>) -> DetailSet {
    let sent: Vec<DetailSet> = cmd_rx
        .try_iter()
        .filter_map(|c| match c {
            AudioCommand::MeasureMix { detail, .. } => Some(detail),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 1, "exactly one MeasureMix per request");
    sent[0]
}

/// An engine spectrum block with unrounded values, so the wire's
/// rounding is visible.
fn spectrum() -> SpectrumDetail {
    SpectrumDetail {
        third_octave: (0..31).map(|i| -30.0 - i as f32 * 0.123_456).collect(),
        tilt_db_per_oct: Some(-4.567_89),
        centroid_hz: Some(1_234.567),
        lowmid_presence_db: Some(2.345_67),
        presence_peakiness_db: Some(1.111_11),
        air_ratio_db: Some(-12.345_6),
        peaks: vec![SpectralPeak {
            freq_hz: 3_149.7,
            excess_db: 6.543_21,
        }],
    }
}

fn rendered(target: StemSource, rate: u32, detail: MeasurementDetail) -> MixMeasurement {
    let frames = u64::from(rate) * 10;
    MixMeasurement {
        target,
        source: EngineSource::Render,
        range_start: 0,
        range_end: frames,
        frames,
        lufs_integrated: -20.5,
        lufs_short_term_max: -18.0,
        lufs_momentary_max: -16.5,
        lra_lu: 7.25,
        true_peak_dbtp: -1.5,
        sample_peak_db: -2.0,
        crest_db: 12.5,
        clipped_samples: 0,
        correlation: 0.82,
        mono_penalty_db: -0.4,
        bands: BandShares {
            low: 0.4,
            mid: 0.35,
            high: 0.2,
            air: 0.05,
        },
        detail,
    }
}

fn with_spectrum() -> MeasurementDetail {
    MeasurementDetail {
        spectrum: Some(spectrum()),
        ..MeasurementDetail::default()
    }
}

/// The exact key set a render-path master result had before `detail`
/// existed. A new key on a default request is a wire change.
const DEFAULT_KEYS: &[&str] = &[
    "target",
    "source",
    "lufs_integrated",
    "lufs_short_max",
    "lufs_momentary_max",
    "lra",
    "true_peak_db",
    "sample_peak_db",
    "crest_db",
    "clipped_samples",
    "correlation",
    "mono_penalty_db",
    "bands",
    "measured_seconds",
];

fn keys(value: &serde_json::Value) -> Vec<String> {
    let mut keys: Vec<String> = value
        .as_object()
        .expect("an object")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

fn sorted(list: &[&str]) -> Vec<String> {
    let mut list: Vec<String> = list.iter().map(|s| (*s).to_owned()).collect();
    list.sort();
    list
}

#[test]
fn no_detail_sends_no_flags_and_keeps_the_payload_unchanged() {
    let (mut app, cmd_rx) = capture_app();
    let rate = app.sample_rate;
    let job = started_job(roundtrip(&mut app, request(1, "meter.measure", json!({}))));
    assert_eq!(sent_detail(&cmd_rx), DetailSet::default());

    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![rendered(StemSource::Master, rate, MeasurementDetail::default())],
    });
    let wire = result_json(&mut app, job);
    assert_eq!(keys(&wire), sorted(DEFAULT_KEYS), "{wire}");
}

#[test]
fn an_empty_detail_list_is_the_same_as_none() {
    let (mut app, cmd_rx) = capture_app();
    let _ = started_job(roundtrip(
        &mut app,
        request(1, "meter.measure", json!({ "detail": [] })),
    ));
    assert_eq!(sent_detail(&cmd_rx), DetailSet::default());
}

#[test]
fn spectrum_is_requested_from_the_engine_and_reported_rounded() {
    let (mut app, cmd_rx) = capture_app();
    let rate = app.sample_rate;
    let job = started_job(roundtrip(
        &mut app,
        request(1, "meter.measure", json!({ "detail": ["spectrum"] })),
    ));
    assert!(sent_detail(&cmd_rx).spectrum, "the engine is asked for the spectrum");

    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![rendered(StemSource::Master, rate, with_spectrum())],
    });
    let wire = result_json(&mut app, job);
    let mut expected = sorted(DEFAULT_KEYS);
    expected.push("spectrum".to_owned());
    expected.sort();
    assert_eq!(keys(&wire), expected, "only the asked-for block is added");

    let s = &wire["spectrum"];
    let bands = s["third_octave"].as_array().expect("an array");
    assert_eq!(bands.len(), THIRD_OCTAVE_HZ.len());
    assert_eq!(bands[0], json!(-30.0));
    assert_eq!(bands[1], json!(-30.1));
    assert_eq!(s["tilt_db_per_oct"], json!(-4.57));
    assert_eq!(s["centroid_hz"], json!(1235.0));
    assert_eq!(s["lowmid_presence_db"], json!(2.35));
    assert_eq!(s["presence_peakiness_db"], json!(1.11));
    assert_eq!(s["air_ratio_db"], json!(-12.35));
    assert_eq!(s["peaks"], json!([{ "freq_hz": 3150.0, "excess_db": 6.5 }]));

    // And it decodes as the typed result.
    let typed: MeasureResult = serde_json::from_value(wire).expect("a MeasureResult");
    assert!(typed.spectrum.is_some());
}

#[test]
fn a_silent_spectrum_reports_null_ratios() {
    let (mut app, _cmd_rx) = capture_app();
    let rate = app.sample_rate;
    let job = started_job(roundtrip(
        &mut app,
        request(1, "meter.measure", json!({ "detail": ["spectrum"] })),
    ));
    let silent = SpectrumDetail {
        third_octave: vec![-120.0; 31],
        tilt_db_per_oct: None,
        centroid_hz: None,
        lowmid_presence_db: None,
        presence_peakiness_db: None,
        air_ratio_db: None,
        peaks: Vec::new(),
    };
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![rendered(
            StemSource::Master,
            rate,
            MeasurementDetail {
                spectrum: Some(silent),
                ..MeasurementDetail::default()
            },
        )],
    });
    let s = result_json(&mut app, job)["spectrum"].clone();
    assert_eq!(s["tilt_db_per_oct"], serde_json::Value::Null);
    assert_eq!(s["centroid_hz"], serde_json::Value::Null);
    assert_eq!(s["peaks"], json!([]));
}

#[test]
fn detail_is_refused_on_the_live_path() {
    let (mut app, cmd_rx) = capture_app();
    let response = roundtrip(
        &mut app,
        request(
            1,
            "meter.measure",
            json!({ "source": "live", "detail": ["spectrum"] }),
        ),
    );
    let error = response.error.expect("live + detail is refused");
    assert_eq!(error.kind(), ErrorKind::InvalidParams, "{error:?}");
    assert!(
        !cmd_rx
            .try_iter()
            .any(|c| matches!(c, AudioCommand::MeasureMix { .. })),
        "a refused request starts nothing"
    );
}

#[test]
fn an_unknown_detail_is_invalid_params() {
    let (mut app, _cmd_rx) = capture_app();
    let response = roundtrip(
        &mut app,
        request(1, "meter.measure", json!({ "detail": ["warmth"] })),
    );
    let error = response.error.expect("an unknown detail is refused");
    assert_eq!(error.kind(), ErrorKind::InvalidParams, "{error:?}");
}

#[test]
fn stems_carries_the_detail_on_the_master_and_every_entry() {
    let (mut app, cmd_rx) = capture_app();
    let rate = app.sample_rate;
    let job = started_job(roundtrip(
        &mut app,
        request(1, "meter.stems", json!({ "detail": ["spectrum"] })),
    ));
    assert!(sent_detail(&cmd_rx).spectrum);

    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: [StemSource::Master, StemSource::Track(DRUMS), StemSource::Track(BASS)]
            .into_iter()
            .map(|t| rendered(t, rate, with_spectrum()))
            .collect(),
    });
    let wire = result_json(&mut app, job);
    assert!(wire["master"]["spectrum"].is_object());
    for entry in wire["tracks"].as_array().expect("tracks") {
        assert!(entry["spectrum"].is_object(), "flattened onto the entry: {entry}");
    }
    let typed: StemsResult = serde_json::from_value(wire).expect("a StemsResult");
    assert_eq!(typed.tracks.len(), 2);
}

/// A hard-panned engine stereo block: one-sided, so every correlation is
/// absent rather than 0 or +1.
fn hard_panned_stereo() -> StereoDetail {
    StereoDetail {
        bands: STEREO_BAND_EDGES_HZ
            .windows(2)
            .map(|e| StereoBand {
                lo_hz: e[0] as f32,
                hi_hz: e[1] as f32,
                correlation: None,
                side_mid_db: Some(0.000_123),
                mono_loss_db: Some(-3.010_3),
            })
            .collect(),
        correlation_windows: None,
        balance_db: Some(60.0),
        one_sided: true,
        haas_lag_ms: None,
    }
}

#[test]
fn stereo_and_dynamics_are_requested_and_reported() {
    let (mut app, cmd_rx) = capture_app();
    let rate = app.sample_rate;
    let job = started_job(roundtrip(
        &mut app,
        request(1, "meter.measure", json!({ "detail": ["stereo", "dynamics"] })),
    ));
    let sent = sent_detail(&cmd_rx);
    assert!(sent.stereo && sent.dynamics && !sent.spectrum, "{sent:?}");

    let mut stereo = hard_panned_stereo();
    stereo.one_sided = false;
    stereo.balance_db = Some(0.123_4);
    stereo.bands[0].correlation = Some(0.987_65);
    stereo.correlation_windows = Some(CorrelationWindows {
        windows: 25,
        pct_below_0_3: 12.345,
        worst: -0.123_45,
        worst_at_seconds: 4.004_2,
    });
    stereo.haas_lag_ms = Some(10.004);
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![rendered(
            StemSource::Master,
            rate,
            MeasurementDetail {
                stereo: Some(stereo),
                dynamics: Some(RangeDynamics {
                    plr_db: Some(19.0),
                    psr_db: Some(16.5),
                }),
                ..MeasurementDetail::default()
            },
        )],
    });
    let wire = result_json(&mut app, job);
    let mut expected = sorted(DEFAULT_KEYS);
    expected.extend(["stereo".to_owned(), "dynamics".to_owned()]);
    expected.sort();
    assert_eq!(keys(&wire), expected);

    let s = &wire["stereo"];
    assert_eq!(s["bands"].as_array().unwrap().len(), 8);
    assert_eq!(
        s["bands"][0],
        json!({
            "lo_hz": 20.0,
            "hi_hz": 60.0,
            "correlation": 0.988,
            "side_mid_db": 0.0,
            "mono_loss_db": -3.0
        })
    );
    assert_eq!(
        s["correlation_windows"],
        json!({ "windows": 25, "pct_below_0_3": 12.3, "worst": -0.123, "worst_at_seconds": 4.0 })
    );
    assert_eq!(s["balance_db"], json!(0.12));
    assert_eq!(s["one_sided"], json!(false));
    assert_eq!(s["haas_lag_ms"], json!(10.0));
    assert_eq!(wire["dynamics"], json!({ "plr_db": 19.0, "psr_db": 16.5 }));

    let typed: MeasureResult = serde_json::from_value(wire).expect("a MeasureResult");
    assert!(typed.stereo.is_some() && typed.dynamics.is_some() && typed.spectrum.is_none());
}

#[test]
fn a_hard_panned_measurement_says_one_sided_with_null_correlations() {
    let (mut app, _cmd_rx) = capture_app();
    let rate = app.sample_rate;
    let job = started_job(roundtrip(
        &mut app,
        request(1, "meter.measure", json!({ "detail": ["stereo"] })),
    ));
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![rendered(
            StemSource::Master,
            rate,
            MeasurementDetail {
                stereo: Some(hard_panned_stereo()),
                ..MeasurementDetail::default()
            },
        )],
    });
    let s = result_json(&mut app, job)["stereo"].clone();
    assert_eq!(s["one_sided"], json!(true));
    assert_eq!(s["balance_db"], json!(60.0));
    assert_eq!(s["correlation_windows"], serde_json::Value::Null);
    assert_eq!(s["haas_lag_ms"], serde_json::Value::Null);
    for band in s["bands"].as_array().unwrap() {
        assert_eq!(band["correlation"], serde_json::Value::Null, "present but null: {band}");
    }
}

#[test]
fn all_three_details_can_be_asked_for_at_once() {
    let (mut app, cmd_rx) = capture_app();
    let _ = started_job(roundtrip(
        &mut app,
        request(
            1,
            "meter.stems",
            json!({ "detail": ["dynamics", "spectrum", "stereo", "stereo"] }),
        ),
    ));
    let sent = sent_detail(&cmd_rx);
    assert!(sent.spectrum && sent.stereo && sent.dynamics, "{sent:?}");
}

#[test]
fn stems_without_detail_adds_no_keys() {
    let (mut app, cmd_rx) = capture_app();
    let rate = app.sample_rate;
    let job = started_job(roundtrip(&mut app, request(1, "meter.stems", json!({}))));
    assert_eq!(sent_detail(&cmd_rx), DetailSet::default());
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: [StemSource::Master, StemSource::Track(DRUMS), StemSource::Track(BASS)]
            .into_iter()
            .map(|t| rendered(t, rate, MeasurementDetail::default()))
            .collect(),
    });
    let wire = result_json(&mut app, job);
    assert_eq!(keys(&wire["master"]), sorted(DEFAULT_KEYS));
    for entry in wire["tracks"].as_array().expect("tracks") {
        let mut expected = sorted(DEFAULT_KEYS);
        expected.extend(["name".to_owned(), "track_id".to_owned()]);
        expected.sort();
        assert_eq!(keys(entry), expected, "{entry}");
    }
}

// ---------------- depth (warmth-width-depth.md §7.6) ----------------

const PAD: u64 = 5;

fn with_depth(drr: Option<f32>, dry_only: bool) -> MeasurementDetail {
    MeasurementDetail {
        depth: Some(DepthDetail {
            hf_tilt_db: Some(-8.765_4),
            drr_db_estimate: drr,
            dry_only,
            sends: if dry_only {
                Vec::new()
            } else {
                vec![DepthSend {
                    bus_id: 50,
                    send_level_db: -10.0,
                    pre_fader: false,
                    return_gain_db: Some(-6.021),
                }]
            },
        }),
        ..MeasurementDetail::default()
    }
}

#[test]
fn stems_depth_ranks_the_tracks_front_to_back() {
    let (mut app, cmd_rx) = capture_app();
    app.test_add_track(PAD, TrackType::Instrument);
    let rate = app.sample_rate;
    let job = started_job(roundtrip(
        &mut app,
        request(1, "meter.stems", json!({ "detail": ["depth"] })),
    ));
    let sent = sent_detail(&cmd_rx);
    assert!(sent.depth && !sent.spectrum, "{sent:?}");

    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![
            rendered(StemSource::Master, rate, with_depth(None, false)),
            // Drums dry-only, bass a little wet, the pad soaked.
            rendered(StemSource::Track(DRUMS), rate, with_depth(None, true)),
            rendered(StemSource::Track(BASS), rate, with_depth(Some(14.25), false)),
            rendered(StemSource::Track(PAD), rate, with_depth(Some(-1.5), false)),
        ],
    });
    let wire = result_json(&mut app, job);
    let entry = |id: u64| {
        wire["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["track_id"] == json!(id))
            .unwrap()["depth"]
            .clone()
    };
    assert_eq!(entry(DRUMS)["layer_hint"], json!("front"));
    assert_eq!(entry(DRUMS)["dry_only"], json!(true));
    assert_eq!(entry(DRUMS)["drr_db_estimate"], serde_json::Value::Null);
    assert!(entry(DRUMS).get("sends").is_none(), "no sends, no key");
    assert_eq!(entry(BASS)["layer_hint"], json!("middle"));
    assert_eq!(entry(PAD)["layer_hint"], json!("back"));
    assert_eq!(entry(PAD)["drr_db_estimate"], json!(-1.5));
    assert_eq!(entry(PAD)["hf_tilt_db"], json!(-8.77));
    assert_eq!(
        entry(PAD)["sends"],
        json!([{
            "bus_id": 50,
            "send_level_db": -10.0,
            "pre_fader": false,
            "return_gain_db": -6.02
        }])
    );
    assert_eq!(
        wire["master"]["depth"]["layer_hint"],
        serde_json::Value::Null,
        "the master is not ranked"
    );
}

#[test]
fn measure_depth_has_no_layer_hint() {
    let (mut app, cmd_rx) = capture_app();
    let rate = app.sample_rate;
    let params = json!({ "target": { "track_id": BASS }, "detail": ["depth"] });
    let job = started_job(roundtrip(&mut app, request(1, "meter.measure", params)));
    assert!(sent_detail(&cmd_rx).depth);
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![rendered(StemSource::Track(BASS), rate, with_depth(Some(3.0), false))],
    });
    let wire = result_json(&mut app, job);
    assert_eq!(wire["depth"]["drr_db_estimate"], json!(3.0));
    assert_eq!(wire["depth"]["layer_hint"], serde_json::Value::Null);
}
