//! `meter.probe` (warmth-width-depth.md §7.3, W3): the handler turns an
//! owner's chain into the engine's probe stages — instrument left out,
//! bypassed / missing slots skipped and listed — and reads the engine's
//! report into the wire result. The probe math and the cloned engine
//! chain are pinned in `resonance-metering/tests/probe.rs` and
//! `resonance-audio/tests/clap_host/probe_chain.rs`.

use resonance_app::state::{PluginAvailability, PluginSlotState};
use resonance_app::Resonance;
use resonance_audio::types::{
    AudioCommand, AudioEvent, ChainProbeReport, ProbeSpec, ProbeStage, ProbedStage, TrackType,
};
use resonance_control::job::{JobStarted, JobState, JobStatus};
use resonance_control::methods::meter::ProbeResult;
use resonance_control::{ErrorKind, Request, Response};
use resonance_metering::probe::HarmonicReport;
use serde_json::{json, Value};

use crate::common::roundtrip;

const SYNTH: u64 = 1;
const AUDIO: u64 = 2;

fn slot(id: u64, plugin: &str) -> PluginSlotState {
    PluginSlotState::new(
        id,
        format!("{plugin} name"),
        plugin.to_owned(),
        format!("/plugins/{plugin}.clap"),
        Vec::new(),
        false,
    )
}

fn capture_app() -> (Resonance, crossbeam_channel::Receiver<AudioCommand>) {
    let (mut app, _task, cmd_rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    app.test_add_track(SYNTH, TrackType::Instrument);
    app.test_add_track(AUDIO, TrackType::Audio);
    // An instrument track: slot 0 is the synth, then two inserts.
    app.test_push_track_plugin(SYNTH, slot(10, "com.example.synth"));
    app.test_push_track_plugin(SYNTH, slot(11, "com.resonance.eq"));
    app.test_push_track_plugin(SYNTH, slot(12, "com.resonance.color"));
    // An audio track: a bypassed EQ, a missing plugin, a saturator.
    let mut bypassed = slot(20, "com.resonance.eq");
    bypassed.bypassed = true;
    let mut missing = slot(21, "com.example.gone");
    missing.availability = PluginAvailability::Missing {
        reason: "not installed".into(),
    };
    app.test_push_track_plugin(AUDIO, bypassed);
    app.test_push_track_plugin(AUDIO, missing);
    app.test_push_track_plugin(AUDIO, slot(22, "com.resonance.mastering"));
    while cmd_rx.try_recv().is_ok() {}
    (app, cmd_rx)
}

fn request(id: i64, params: Value) -> Request {
    Request::new(id, "meter.probe", &params).expect("params serialize")
}

fn started_job(response: Response) -> u64 {
    let started: JobStarted = response.result().expect("job started");
    u64::from(started.job_id)
}

fn status(app: &mut Resonance, job: u64) -> JobStatus {
    let request = Request::new(99, "job.status", &json!({ "job_id": job })).unwrap();
    roundtrip(app, request).result().expect("job.status succeeds")
}

type Sent = (u64, Vec<ProbeStage>, ProbeSpec);

fn sent(cmd_rx: &crossbeam_channel::Receiver<AudioCommand>) -> Vec<Sent> {
    cmd_rx
        .try_iter()
        .filter_map(|c| match c {
            AudioCommand::ProbeChain {
                probe_id,
                stages,
                spec,
            } => Some((probe_id, stages, spec)),
            _ => None,
        })
        .collect()
}

fn report(stages: &[ProbeStage]) -> ChainProbeReport {
    ChainProbeReport {
        stages: stages
            .iter()
            .map(|s| ProbedStage {
                instance_id: s.instance_id,
                state_copied: true,
            })
            .collect(),
        harmonics: HarmonicReport {
            freq_hz: 999.755_859_375,
            fundamental_dbfs: -10.512_34,
            thd_pct: 1.234_567_89,
            h: vec![
                Some(-38.123),
                Some(-45.678),
                Some(-60.0),
                Some(-70.0),
                Some(-80.0),
                Some(-90.0),
                Some(-100.0),
                Some(-110.0),
            ],
            h2_h3_db: Some(7.555),
            decay_db_per_order: Some(10.123),
            aliasing_floor_dbc: -123.456,
        },
        imd_pct: None,
        latency_samples: 64,
    }
}

#[test]
fn a_track_probe_leaves_out_the_instrument() {
    let (mut app, cmd_rx) = capture_app();
    let params = json!({ "target": { "track_id": SYNTH } });
    let job = started_job(roundtrip(&mut app, request(1, params)));
    let commands = sent(&cmd_rx);
    assert_eq!(commands.len(), 1);
    let (probe_id, stages, spec) = &commands[0];
    assert_eq!(*probe_id, job, "the job id is the correlation token");
    let ids: Vec<u64> = stages.iter().map(|s| s.instance_id).collect();
    assert_eq!(ids, vec![11, 12], "the synth generates, it is not an insert");
    assert_eq!(stages[0].clap_file_path, "/plugins/com.resonance.eq.clap");
    assert_eq!(
        *spec,
        ProbeSpec {
            freq_hz: 1_000.0,
            level_dbfs: -12.0,
            imd: false
        },
        "defaults"
    );
}

#[test]
fn bypassed_and_missing_slots_are_skipped_and_listed() {
    let (mut app, cmd_rx) = capture_app();
    let job = started_job(roundtrip(
        &mut app,
        request(
            1,
            json!({
                "target": { "track_id": AUDIO },
                "freq_hz": 5000,
                "level_dbfs": -6,
                "imd": true
            }),
        ),
    ));
    let (_, stages, spec) = sent(&cmd_rx).remove(0);
    assert_eq!(stages.iter().map(|s| s.instance_id).collect::<Vec<_>>(), vec![22]);
    assert!(spec.imd && spec.freq_hz == 5_000.0 && spec.level_dbfs == -6.0);

    app.test_apply_engine_event(AudioEvent::ChainProbed {
        probe_id: job,
        report: report(&stages),
    });
    let status = status(&mut app, job);
    assert_eq!(status.state, JobState::Done, "{:?}", status.error);
    let wire = status.result.unwrap();
    assert_eq!(
        wire["skipped"],
        json!([
            { "plugin_id": "com.resonance.eq", "occurrence": 0, "reason": "bypassed" },
            { "plugin_id": "com.example.gone", "occurrence": 0, "reason": "missing" }
        ])
    );
    let r: ProbeResult = serde_json::from_value(wire).unwrap();
    assert_eq!(r.stages.len(), 1);
    assert_eq!(r.stages[0].plugin_id, "com.resonance.mastering");
    assert!(r.stages[0].state_copied);
    assert_eq!(r.level_dbfs, -6.0);
    assert_eq!(r.gain_db, -4.51, "output fundamental minus input level");
    assert_eq!(r.thd_pct, 1.23457);
    assert_eq!(r.h[0], Some(-38.12));
    assert_eq!(r.h2_h3_db, Some(7.56));
    assert_eq!(r.aliasing_floor_dbc, -123.46);
    assert_eq!(r.latency_samples, 64);
    assert_eq!(r.freq_hz, 999.756);
}

#[test]
fn a_chain_bypass_skips_every_insert() {
    let (mut app, cmd_rx) = capture_app();
    app.test_apply_engine_event(AudioEvent::MasterFxBypassChanged { bypassed: true });
    let mut master = slot(30, "com.resonance.mastering");
    master.bypassed = false;
    app.test_push_master_plugin(master);
    let _ = started_job(roundtrip(&mut app, request(1, json!({}))));
    let (_, stages, _) = sent(&cmd_rx).remove(0);
    assert!(stages.is_empty(), "a bypassed chain is a straight wire");
}

#[test]
fn an_engine_error_fails_the_job() {
    let (mut app, _cmd_rx) = capture_app();
    let params = json!({ "target": { "track_id": AUDIO } });
    let job = started_job(roundtrip(&mut app, request(1, params)));
    app.test_apply_engine_event(AudioEvent::ChainProbeError {
        probe_id: job,
        message: "could not clone x".into(),
    });
    let status = status(&mut app, job);
    assert_eq!(status.state, JobState::Error);
    assert_eq!(status.error.unwrap().message, "could not clone x");
}

#[test]
fn out_of_range_stimulus_and_unknown_targets_are_refused() {
    let (mut app, cmd_rx) = capture_app();
    for (params, kind) in [
        (json!({ "freq_hz": 5.0 }), ErrorKind::InvalidParams),
        (json!({ "freq_hz": 30_000.0 }), ErrorKind::InvalidParams),
        (json!({ "level_dbfs": 3.0 }), ErrorKind::InvalidParams),
        (json!({ "target": { "track_id": 999 } }), ErrorKind::NotFound),
    ] {
        let error = roundtrip(&mut app, request(1, params.clone())).error.expect("refused");
        assert_eq!(error.kind(), kind, "{params}: {error:?}");
    }
    assert!(sent(&cmd_rx).is_empty(), "nothing reached the engine");
}

#[test]
fn a_probe_runs_while_a_render_measurement_is_in_flight() {
    let (mut app, cmd_rx) = capture_app();
    let _ = started_job(roundtrip(
        &mut app,
        Request::new(1, "meter.measure", &json!({})).unwrap(),
    ));
    let params = json!({ "target": { "track_id": AUDIO } });
    let _ = started_job(roundtrip(&mut app, request(2, params)));
    assert_eq!(sent(&cmd_rx).len(), 1, "the probe clones its chain, it contends with nothing");
}
