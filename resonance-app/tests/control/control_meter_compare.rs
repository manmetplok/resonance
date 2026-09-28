//! `meter.snapshot` + `meter.compare` (warmth-width-depth.md §7.2, W2).
//!
//! The measurements fed back as engine events are REAL: the engine's own
//! `measure_rendered_buffer_detailed` over a noise buffer and over the
//! same buffer 3 dB louder. So "a pure gain compares to ≈0 after
//! matching" is checked against what the meters actually report at the
//! other gain, not against the compare arithmetic's own assumptions.

use resonance_app::Resonance;
use resonance_audio::test_support::measure_rendered_buffer_detailed;
use resonance_audio::types::{
    AudioCommand, AudioEvent, DetailSet, MixMeasurement, StemSource, TrackType,
};
use resonance_control::job::{JobStarted, JobState, JobStatus};
use resonance_control::methods::meter::{CompareResult, SnapshotResult, SNAPSHOT_CAPACITY};
use resonance_control::{ErrorKind, Request, Response};
use serde_json::{json, Value};

use crate::common::roundtrip;

const TRACK: u64 = 1;
const ALL: DetailSet = DetailSet {
    spectrum: true,
    stereo: true,
    dynamics: true,
    depth: false,
};

fn capture_app() -> (Resonance, crossbeam_channel::Receiver<AudioCommand>) {
    let (mut app, _task, cmd_rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    app.test_add_track(TRACK, TrackType::Instrument);
    while cmd_rx.try_recv().is_ok() {}
    (app, cmd_rx)
}

fn request(id: i64, method: &str, params: Value) -> Request {
    Request::new(id, method, &params).expect("params serialize")
}

fn started_job(response: Response) -> u64 {
    let started: JobStarted = response.result().expect("job started");
    u64::from(started.job_id)
}

fn status(app: &mut Resonance, job: u64) -> JobStatus {
    roundtrip(app, request(99, "job.status", json!({ "job_id": job })))
        .result()
        .expect("job.status succeeds")
}

fn done(app: &mut Resonance, job: u64) -> Value {
    let status = status(app, job);
    assert_eq!(status.state, JobState::Done, "job errored: {:?}", status.error);
    status.result.expect("done carries a result")
}

/// The one `MeasureMix` sent since the last drain: `(targets, range, detail)`.
type Sent = (Vec<StemSource>, Option<(u64, u64)>, DetailSet);

fn sent(cmd_rx: &crossbeam_channel::Receiver<AudioCommand>) -> Vec<Sent> {
    cmd_rx
        .try_iter()
        .filter_map(|c| match c {
            AudioCommand::MeasureMix {
                targets,
                range,
                detail,
                ..
            } => Some((targets, range, detail)),
            _ => None,
        })
        .collect()
}

/// Four seconds of decorrelated-ish stereo noise, `gain_db` louder.
fn noise(rate: u32, gain_db: f32) -> Vec<f32> {
    let gain = 10f32.powf(gain_db / 20.0);
    let mut state = 0x5EED_0042u32;
    let mut next = || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 8) as f32 / 8_388_608.0 - 1.0
    };
    let (mut b0, mut b1) = (0.0f32, 0.0f32);
    (0..rate as usize * 4)
        .flat_map(|_| {
            let (w, v) = (next(), next());
            b0 = 0.99 * b0 + 0.1 * w;
            b1 = 0.95 * b1 + 0.2 * v;
            let common = 0.1 * (b0 + w * 0.3);
            [gain * (common + 0.05 * b1), gain * (common - 0.02 * v)]
        })
        .collect()
}

fn measured(rate: u32, gain_db: f32) -> MixMeasurement {
    let pcm = noise(rate, gain_db);
    let frames = (pcm.len() / 2) as u64;
    measure_rendered_buffer_detailed(StemSource::Master, 0, frames, &pcm, rate, ALL)
}

/// Take a snapshot and resolve it with `m`; returns the snapshot id.
fn snapshot_of(app: &mut Resonance, m: MixMeasurement) -> u64 {
    let job = started_job(roundtrip(app, request(1, "meter.snapshot", json!({}))));
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![m],
    });
    let result: SnapshotResult = serde_json::from_value(done(app, job)).expect("a SnapshotResult");
    result.snapshot_id
}

/// Every number anywhere under `v`.
fn numbers(v: &Value, out: &mut Vec<f64>) {
    match v {
        Value::Number(n) => out.push(n.as_f64().unwrap()),
        Value::Array(items) => items.iter().for_each(|i| numbers(i, out)),
        Value::Object(map) => map.values().for_each(|i| numbers(i, out)),
        _ => {}
    }
}

/// The delta fields that are band edges, not deltas.
fn without_edges(mut deltas: Value) -> Value {
    if let Some(bands) = deltas["stereo"]["bands"].as_array_mut() {
        for band in bands {
            let band = band.as_object_mut().unwrap();
            band.remove("lo_hz");
            band.remove("hi_hz");
        }
    }
    deltas
}

#[test]
fn a_snapshot_renders_every_detail_and_keeps_the_measurement() {
    let (mut app, cmd_rx) = capture_app();
    let rate = app.sample_rate;
    let job = started_job(roundtrip(&mut app, request(1, "meter.snapshot", json!({}))));
    let commands = sent(&cmd_rx);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].0, vec![StemSource::Master]);
    assert_eq!(commands[0].2, ALL, "a snapshot keeps every detail by default");

    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![measured(rate, 0.0)],
    });
    let wire = done(&mut app, job);
    assert!(wire["snapshot_id"].as_u64().is_some(), "{wire}");
    let m = &wire["measurement"];
    for block in ["spectrum", "stereo", "dynamics"] {
        assert!(m[block].is_object(), "{block} kept: {m}");
    }
}

#[test]
fn a_snapshot_can_keep_fewer_details() {
    let (mut app, cmd_rx) = capture_app();
    let _ = started_job(roundtrip(
        &mut app,
        request(1, "meter.snapshot", json!({ "detail": ["stereo"] })),
    ));
    let detail = sent(&cmd_rx)[0].2;
    assert!(detail.stereo && !detail.spectrum && !detail.dynamics, "{detail:?}");
}

#[test]
fn the_same_state_compares_to_all_zero_deltas() {
    let (mut app, cmd_rx) = capture_app();
    let rate = app.sample_rate;
    let id = snapshot_of(&mut app, measured(rate, 0.0));
    while cmd_rx.try_recv().is_ok() {}

    let job = started_job(roundtrip(&mut app, request(2, "meter.compare", json!({ "a": id }))));
    let commands = sent(&cmd_rx);
    assert_eq!(commands.len(), 1, "\"current\" renders once");
    let frames = u64::from(rate) * 4;
    assert_eq!(commands[0].1, Some((0, frames)), "exactly the snapshot's samples");
    assert_eq!(commands[0].2, ALL);

    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![measured(rate, 0.0)],
    });
    let wire = done(&mut app, job);
    let result: CompareResult = serde_json::from_value(wire.clone()).expect("a CompareResult");
    assert!(result.matched);
    assert_eq!(result.match_gain_db, 0.0);
    for block in ["spectrum", "stereo", "dynamics"] {
        assert!(wire["deltas"][block].is_object(), "{block} delta present");
    }
    let mut all = Vec::new();
    numbers(&without_edges(wire["deltas"].clone()), &mut all);
    assert!(all.len() > 60, "every proxy is compared: {}", all.len());
    assert!(all.iter().all(|&d| d == 0.0), "identical states: {}", wire["deltas"]);
}

#[test]
fn a_three_db_gain_alone_compares_to_zero_after_matching() {
    let (mut app, _cmd_rx) = capture_app();
    let rate = app.sample_rate;
    let id = snapshot_of(&mut app, measured(rate, 0.0));

    let job = started_job(roundtrip(&mut app, request(2, "meter.compare", json!({ "a": id }))));
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![measured(rate, 3.0)],
    });
    let wire = done(&mut app, job);
    let result: CompareResult = serde_json::from_value(wire.clone()).unwrap();
    assert!(result.matched);
    assert!((result.match_gain_db - -3.0).abs() < 0.02, "{}", result.match_gain_db);

    let mut all = Vec::new();
    numbers(&without_edges(wire["deltas"].clone()), &mut all);
    let worst = all.iter().fold(0.0f64, |m, d| m.max(d.abs()));
    assert!(
        worst <= 0.05,
        "a pure gain must vanish after matching: worst {worst}, {}",
        wire["deltas"]
    );

    // Unmatched, the same pair shows the gain in every level figure and
    // nowhere else.
    let job = started_job(roundtrip(
        &mut app,
        request(3, "meter.compare", json!({ "a": id, "match": "none" })),
    ));
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![measured(rate, 3.0)],
    });
    let raw: CompareResult = serde_json::from_value(done(&mut app, job)).unwrap();
    assert!(!raw.matched && raw.match_gain_db == 0.0);
    let d = &raw.deltas;
    for level in [d.lufs_integrated, d.true_peak_db, d.lufs_short_max] {
        assert!((level.unwrap() - 3.0).abs() < 0.05, "{level:?}");
    }
    assert!(d.crest_db.unwrap().abs() < 0.05, "crest is a shape figure");
    let spectrum = d.spectrum.as_ref().unwrap();
    assert!(spectrum.tilt_db_per_oct.unwrap().abs() < 0.05);
    assert!((spectrum.third_octave[17].unwrap() - 3.0).abs() < 0.05);
}

#[test]
fn two_snapshots_compare_without_a_render() {
    let (mut app, cmd_rx) = capture_app();
    let rate = app.sample_rate;
    let a = snapshot_of(&mut app, measured(rate, 0.0));
    let b = snapshot_of(&mut app, measured(rate, 3.0));
    while cmd_rx.try_recv().is_ok() {}
    let job = started_job(roundtrip(
        &mut app,
        request(3, "meter.compare", json!({ "a": a, "b": b })),
    ));
    assert!(sent(&cmd_rx).is_empty(), "nothing to render");
    let result: CompareResult = serde_json::from_value(done(&mut app, job)).unwrap();
    assert!((result.match_gain_db - -3.0).abs() < 0.02);
    assert_eq!(serde_json::to_value(result.a.side).unwrap(), json!(a));
    assert_eq!(serde_json::to_value(result.b.side).unwrap(), json!(b));
}

#[test]
fn current_against_current_renders_once_and_compares_to_zero() {
    let (mut app, cmd_rx) = capture_app();
    let rate = app.sample_rate;
    let job = started_job(roundtrip(
        &mut app,
        request(1, "meter.compare", json!({ "a": "current", "b": "current" })),
    ));
    assert_eq!(sent(&cmd_rx).len(), 1);
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![measured(rate, 0.0)],
    });
    let result: CompareResult = serde_json::from_value(done(&mut app, job)).unwrap();
    assert_eq!(result.deltas.lufs_integrated, Some(0.0));
    assert_eq!(serde_json::to_value(result.a.side).unwrap(), json!("current"));
}

#[test]
fn an_unknown_snapshot_is_not_found_and_starts_nothing() {
    let (mut app, cmd_rx) = capture_app();
    let response = roundtrip(&mut app, request(1, "meter.compare", json!({ "a": 777 })));
    let error = response.error.expect("refused");
    assert_eq!(error.kind(), ErrorKind::NotFound, "{error:?}");
    assert!(sent(&cmd_rx).is_empty());
}

#[test]
fn a_target_that_contradicts_the_snapshot_is_invalid() {
    let (mut app, _cmd_rx) = capture_app();
    let rate = app.sample_rate;
    let id = snapshot_of(&mut app, measured(rate, 0.0));
    let response = roundtrip(
        &mut app,
        request(1, "meter.compare", json!({ "a": id, "target": { "track_id": TRACK } })),
    );
    let error = response.error.expect("refused");
    assert_eq!(error.kind(), ErrorKind::InvalidParams, "{error:?}");
}

#[test]
fn a_silent_side_is_compared_unmatched() {
    let (mut app, _cmd_rx) = capture_app();
    let rate = app.sample_rate;
    let silence = vec![0.0f32; rate as usize * 8];
    let frames = u64::from(rate) * 4;
    let silent =
        measure_rendered_buffer_detailed(StemSource::Master, 0, frames, &silence, rate, ALL);
    let id = snapshot_of(&mut app, silent);
    let job = started_job(roundtrip(&mut app, request(2, "meter.compare", json!({ "a": id }))));
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![measured(rate, 0.0)],
    });
    let result: CompareResult = serde_json::from_value(done(&mut app, job)).unwrap();
    assert!(!result.matched, "nothing to match against silence");
    assert_eq!(result.deltas.lufs_integrated, None, "no loudness on one side");
}

#[test]
fn the_least_recently_used_snapshot_is_evicted_past_the_cap() {
    let (mut app, _cmd_rx) = capture_app();
    let rate = app.sample_rate;
    let m = measured(rate, 0.0);
    let first = snapshot_of(&mut app, m.clone());
    let second = snapshot_of(&mut app, m.clone());
    for _ in 2..SNAPSHOT_CAPACITY {
        snapshot_of(&mut app, m.clone());
    }
    // Using `first` makes `second` the least recently used.
    let job = started_job(roundtrip(
        &mut app,
        request(5, "meter.compare", json!({ "a": first, "b": first })),
    ));
    done(&mut app, job);
    snapshot_of(&mut app, m);

    let gone = roundtrip(&mut app, request(6, "meter.compare", json!({ "a": second, "b": first })));
    assert_eq!(gone.error.expect("evicted").kind(), ErrorKind::NotFound);
    let kept = started_job(roundtrip(
        &mut app,
        request(7, "meter.compare", json!({ "a": first, "b": first })),
    ));
    done(&mut app, kept);
}
