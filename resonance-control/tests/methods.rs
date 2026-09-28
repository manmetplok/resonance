//! Method params/results: capability list, defaults, destructive-op
//! confirm flags, jobs, and per-namespace round-trips.

use resonance_control::common::{MutationAck, PositionSpec};
use resonance_control::ids::{ClipId, JobId, SectionDefinitionId, TrackId};
use resonance_control::job::{JobStarted, JobState, JobStatus, WaitParams};
use resonance_control::methods::{
    self, control, generate, global, harmony, mixer, notes, project, render, section, track,
    transport, vocal,
};
use resonance_control::PROTOCOL_VERSION;
use serde_json::json;
use std::collections::HashSet;

#[test]
fn capabilities_cover_all_namespaces_without_duplicates() {
    let caps = methods::capabilities();
    let unique: HashSet<_> = caps.iter().copied().collect();
    assert_eq!(unique.len(), caps.len(), "duplicate method names");
    for name in &caps {
        let (namespace, method) = name.split_once('.').expect("namespaced name");
        assert!(!namespace.is_empty() && !method.is_empty());
    }
    for expected in [
        "control.hello",
        "song.summary",
        "song.notes",
        "project.open",
        "transport.set_tempo",
        "track.add",
        "mixer.set_volume",
        "section.place",
        "harmony.apply_progression",
        "generate.part",
        "notes.insert",
        "vocal.render",
        "render.mixdown",
        "job.wait",
    ] {
        assert!(caps.contains(&expected), "missing {expected}");
    }
}

#[test]
fn hello_roundtrips_and_reports_capabilities() {
    assert_eq!(PROTOCOL_VERSION, 1);
    let result = control::HelloResult {
        app_version: "0.1.0".to_owned(),
        protocol_version: PROTOCOL_VERSION,
        capabilities: methods::capabilities()
            .into_iter()
            .map(str::to_owned)
            .collect(),
    };
    let wire = serde_json::to_value(&result).unwrap();
    assert_eq!(wire["protocol_version"], json!(1));
    let back: control::HelloResult = serde_json::from_value(wire).unwrap();
    assert_eq!(back, result);
}

#[test]
fn destructive_params_default_to_unconfirmed() {
    let open: project::OpenParams =
        serde_json::from_value(json!({"path": "/tmp/song.rproj"})).unwrap();
    assert!(!open.confirm);

    let delete: track::DeleteParams = serde_json::from_value(json!({"track_id": 3})).unwrap();
    assert!(!delete.confirm);
    assert_eq!(delete.track_id, TrackId(3));

    let mixdown: render::MixdownParams =
        serde_json::from_value(json!({"path": "/tmp/mix.wav"})).unwrap();
    assert!(!mixdown.overwrite);
    assert!(mixdown.range.is_none());
}

#[test]
fn seek_params_flatten_the_position() {
    let musical: transport::SeekParams =
        serde_json::from_value(json!({"bar": 5, "beat": 2.0})).unwrap();
    assert_eq!(musical.position, PositionSpec::musical(5, 2.0));
    assert!(!musical.position.is_empty());

    let by_sample: transport::SeekParams =
        serde_json::from_value(json!({"sample": 96000})).unwrap();
    assert_eq!(by_sample.position, PositionSpec::sample(96000));

    let empty: transport::SeekParams = serde_json::from_value(json!({})).unwrap();
    assert!(empty.position.is_empty());

    let wire = serde_json::to_value(transport::SeekParams {
        position: PositionSpec::musical(5, 2.0),
    })
    .unwrap();
    assert_eq!(wire, json!({"bar": 5, "beat": 2.0}));
}

#[test]
fn note_insert_defaults_velocity() {
    let params: notes::InsertParams = serde_json::from_value(json!({
        "clip_id": 7, "pitch": 64, "start_beat": 2.0, "duration_beats": 0.5
    }))
    .unwrap();
    assert_eq!(params.velocity, 100);
    assert_eq!(params.clip_id, ClipId(7));
}

#[test]
fn job_lifecycle_types_roundtrip() {
    let started = JobStarted { job_id: JobId(7) };
    assert_eq!(serde_json::to_value(started).unwrap(), json!({"job_id": 7}));

    let status = JobStatus {
        job_id: JobId(7),
        state: JobState::Running,
        progress: Some(0.5),
        result: None,
        error: None,
    };
    let wire = serde_json::to_value(&status).unwrap();
    assert_eq!(
        wire,
        json!({"job_id": 7, "state": "running", "progress": 0.5})
    );
    let back: JobStatus = serde_json::from_value(wire).unwrap();
    assert_eq!(back, status);

    assert!(!JobState::Pending.is_terminal());
    assert!(!JobState::Running.is_terminal());
    assert!(JobState::Done.is_terminal());
    assert!(JobState::Error.is_terminal());

    let wait: WaitParams = serde_json::from_value(json!({"job_id": 7})).unwrap();
    assert_eq!(wait.timeout_ms, None);
}

/// ARCH-05 / epic C, C-2: `JobStatus.error` gained an optional `kind`
/// alongside the message it always carried. The new shape round-trips;
/// an old-shape status — `error` as a bare string, the pre-C-2 wire
/// format — must still deserialize under the current type so a job
/// failure recorded (or replayed) before this change doesn't break.
#[test]
fn job_error_kind_roundtrips_and_the_old_bare_string_shape_still_deserializes() {
    use resonance_control::job::JobError;
    use resonance_control::ErrorKind;

    let status = JobStatus {
        job_id: JobId(9),
        state: JobState::Error,
        progress: None,
        result: None,
        error: Some(JobError::new("no such track", Some(ErrorKind::NotFound))),
    };
    let wire = serde_json::to_value(&status).unwrap();
    assert_eq!(
        wire["error"],
        json!({"message": "no such track", "kind": "not_found"})
    );
    let back: JobStatus = serde_json::from_value(wire).unwrap();
    assert_eq!(back, status);

    // A kind-less failure serializes without the field at all, not `null`.
    let untyped = JobStatus {
        job_id: JobId(9),
        state: JobState::Error,
        progress: None,
        result: None,
        error: Some(JobError::new("mixdown did not start", None)),
    };
    let wire = serde_json::to_value(&untyped).unwrap();
    assert_eq!(wire["error"], json!({"message": "mixdown did not start"}));
    let back: JobStatus = serde_json::from_value(wire).unwrap();
    assert_eq!(back, untyped);

    // The pre-C-2 wire shape: `error` was a bare string.
    let legacy = json!({"job_id": 9, "state": "error", "error": "disk full"});
    let back: JobStatus = serde_json::from_value(legacy).unwrap();
    assert_eq!(back.error, Some(JobError::from("disk full".to_owned())));
}

#[test]
fn done_job_carries_the_method_result_payload() {
    let mixdown = render::MixdownResult {
        path: "/tmp/mix.wav".to_owned(),
        duration_s: 42.5,
        sample_rate: 48_000,
        // Nothing soloed: the field is skipped on the wire entirely, so
        // an existing client sees the shape it always saw.
        soloed_track_ids: Vec::new(),
    };
    let status = JobStatus {
        job_id: JobId(3),
        state: JobState::Done,
        progress: Some(1.0),
        result: Some(serde_json::to_value(&mixdown).unwrap()),
        error: None,
    };
    let wire = serde_json::to_value(&status).unwrap();
    assert_eq!(
        wire["result"],
        json!({"path": "/tmp/mix.wav", "duration_s": 42.5, "sample_rate": 48_000})
    );
    let back: render::MixdownResult = serde_json::from_value(wire["result"].clone()).unwrap();
    assert_eq!(back, mixdown);
}

#[test]
fn mutation_ack_is_just_the_revision() {
    let ack = MutationAck { revision: 41 };
    assert_eq!(serde_json::to_value(ack).unwrap(), json!({"revision": 41}));
}

#[test]
fn harmony_progression_accepts_symbols_or_theory_requests() {
    let by_symbols: harmony::ApplyProgressionParams = serde_json::from_value(json!({
        "section_id": 1,
        "symbols": ["Am7", "Dm7", "G7", "Cmaj7"],
        "beats_per_chord": 4.0
    }))
    .unwrap();
    assert_eq!(by_symbols.symbols.as_deref().unwrap().len(), 4);
    assert!(by_symbols.numerals.is_none());

    let by_theory: harmony::ApplyProgressionParams = serde_json::from_value(json!({
        "section_id": 1,
        "key": {"tonic": "A", "scale": "minor"},
        "numerals": ["i", "VI", "III", "VII"],
        "sevenths": true
    }))
    .unwrap();
    assert_eq!(by_theory.key.as_ref().unwrap().tonic, "A");
    assert_eq!(by_theory.sevenths, Some(true));

    let minimal = harmony::ApplyProgressionParams::for_section(SectionDefinitionId(1));
    assert_eq!(
        serde_json::to_value(&minimal).unwrap(),
        json!({"section_id": 1})
    );
}

#[test]
fn enum_params_use_lowercase_strings() {
    let add: track::AddParams =
        serde_json::from_value(json!({"kind": "vocal", "name": "Lead Vox"})).unwrap();
    assert_eq!(
        serde_json::to_value(&add).unwrap(),
        json!({"kind": "vocal", "name": "Lead Vox"})
    );

    let part: generate::PartParams = serde_json::from_value(json!({
        "section_id": 1, "track_id": 2, "role": "bass", "seed": 7
    }))
    .unwrap();
    assert_eq!(part.role, generate::GenerateRole::Bass);
    assert_eq!(serde_json::to_value(part.role).unwrap(), json!("bass"));

    let kind: track::PluginKind = serde_json::from_value(json!("effect")).unwrap();
    assert_eq!(kind, track::PluginKind::Effect);
}

#[test]
fn set_lane_generator_params_roundtrip() {
    assert!(section::METHODS.contains(&section::SET_LANE_GENERATOR));
    assert!(methods::capabilities().contains(&"section.set_lane_generator"));

    // Minimal: kind only, options/seed omitted.
    let minimal: section::SetLaneGeneratorParams = serde_json::from_value(json!({
        "section_id": 3, "track_id": 5, "kind": "vocal"
    }))
    .unwrap();
    assert_eq!(minimal.kind, section::LaneKind::Vocal);
    assert!(minimal.seed.is_none() && minimal.options.is_none());
    assert_eq!(
        serde_json::to_value(&minimal).unwrap(),
        json!({"section_id": 3, "track_id": 5, "kind": "vocal"})
    );

    // Full: snake_case kind, seed, and a per-kind options passthrough.
    let full: section::SetLaneGeneratorParams = serde_json::from_value(json!({
        "section_id": 1, "track_id": 2, "kind": "bass", "seed": 9,
        "options": {"octave": 2}
    }))
    .unwrap();
    assert_eq!(full.kind, section::LaneKind::Bass);
    assert_eq!(full.seed, Some(9));
    assert_eq!(full.options.as_ref().unwrap()["octave"], json!(2));

    for (kind, wire) in [
        (section::LaneKind::Manual, "manual"),
        (section::LaneKind::Bass, "bass"),
        (section::LaneKind::Melody, "melody"),
        (section::LaneKind::Pad, "pad"),
        (section::LaneKind::Vocal, "vocal"),
    ] {
        assert_eq!(serde_json::to_value(kind).unwrap(), json!(wire));
    }

    let result = section::SetLaneGeneratorResult { revision: 7 };
    assert_eq!(serde_json::to_value(result).unwrap(), json!({"revision": 7}));
}

#[test]
fn mixer_and_vocal_params_roundtrip() {
    let volume = mixer::SetVolumeParams {
        track_id: TrackId(2),
        volume: 0.75,
    };
    assert_eq!(
        serde_json::to_value(volume).unwrap(),
        json!({"track_id": 2, "volume": 0.75})
    );

    let lyrics: vocal::SetLyricsParams = serde_json::from_value(json!({
        "track_id": 9, "text": "line one\nline two"
    }))
    .unwrap();
    assert_eq!(lyrics.track_id, TrackId(9));

    let render_all: vocal::RenderParams = serde_json::from_value(json!({})).unwrap();
    assert_eq!(render_all, vocal::RenderParams::default());
    assert!(render_all.voicebank.is_none());
}

#[test]
fn global_add_params_roundtrip_with_1_based_bars() {
    // `bar` is the address, not an index, and it is 1-based like every
    // other bar on this surface (ba doc #286 §2).
    let tempo = global::AddTempoEventParams {
        bar: 33,
        bpm: 140.0,
    };
    assert_eq!(
        serde_json::to_value(tempo).unwrap(),
        json!({"bar": 33, "bpm": 140.0})
    );
    let back: global::AddTempoEventParams =
        serde_json::from_value(json!({"bar": 33, "bpm": 140.0})).unwrap();
    assert_eq!(back, tempo);

    // The denominator is RESOLVED (8 for 7/8), never an exponent.
    let meter = global::AddSignatureEventParams {
        bar: 17,
        numerator: 7,
        denominator: 8,
    };
    assert_eq!(
        serde_json::to_value(meter).unwrap(),
        json!({"bar": 17, "numerator": 7, "denominator": 8})
    );
    let back: global::AddSignatureEventParams =
        serde_json::from_value(json!({"bar": 17, "numerator": 7, "denominator": 8})).unwrap();
    assert_eq!(back, meter);

    // Both adds are advertised, so a client can tell from the handshake
    // whether this build can write the tempo map.
    let caps = methods::capabilities();
    assert!(caps.contains(&global::ADD_TEMPO_EVENT));
    assert!(caps.contains(&global::ADD_SIGNATURE_EVENT));
}

#[test]
fn global_remove_params_are_a_bar_and_nothing_else() {
    // The removes address an event the way every other `global.*` method
    // does — by the 1-based bar it sits on, never by its position in the
    // list, which re-sorts on every mutation (ba doc #286 §2).
    let tempo = global::RemoveTempoEventParams { bar: 33 };
    assert_eq!(serde_json::to_value(tempo).unwrap(), json!({"bar": 33}));
    let back: global::RemoveTempoEventParams = serde_json::from_value(json!({"bar": 33})).unwrap();
    assert_eq!(back, tempo);

    let meter = global::RemoveSignatureEventParams { bar: 17 };
    assert_eq!(serde_json::to_value(meter).unwrap(), json!({"bar": 17}));
    let back: global::RemoveSignatureEventParams =
        serde_json::from_value(json!({"bar": 17})).unwrap();
    assert_eq!(back, meter);

    let caps = methods::capabilities();
    assert!(caps.contains(&global::REMOVE_TEMPO_EVENT));
    assert!(caps.contains(&global::REMOVE_SIGNATURE_EVENT));
}

/// `meter.*` `detail` (warmth-width-depth.md §7.1): absent on the wire
/// when empty, so a default request serializes exactly as before, and
/// named in lowercase.
#[test]
fn meter_detail_is_omitted_when_empty_and_lowercase_when_set() {
    use resonance_control::methods::meter::{MeasureDetail, MeasureParams, StemsParams};

    assert_eq!(serde_json::to_value(MeasureParams::default()).unwrap(), json!({
        "target": "master",
        "source": "render"
    }));
    assert_eq!(
        serde_json::to_value(StemsParams::default()).unwrap(),
        json!({ "include_busses": false })
    );

    let params: MeasureParams =
        serde_json::from_value(json!({ "detail": ["spectrum"] })).unwrap();
    assert_eq!(params.detail, vec![MeasureDetail::Spectrum]);
    assert_eq!(serde_json::to_value(&params).unwrap()["detail"], json!(["spectrum"]));

    let stems: StemsParams = serde_json::from_value(json!({ "detail": ["spectrum"] })).unwrap();
    assert_eq!(stems.detail, vec![MeasureDetail::Spectrum]);

    assert!(serde_json::from_value::<MeasureParams>(json!({ "detail": ["warmth"] })).is_err());
}
