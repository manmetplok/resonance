//! `meter.*` control handlers (ba doc #273, todos #1219 / #1220):
//! measure the mix over the control API instead of bouncing WAVs and
//! analysing them.
//!
//! `meter.measure` — the job round trip (start -> engine event -> typed
//! result), target resolution for master / track / bus / an id that does
//! not exist, the range clamp, the guards that keep a measurement off a
//! busy renderer, and — the point of the whole result shape — that a
//! number the meters cannot supply comes back as `null` rather than as a
//! zero that reads like a measurement.
//!
//! `meter.stems` — master plus ONE entry per top-level track from ONE
//! engine pass, with the drum kit's sub-track folded into its parent
//! instead of competing with it, ids and names that join to
//! `song.summary`, and one shared range across every entry.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::{TrackState, ViewMode};
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::{
    AudioEvent, MeasureSource as EngineSource, MixMeasurement, StemSource, TrackType,
};
use resonance_control::job::{JobStarted, JobState, JobStatus};
use resonance_control::methods::control::HelloResult;
use resonance_control::methods::meter::{
    MeasureResult, MeasureSource, MeasureTarget, StemsResult,
};
use resonance_control::{ErrorKind, Request, Response};
use resonance_metering::offline::BandShares;
use serde_json::json;

const DRUMS: u64 = 1;
const KICK: u64 = 2;
const BASS: u64 = 3;
const BUS: u64 = 9_000;

fn app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_add_track(DRUMS, TrackType::Instrument);
    app.test_add_track(BASS, TrackType::Instrument);
    // A sub-track of the multi-output drum instrument, as
    // `ensure_subtracks` would create for output port 1.
    app.test_push_track(TrackState::new_sub_track(
        KICK,
        2,
        "Drums → Kick".to_owned(),
        DRUMS,
        1,
    ));
    app.test_apply_engine_event(AudioEvent::BusAdded {
        bus_id: BUS,
        name: "Drum Bus".to_owned(),
    });
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

/// The job's terminal result, decoded.
fn measured(app: &mut Resonance, job: u64) -> MeasureResult {
    let status = job_status(app, job);
    assert_eq!(status.state, JobState::Done, "job errored: {:?}", status.error);
    serde_json::from_value(status.result.expect("done carries a result"))
        .expect("result is a MeasureResult")
}

/// The job's terminal result as raw JSON, for asserting wire shape
/// (present-but-null vs absent).
fn measured_json(app: &mut Resonance, job: u64) -> serde_json::Value {
    job_status(app, job).result.expect("done carries a result")
}

/// A plausible rendered measurement of `target`: 10 s of non-silent
/// material at a realistic mix level.
fn rendered(target: StemSource, sample_rate: u32) -> MixMeasurement {
    let frames = u64::from(sample_rate) * 10;
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
    }
}

/// What the engine reports off the live master tap: real streaming
/// readings, documented placeholders everywhere else.
///
/// This mirrors `bounce::measure::from_live_snapshot` field for field,
/// INCLUDING the two values that look like readings but are not:
/// `crest_db` and `correlation` come straight off a `MeterSnapshot`
/// that `ABMeterTap::snapshot` never writes (it ends
/// `..MeterSnapshot::default()`, and neither `CrestMeter` nor
/// `CorrelationMeter` is instantiated anywhere), so they are always
/// exactly 0.0 no matter what is playing. A fixture that invented
/// plausible numbers here would let the handler pass a fabrication
/// through unnoticed — which is what it did until this was fixed.
fn live_snapshot() -> MixMeasurement {
    MixMeasurement {
        target: StemSource::Master,
        source: EngineSource::Live,
        range_start: 0,
        range_end: 0,
        frames: 0,
        lufs_integrated: -21.0,
        // The tap's CURRENT windows, not maxima.
        lufs_short_term_max: -19.0,
        lufs_momentary_max: -17.0,
        // Session-cumulative, not figures for a range.
        lra_lu: 5.0,
        true_peak_dbtp: -1.0,
        // Placeholders the streaming tap cannot fill.
        sample_peak_db: -120.0,
        crest_db: 0.0,
        clipped_samples: 0,
        correlation: 0.0,
        mono_penalty_db: 0.0,
        bands: BandShares::SILENT,
    }
}

/// A 4-second audio clip at the song's start, so the project has an end
/// for a range to be clamped to.
fn four_second_clip(sample_rate: u32) -> resonance_app::state::ClipState {
    resonance_app::state::ClipState {
        id: 1,
        track_id: DRUMS,
        start_sample: 0,
        duration_samples: u64::from(sample_rate) * 4,
        name: "take".to_owned(),
        total_frames: u64::from(sample_rate) * 4,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: resonance_audio::types::FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: resonance_audio::types::FadeCurve::default(),
        gain_db: 0.0,
        waveform_peaks: Vec::new(),
        vocal_tuning: None,
        asset_ref: None,
    }
}

// ---------------- capability + job round trip ----------------

#[test]
fn hello_lists_meter_measure() {
    let mut app = app();
    let hello: HelloResult = roundtrip(
        &mut app,
        request(1, "control.hello", json!({ "protocol_version": 1 })),
    )
    .result()
    .expect("hello succeeds");
    assert!(
        hello.capabilities.iter().any(|m| m == "meter.measure"),
        "meter.measure missing from capabilities: {:?}",
        hello.capabilities
    );
}

#[test]
fn measuring_the_master_returns_every_figure() {
    let mut app = app();
    let rate = app.sample_rate;
    let job = started_job(roundtrip(&mut app, request(1, "meter.measure", json!({}))));
    assert_eq!(job_status(&mut app, job).state, JobState::Pending);

    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![rendered(StemSource::Master, rate)],
    });

    let result = measured(&mut app, job);
    assert_eq!(result.target, MeasureTarget::Master);
    assert_eq!(result.source, MeasureSource::Render);
    assert_eq!(result.lufs_integrated, Some(-20.5));
    assert_eq!(result.lufs_short_max, Some(-18.0));
    assert_eq!(result.lufs_momentary_max, Some(-16.5));
    assert_eq!(result.lra, 7.25);
    assert_eq!(result.true_peak_db, -1.5);
    assert_eq!(result.sample_peak_db, Some(-2.0));
    assert_eq!(result.crest_db, Some(12.5));
    assert_eq!(result.clipped_samples, Some(0));
    assert_eq!(result.correlation, Some(0.82));
    assert_eq!(result.mono_penalty_db, Some(-0.4));
    let bands = result.bands.expect("render path carries bands");
    assert_eq!((bands.low, bands.mid, bands.high, bands.air), (0.4, 0.35, 0.2, 0.05));
    assert_eq!(result.measured_seconds, Some(10.0));
    // The live-only readings never appear on the render path.
    assert_eq!(result.lufs_short_term_now, None);
    assert_eq!(result.lufs_momentary_now, None);
}

#[test]
fn a_track_target_is_echoed_back_on_its_result() {
    let mut app = app();
    let rate = app.sample_rate;
    let job = started_job(roundtrip(
        &mut app,
        request(1, "meter.measure", json!({ "target": { "track_id": DRUMS } })),
    ));
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![rendered(StemSource::Track(DRUMS), rate)],
    });
    assert_eq!(
        measured(&mut app, job).target,
        MeasureTarget::Track(DRUMS.into())
    );
}

/// A bus is reachable both by its explicit spelling and — because
/// `song.summary` reports busses as track lines from the same id space —
/// as a `track_id`.
#[test]
fn a_bus_is_reachable_under_both_spellings() {
    let mut app = app();
    let rate = app.sample_rate;

    for (id, params) in [
        (1, json!({ "target": { "bus_id": BUS } })),
        (2, json!({ "target": { "track_id": BUS } })),
    ] {
        let job = started_job(roundtrip(&mut app, request(id, "meter.measure", params)));
        app.test_apply_engine_event(AudioEvent::MixMeasured {
            measure_id: job,
            results: vec![rendered(StemSource::Bus(BUS), rate)],
        });
        assert_eq!(measured(&mut app, job).target, MeasureTarget::Bus(BUS.into()));
    }
}

#[test]
fn an_engine_failure_fails_the_job_with_its_reason() {
    let mut app = app();
    let job = started_job(roundtrip(&mut app, request(1, "meter.measure", json!({}))));
    app.test_apply_engine_event(AudioEvent::MixMeasureError {
        measure_id: job,
        message: "Another offline render is in progress".to_owned(),
    });
    let status = job_status(&mut app, job);
    assert_eq!(status.state, JobState::Error);
    assert_eq!(
        status.error.as_deref(),
        Some("Another offline render is in progress")
    );
}

// ---------------- honesty of the result shape ----------------

/// The whole point of the nullable fields: a figure the meters cannot
/// supply must not come back as a zero that reads like a measurement.
#[test]
fn the_live_path_nulls_what_the_streaming_tap_cannot_supply() {
    let mut app = app();
    let job = started_job(roundtrip(
        &mut app,
        request(1, "meter.measure", json!({ "source": "live" })),
    ));
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![live_snapshot()],
    });

    let result = measured(&mut app, job);
    assert_eq!(result.source, MeasureSource::Live);
    // Real streaming readings survive — the three the tap does measure.
    // (lra and true_peak_db are session-cumulative on this path, which
    // the field docs and the tool description say; they are still real.)
    assert_eq!(result.lufs_integrated, Some(-21.0));
    assert_eq!(result.true_peak_db, -1.0);
    assert_eq!(result.lra, 5.0);
    // The tap keeps no history, so these are NOT maxima and must not be
    // reported as such — they move to the `_now` fields.
    assert_eq!(result.lufs_short_max, None);
    assert_eq!(result.lufs_momentary_max, None);
    assert_eq!(result.lufs_short_term_now, Some(-19.0));
    assert_eq!(result.lufs_momentary_now, Some(-17.0));
    // Whole-buffer figures the tap never sees. The engine carries
    // placeholders (-120 dBFS, 0 samples, 0 dB, silent bands); none of
    // them reach the wire as a number.
    assert_eq!(result.sample_peak_db, None);
    assert_eq!(result.clipped_samples, None);
    assert_eq!(result.mono_penalty_db, None);
    assert!(result.bands.is_none());
    assert_eq!(result.measured_seconds, None);
    // The two the tap runs no meter for at all. Their engine-side value
    // is a hard 0.0, which is INSIDE each field's plausible range —
    // "maximally squashed" and "perfectly wide" — so passing it through
    // would be indistinguishable from a reading. Null is the only
    // honest answer.
    assert_eq!(result.crest_db, None);
    assert_eq!(result.correlation, None);

    // On the wire they are explicit nulls, not omissions: an agent must
    // see that the field exists and was not measurable.
    let wire = measured_json(&mut app, job);
    for field in [
        "sample_peak_db",
        "clipped_samples",
        "crest_db",
        "correlation",
        "mono_penalty_db",
        "bands",
    ] {
        assert_eq!(wire[field], serde_json::Value::Null, "{field} on the wire");
    }
    assert_eq!(wire["source"], json!("live"));
    // The live-only readings are absent on the render path rather than
    // null, so they never add noise to the common case.
    assert_eq!(wire["lufs_short_term_now"], json!(-19.0));
}

/// A silent render leaves the LUFS meters at `-inf`, which JSON cannot
/// carry. It must arrive as `null`, never as a bogus level.
#[test]
fn silence_reports_null_loudness_rather_than_negative_infinity() {
    let mut app = app();
    let rate = app.sample_rate;
    let mut silent = rendered(StemSource::Master, rate);
    silent.lufs_integrated = f32::NEG_INFINITY;
    silent.lufs_short_term_max = f32::NEG_INFINITY;
    silent.lufs_momentary_max = f32::NEG_INFINITY;

    let job = started_job(roundtrip(&mut app, request(1, "meter.measure", json!({}))));
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![silent],
    });

    let result = measured(&mut app, job);
    assert_eq!(result.lufs_integrated, None);
    assert_eq!(result.lufs_short_max, None);
    assert_eq!(result.lufs_momentary_max, None);
    let wire = measured_json(&mut app, job);
    assert_eq!(wire["lufs_integrated"], serde_json::Value::Null);
}

// ---------------- params ----------------

#[test]
fn an_unknown_id_is_not_found_and_starts_nothing() {
    let mut app = app();
    for params in [
        json!({ "target": { "track_id": 4_242 } }),
        json!({ "target": { "bus_id": 4_242 } }),
        // A real track id is still not a bus.
        json!({ "target": { "bus_id": BASS } }),
    ] {
        let response = roundtrip(&mut app, request(1, "meter.measure", params.clone()));
        assert_eq!(
            response.error.unwrap_or_else(|| panic!("{params} should fail")).kind(),
            ErrorKind::NotFound
        );
    }
}

#[test]
fn a_range_past_the_end_of_the_song_is_clamped_not_refused() {
    let mut app = app();
    let rate = app.sample_rate;
    // Give the song some length so there is something to clamp to.
    app.test_push_clip(four_second_clip(rate));

    let job = started_job(roundtrip(
        &mut app,
        request(
            1,
            "meter.measure",
            json!({ "range": { "start": { "sample": 0 }, "end": { "sample": u64::from(rate) * 600 } } }),
        ),
    ));
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![rendered(StemSource::Master, rate)],
    });
    assert_eq!(measured(&mut app, job).source, MeasureSource::Render);
}

#[test]
fn an_empty_or_backwards_range_is_invalid_params() {
    let mut app = app();
    let response = roundtrip(
        &mut app,
        request(
            1,
            "meter.measure",
            json!({ "range": { "start": { "sample": 5_000 }, "end": { "sample": 1_000 } } }),
        ),
    );
    assert_eq!(
        response.error.expect("backwards range").kind(),
        ErrorKind::InvalidParams
    );
}

#[test]
fn an_omitted_range_measures_the_whole_song() {
    let mut app = app();
    // No `range` key at all, and an explicitly empty one, both start.
    for (id, params) in [(1, json!({})), (2, json!({ "range": {} }))] {
        let job = started_job(roundtrip(&mut app, request(id, "meter.measure", params)));
        assert_eq!(job_status(&mut app, job).state, JobState::Pending);
        app.test_apply_engine_event(AudioEvent::MixMeasureError {
            measure_id: job,
            message: "done".to_owned(),
        });
    }
}

// ---------------- guards ----------------

#[test]
fn live_is_refused_for_anything_but_the_master() {
    let mut app = app();
    let response = roundtrip(
        &mut app,
        request(
            1,
            "meter.measure",
            json!({ "target": { "track_id": DRUMS }, "source": "live" }),
        ),
    );
    let error = response.error.expect("live on a track is refused");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert!(
        error.message.contains("master"),
        "the error must say why: {}",
        error.message
    );
}

#[test]
fn live_refuses_a_range_rather_than_ignoring_it() {
    let mut app = app();
    let response = roundtrip(
        &mut app,
        request(
            1,
            "meter.measure",
            json!({ "source": "live", "range": { "start": { "bar": 2 } } }),
        ),
    );
    assert_eq!(
        response.error.expect("live has no range").kind(),
        ErrorKind::InvalidParams
    );
}

#[test]
fn measuring_while_a_render_is_in_flight_is_busy() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut app = app();

    // Start a real mixdown so the app-side render flag is set.
    let target = dir.path().join("mix.wav").display().to_string();
    let _ = roundtrip(&mut app, request(1, "render.mixdown", json!({ "path": target })));
    assert!(app.test_is_bouncing());

    let response = roundtrip(&mut app, request(2, "meter.measure", json!({})));
    let error = response.error.expect("render guard");
    assert_eq!(error.kind(), ErrorKind::Busy);
    assert!(
        error.message.contains("render"),
        "the error must name the render: {}",
        error.message
    );

    // The live meter needs no renderer, so it stays available.
    let job = started_job(roundtrip(
        &mut app,
        request(3, "meter.measure", json!({ "source": "live" })),
    ));
    assert_eq!(job_status(&mut app, job).state, JobState::Pending);
}

#[test]
fn measuring_while_recording_is_busy() {
    let mut app = app();
    app.test_set_transport_recording(true);
    let response = roundtrip(&mut app, request(1, "meter.measure", json!({})));
    assert_eq!(
        response.error.expect("recording guard").kind(),
        ErrorKind::Busy
    );
}

#[test]
fn a_second_measurement_while_one_is_in_flight_is_busy() {
    let mut app = app();
    let rate = app.sample_rate;
    let job = started_job(roundtrip(&mut app, request(1, "meter.measure", json!({}))));

    let response = roundtrip(&mut app, request(2, "meter.measure", json!({})));
    assert_eq!(
        response.error.expect("one measurement at a time").kind(),
        ErrorKind::Busy
    );

    // Once the first resolves, the next one is accepted.
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: vec![rendered(StemSource::Master, rate)],
    });
    let _ = measured(&mut app, job);
    let next = started_job(roundtrip(&mut app, request(3, "meter.measure", json!({}))));
    assert_eq!(job_status(&mut app, next).state, JobState::Pending);
}

/// ba todo #1243, the correlation fix, pinned on the order that actually
/// mis-attributes.
///
/// The old rule was "resolve the newest live Measure job"
/// (`max_by_key(id)` over non-terminal Measure jobs), and `next_id` is
/// monotonic — so the job asked for SECOND always won. Landing the live
/// event first would therefore have resolved the live job correctly even
/// without a correlation id; that ordering proves nothing.
///
/// The hazard is the other way round, and it is what the guard this todo
/// relaxes existed to prevent: the RENDER measurement (asked first, so
/// the older id) answers while the live job is still open. Under the old
/// rule "newest" named the LIVE job, so the render's numbers completed
/// it — a caller asking for a live meter reading receiving a rendered
/// measurement of the whole song. With `measure_id` each event names its
/// own job, so this now resolves the render job and leaves the live one
/// untouched.
#[test]
fn a_render_event_resolves_its_own_job_not_the_newer_live_one() {
    let mut app = app();
    let rate = app.sample_rate;

    let render = started_job(roundtrip(&mut app, request(1, "meter.measure", json!({}))));
    let live = started_job(roundtrip(
        &mut app,
        request(2, "meter.measure", json!({ "source": "live" })),
    ));
    assert!(
        live > render,
        "the live job must be the NEWER of the two ({render} then {live}),          or this test is not exercising the mis-attributing case"
    );
    assert_eq!(job_status(&mut app, render).state, JobState::Pending);
    assert_eq!(job_status(&mut app, live).state, JobState::Pending);

    // The render answers while the live job is open. This is the event
    // that used to complete the live job with rendered numbers.
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: render,
        results: vec![rendered(StemSource::Master, rate)],
    });
    assert_eq!(
        measured(&mut app, render).source,
        MeasureSource::Render,
        "the render job gets the render numbers"
    );
    assert_eq!(
        job_status(&mut app, live).state,
        JobState::Pending,
        "and the newer live job is untouched — this is the assertion the          old 'newest pending' rule could not satisfy"
    );

    // The live read then resolves with its own, quite different, numbers.
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: live,
        results: vec![live_snapshot()],
    });
    assert_eq!(measured(&mut app, live).source, MeasureSource::Live);
}

/// The failure path correlates the same way, in the same direction: the
/// OLDER render job fails while the newer live job is open, and only the
/// render job fails. Under "newest pending" this error would have killed
/// the live job instead.
#[test]
fn an_error_fails_only_the_job_it_names() {
    let mut app = app();

    let render = started_job(roundtrip(&mut app, request(1, "meter.measure", json!({}))));
    let live = started_job(roundtrip(
        &mut app,
        request(2, "meter.measure", json!({ "source": "live" })),
    ));
    assert!(live > render, "the live job is the newer of the two");

    app.test_apply_engine_event(AudioEvent::MixMeasureError {
        measure_id: render,
        message: "Stop transport before measuring the mix".to_owned(),
    });
    let status = job_status(&mut app, render);
    assert_eq!(status.state, JobState::Error);
    assert_eq!(
        status.error.as_deref(),
        Some("Stop transport before measuring the mix")
    );
    assert_eq!(
        job_status(&mut app, live).state,
        JobState::Pending,
        "the newer live job survives the older job's failure"
    );

    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: live,
        results: vec![live_snapshot()],
    });
    assert_eq!(measured(&mut app, live).source, MeasureSource::Live);
}

/// An event naming a job that is already finished resolves NOTHING. This
/// is what makes a wall-clock timeout safe to add later (ba todo #1243):
/// under "newest pending measure job" a late event would have completed
/// whichever job was open next — one caller receiving another's numbers,
/// the defect the #1219 review caught.
#[test]
fn a_late_event_for_a_finished_job_cannot_complete_a_later_one() {
    let mut app = app();
    let rate = app.sample_rate;

    let first = started_job(roundtrip(&mut app, request(1, "meter.measure", json!({}))));
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: first,
        results: vec![rendered(StemSource::Master, rate)],
    });
    let _ = measured(&mut app, first);

    // A second measurement is now accepted...
    let second = started_job(roundtrip(&mut app, request(2, "meter.measure", json!({}))));
    assert_eq!(job_status(&mut app, second).state, JobState::Pending);

    // ...and a duplicate/late event for the FIRST job must not touch it.
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: first,
        results: vec![rendered(StemSource::Track(DRUMS), rate)],
    });
    assert_eq!(
        job_status(&mut app, second).state,
        JobState::Pending,
        "the open job is still waiting for its own result"
    );

    // An id nobody ever issued is inert too.
    app.test_apply_engine_event(AudioEvent::MixMeasureError {
        measure_id: 9_999_999,
        message: "from nowhere".to_owned(),
    });
    assert_eq!(job_status(&mut app, second).state, JobState::Pending);

    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: second,
        results: vec![rendered(StemSource::Master, rate)],
    });
    assert_eq!(measured(&mut app, second).source, MeasureSource::Render);
}

/// A render measurement is still refused while another RENDER
/// measurement is open — that one is a genuine conflict over the single
/// offline renderer, not a correlation artefact, and the engine's
/// `OfflineRenderGuard` would refuse it anyway. A synchronous `busy`
/// beats a job id that is only going to fail.
#[test]
fn a_render_measure_while_a_live_measure_is_pending_is_accepted() {
    let mut app = app();
    let live = started_job(roundtrip(
        &mut app,
        request(1, "meter.measure", json!({ "source": "live" })),
    ));
    assert_eq!(job_status(&mut app, live).state, JobState::Pending);

    let render = started_job(roundtrip(&mut app, request(2, "meter.measure", json!({}))));
    assert_eq!(
        job_status(&mut app, render).state,
        JobState::Pending,
        "a pending live read does not hold the offline renderer"
    );
}

// ---------------------------------------------------------------------------
// meter.stems (ba todo #1220)
// ---------------------------------------------------------------------------

/// The measurements the engine would answer a `meter.stems` pass with:
/// one per requested target, all over the same range.
fn stems_results(targets: &[StemSource], rate: u32) -> Vec<MixMeasurement> {
    targets.iter().map(|t| rendered(*t, rate)).collect()
}

fn stems_result(app: &mut Resonance, job: u64) -> StemsResult {
    let status = job_status(app, job);
    assert_eq!(status.state, JobState::Done, "job errored: {:?}", status.error);
    serde_json::from_value(status.result.expect("done carries a result"))
        .expect("result is a StemsResult")
}

/// The core of the todo: master plus ONE entry per top-level track, and
/// the drum kit's sub-track folded into its parent rather than competing
/// with it. The sub-track must not be a target either — the engine would
/// render it silent, since a sub-track's audio only exists while its
/// parent's instrument runs.
#[test]
fn stems_reports_top_level_tracks_once_with_sub_tracks_folded_in() {
    let mut app = app();
    let rate = app.sample_rate;
    let job = started_job(roundtrip(&mut app, request(1, "meter.stems", json!({}))));

    // Exactly what one engine pass would answer for master + the two
    // TOP-LEVEL tracks. KICK is deliberately absent: it is a sub-track.
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: stems_results(
            &[
                StemSource::Master,
                StemSource::Track(DRUMS),
                StemSource::Track(BASS),
            ],
            rate,
        ),
    });

    let result = stems_result(&mut app, job);
    assert_eq!(result.master.target, MeasureTarget::Master);

    let ids: Vec<u64> = result.tracks.iter().map(|t| t.track_id.0).collect();
    assert_eq!(ids, vec![DRUMS, BASS], "one entry per top-level track");
    assert!(
        !ids.contains(&KICK),
        "the sub-track must not compete with its parent: {ids:?}"
    );

    let drums = &result.tracks[0];
    assert_eq!(
        drums.includes_track_ids.iter().map(|i| i.0).collect::<Vec<_>>(),
        vec![KICK],
        "the parent must say which sub-tracks its numbers already contain"
    );
    assert!(result.tracks[1].includes_track_ids.is_empty());

    // Every entry came from the same pass over the same range.
    let seconds = result.master.measured_seconds;
    assert_eq!(seconds, Some(10.0));
    for track in &result.tracks {
        assert_eq!(
            track.measurement.measured_seconds, seconds,
            "{} was measured over a different range",
            track.name
        );
    }
}

/// Ids and names line up with what `song.summary` reports, so a client
/// can join the two without a lookup table.
#[test]
fn stems_entries_match_song_summary() {
    use resonance_control::methods::song::SongSummary;

    let mut app = app();
    let rate = app.sample_rate;
    let job = started_job(roundtrip(&mut app, request(1, "meter.stems", json!({}))));
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: stems_results(
            &[
                StemSource::Master,
                StemSource::Track(DRUMS),
                StemSource::Track(BASS),
            ],
            rate,
        ),
    });
    let result = stems_result(&mut app, job);

    let summary: SongSummary = roundtrip(&mut app, Request::without_params(2, "song.summary"))
        .result()
        .expect("song.summary succeeds");
    for entry in &result.tracks {
        let line = summary
            .tracks
            .iter()
            .find(|t| t.id == entry.track_id)
            .unwrap_or_else(|| panic!("track {} is not in song.summary", entry.track_id));
        assert_eq!(entry.name, line.name);
    }
}

/// The wire shape: the measurement's fields sit alongside `track_id` and
/// `name` on one flat object, and `includes_track_ids` is omitted rather
/// than emitted as an empty array on the common case.
#[test]
fn a_stem_entry_is_one_flat_object_on_the_wire() {
    let mut app = app();
    let rate = app.sample_rate;
    let job = started_job(roundtrip(&mut app, request(1, "meter.stems", json!({}))));
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: stems_results(
            &[
                StemSource::Master,
                StemSource::Track(DRUMS),
                StemSource::Track(BASS),
            ],
            rate,
        ),
    });

    let wire = measured_json(&mut app, job);
    let drums = &wire["tracks"][0];
    assert_eq!(drums["track_id"], json!(DRUMS));
    assert_eq!(drums["lufs_integrated"], json!(-20.5));
    assert_eq!(drums["target"], json!({ "track_id": DRUMS }));
    assert_eq!(drums["includes_track_ids"], json!([KICK]));
    assert!(
        wire["tracks"][1].get("includes_track_ids").is_none(),
        "an ordinary track must not carry an empty array: {}",
        wire["tracks"][1]
    );
}

#[test]
fn busses_are_opt_in() {
    let mut app = app();
    let rate = app.sample_rate;

    // Default: no bus entry, and the bus is not even a target.
    let job = started_job(roundtrip(&mut app, request(1, "meter.stems", json!({}))));
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: stems_results(
            &[
                StemSource::Master,
                StemSource::Track(DRUMS),
                StemSource::Track(BASS),
            ],
            rate,
        ),
    });
    let without = stems_result(&mut app, job);
    assert!(without.tracks.iter().all(|t| t.track_id.0 != BUS));

    let job = started_job(roundtrip(
        &mut app,
        request(2, "meter.stems", json!({ "include_busses": true })),
    ));
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: stems_results(
            &[
                StemSource::Master,
                StemSource::Track(DRUMS),
                StemSource::Track(BASS),
                StemSource::Bus(BUS),
            ],
            rate,
        ),
    });
    let with = stems_result(&mut app, job);
    let bus = with
        .tracks
        .iter()
        .find(|t| t.track_id.0 == BUS)
        .expect("bus entry");
    assert_eq!(bus.name, "Drum Bus");
    assert_eq!(bus.measurement.target, MeasureTarget::Bus(BUS.into()));
}

#[test]
fn stems_accepts_an_explicit_range_and_no_range_alike() {
    let mut app = app();
    let rate = app.sample_rate;
    app.test_push_clip(four_second_clip(rate));

    for (id, params) in [
        (1, json!({})),
        (
            2,
            json!({ "range": { "start": { "bar": 1 }, "end": { "sample": u64::from(rate) * 2 } } }),
        ),
    ] {
        let job = started_job(roundtrip(&mut app, request(id, "meter.stems", params)));
        assert_eq!(job_status(&mut app, job).state, JobState::Pending);
        app.test_apply_engine_event(AudioEvent::MixMeasured {
            measure_id: job,
            results: stems_results(&[StemSource::Master, StemSource::Track(DRUMS)], rate),
        });
        assert_eq!(stems_result(&mut app, job).tracks.len(), 1);
    }
}

/// A pass with no master measurement is not a partial answer to report —
/// it is a broken one.
#[test]
fn stems_without_a_master_result_fails_the_job() {
    let mut app = app();
    let rate = app.sample_rate;
    let job = started_job(roundtrip(&mut app, request(1, "meter.stems", json!({}))));
    app.test_apply_engine_event(AudioEvent::MixMeasured {
        measure_id: job,
        results: stems_results(&[StemSource::Track(DRUMS)], rate),
    });
    let status = job_status(&mut app, job);
    assert_eq!(status.state, JobState::Error);
}

#[test]
fn stems_while_a_render_is_in_flight_is_busy() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut app = app();
    let target = dir.path().join("mix.wav").display().to_string();
    let _ = roundtrip(&mut app, request(1, "render.mixdown", json!({ "path": target })));

    let response = roundtrip(&mut app, request(2, "meter.stems", json!({})));
    assert_eq!(response.error.expect("render guard").kind(), ErrorKind::Busy);
}

#[test]
fn hello_lists_meter_stems() {
    let mut app = app();
    let hello: HelloResult = roundtrip(
        &mut app,
        request(1, "control.hello", json!({ "protocol_version": 1 })),
    )
    .result()
    .expect("hello succeeds");
    assert!(
        hello.capabilities.iter().any(|m| m == "meter.stems"),
        "meter.stems missing from capabilities: {:?}",
        hello.capabilities
    );
}
