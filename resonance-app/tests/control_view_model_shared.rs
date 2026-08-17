//! The control layer's shared wire projection (ba todo #1256).
//!
//! `update/control/view_model/` turns app state into wire types for the
//! WHOLE control layer, not just `song.*`: `transport.*` reports the
//! same playhead, `clip.*` stamps the same positions, `track.*` names
//! plugins with the same chain addressing, `meter.*` clamps to the same
//! song end. While those projections lived inside `song.rs` as
//! `pub(super)` helpers, nothing pinned that agreement — a handler could
//! quietly grow its own copy and two methods would answer one question
//! two ways.
//!
//! These tests assert the agreement across namespaces, so the projection
//! stays one implementation wherever it is called from.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::{MidiClipState, PluginSlotState, ViewMode};
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::{MidiNote, TrackType};
use resonance_control::methods::song::{SongSummary, TracksView};
use resonance_control::methods::track::PluginParamsView;
use resonance_control::methods::transport::TransportResult;
use resonance_control::{Request, Response, TrackKind, TransportState};

const SR: u32 = 48_000;
const INSTRUMENT: u64 = 1;
const AUDIO: u64 = 2;
const MIDI_CLIP: u64 = 100;
const AUDIO_CLIP: u64 = 200;

/// One instrument track (synth + EQ, so the chain has an instrument and
/// an effect) with a 1-bar MIDI clip, plus an audio track whose clip
/// starts at bar 3 and ends the song.
fn app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_sample_rate(SR);
    app.test_rebuild_tempo_map();
    app.test_set_active_project(true);

    app.test_add_track(INSTRUMENT, TrackType::Instrument);
    app.test_add_track(AUDIO, TrackType::Audio);
    app.test_push_track_plugin(
        INSTRUMENT,
        PluginSlotState::new(
            900,
            "Test Synth".to_owned(),
            "com.test.synth".to_owned(),
            "/plugins/test.clap".to_owned(),
            Vec::new(),
            false,
        ),
    );
    app.test_push_track_plugin(
        INSTRUMENT,
        PluginSlotState::new(
            901,
            "Test EQ".to_owned(),
            "com.test.eq".to_owned(),
            "/plugins/eq.clap".to_owned(),
            Vec::new(),
            false,
        ),
    );

    app.test_push_midi_clip(MidiClipState {
        id: MIDI_CLIP,
        track_id: INSTRUMENT,
        start_sample: 0,
        duration_ticks: 4 * 480,
        name: "riff".to_owned(),
        notes: vec![MidiNote {
            note: 60,
            velocity: 0.8,
            start_tick: 0,
            duration_ticks: 480,
        }],
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    // 120 bpm 4/4: one bar is 2 s, so this clip runs 4 s -> 6 s.
    app.test_push_clip(resonance_app::state::ClipState {
        id: AUDIO_CLIP,
        track_id: AUDIO,
        start_sample: 4 * SR as u64,
        duration_samples: 2 * SR as u64,
        name: "take".to_owned(),
        total_frames: 2 * SR as u64,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: Default::default(),
        fade_out_frames: 0,
        fade_out_curve: Default::default(),
        gain_db: 0.0,
        waveform_peaks: Vec::new(),
        vocal_tuning: None,
        asset_ref: None,
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

fn call(app: &mut Resonance, method: &str, params: serde_json::Value) -> Response {
    roundtrip(
        app,
        Request::new(1, method, &params).expect("params serialize"),
    )
}

fn call_no_params(app: &mut Resonance, method: &str) -> Response {
    roundtrip(app, Request::without_params(1, method))
}

fn summary(app: &mut Resonance) -> SongSummary {
    call_no_params(app, "song.summary")
        .result()
        .expect("song.summary succeeds")
}

// ---------------- position + transport state ----------------

#[test]
fn transport_and_song_summary_report_the_same_playhead() {
    let mut app = app();

    let seek: TransportResult = call(
        &mut app,
        "transport.seek",
        serde_json::json!({ "sample": 3 * SR as u64 }),
    )
    .result()
    .expect("seek succeeds");

    // `song.summary` projects the playhead through the same helper, so
    // bar, fractional beat and sample must match field for field.
    let summary = summary(&mut app);
    assert_eq!(summary.playhead.sample, seek.playhead.sample);
    assert_eq!(summary.playhead.bar, seek.playhead.bar);
    assert_eq!(summary.playhead.beat, seek.playhead.beat);
    // 120 bpm 4/4: 3 s in is bar 2, beat 3.
    assert_eq!(seek.playhead.bar, 2);
    assert_eq!(seek.playhead.beat, 3.0);
}

#[test]
fn transport_and_song_summary_report_the_same_state() {
    let mut app = app();
    assert_eq!(summary(&mut app).transport, TransportState::Stopped);

    let play: TransportResult = call_no_params(&mut app, "transport.play")
        .result()
        .expect("play succeeds");
    assert_eq!(play.state, TransportState::Playing);
    assert_eq!(summary(&mut app).transport, TransportState::Playing);

    let stop: TransportResult = call_no_params(&mut app, "transport.stop")
        .result()
        .expect("stop succeeds");
    assert_eq!(stop.state, TransportState::Stopped);
    assert_eq!(summary(&mut app).transport, TransportState::Stopped);
}

#[test]
fn song_length_is_the_furthest_clip_end_over_both_clip_kinds() {
    let mut app = app();
    // The audio clip (4 s -> 6 s) outlasts the 2 s MIDI clip, and the
    // length is reported in both samples and bars off the same number.
    let summary = summary(&mut app);
    assert_eq!(summary.length_samples, 6 * SR as u64);
    assert!(
        (summary.length_bars - 3.0).abs() < 1e-6,
        "6 s at 120 bpm 4/4 is 3 bars, got {}",
        summary.length_bars
    );
}

// ---------------- clip projection ----------------

#[test]
fn song_tracks_projects_both_clip_kinds_in_start_order() {
    let mut app = app();
    let view: TracksView = call_no_params(&mut app, "song.tracks")
        .result()
        .expect("song.tracks succeeds");

    let instrument = view
        .tracks
        .iter()
        .find(|t| t.summary.id.0 == INSTRUMENT)
        .expect("instrument track present");
    assert_eq!(instrument.clips.len(), 1);
    assert!(instrument.clips[0].midi, "MIDI clip flagged as midi");
    assert_eq!(instrument.clips[0].start.sample, 0);
    assert_eq!(instrument.clips[0].length_beats, 4.0);
    // The count in the summary and the projected list are one fact.
    assert_eq!(instrument.summary.clip_count, instrument.clips.len());

    let audio = view
        .tracks
        .iter()
        .find(|t| t.summary.id.0 == AUDIO)
        .expect("audio track present");
    assert_eq!(audio.clips.len(), 1);
    assert!(!audio.clips[0].midi, "audio clip not flagged as midi");
    // Clip positions come through the same projection as the playhead.
    assert_eq!(audio.clips[0].start.sample, 4 * SR as u64);
    assert_eq!(audio.clips[0].start.bar, 3);
    assert_eq!(audio.summary.clip_count, audio.clips.len());
    assert_eq!(audio.summary.kind, TrackKind::Audio);
}

// ---------------- plugin chain addressing ----------------

#[test]
fn plugin_params_and_song_tracks_agree_on_the_chain() {
    let mut app = app();

    let tracks: TracksView = call_no_params(&mut app, "song.tracks")
        .result()
        .expect("song.tracks succeeds");
    let instrument = tracks
        .tracks
        .iter()
        .find(|t| t.summary.id.0 == INSTRUMENT)
        .expect("instrument track present");
    // Slot 0 is the instrument, so only the EQ is an effect.
    assert_eq!(instrument.summary.instrument.as_deref(), Some("com.test.synth"));
    assert_eq!(instrument.effects, vec!["com.test.eq".to_owned()]);

    let params: PluginParamsView = call(
        &mut app,
        "track.plugin_params",
        serde_json::json!({ "track_id": INSTRUMENT }),
    )
    .result()
    .expect("track.plugin_params succeeds");
    // Same chain, same order, same instrument/effect split — the
    // per-plugin view is the entry list `song.tracks` summarizes.
    let ids: Vec<&str> = params.plugins.iter().map(|p| p.plugin_id.as_str()).collect();
    assert_eq!(ids, vec!["com.test.synth", "com.test.eq"]);
    assert_eq!(params.plugins[0].slot, 0);
    assert_eq!(params.plugins[1].slot, 1);
    let effects: Vec<&str> = params
        .plugins
        .iter()
        .filter(|p| p.kind == resonance_control::methods::track::PluginKind::Effect)
        .map(|p| p.plugin_id.as_str())
        .collect();
    assert_eq!(effects, instrument.effects);
}

#[test]
fn an_unknown_plugin_reads_the_same_in_every_namespace_that_addresses_one() {
    let mut app = app();

    let read = call(
        &mut app,
        "track.plugin_params",
        serde_json::json!({ "track_id": INSTRUMENT, "plugin_id": "com.example.ghost" }),
    )
    .error
    .expect("unknown plugin rejected");
    let write = call(
        &mut app,
        "track.remove_effect",
        serde_json::json!({ "track_id": INSTRUMENT, "plugin_id": "com.example.ghost" }),
    )
    .error
    .expect("unknown plugin rejected");

    // Read and write both build the message from the projected chain, so
    // a client is told the same thing whichever it asked.
    assert_eq!(read.message, write.message);
    assert!(
        read.message.contains("com.test.synth") && read.message.contains("com.test.eq"),
        "the error lists what the track does carry: {}",
        read.message
    );
}
