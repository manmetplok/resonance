//! `transport.*` control methods through the real update path (ba doc
//! #265, todo #1150): play/stop/pause/seek/loop, tempo, time signature,
//! and the global key — asserting transport state, engine commands,
//! reply echoes, and the undo/revision contract (navigation transient,
//! musical edits undoable; a GUI Cmd-Z reverts a remote tempo change).

use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance};
use resonance_audio::types::AudioCommand;
use resonance_control::methods::transport::TransportResult;
use resonance_control::{ErrorKind, Request, Response, TransportState};
use crate::common::{call, roundtrip};

const SR: u32 = 48_000;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_sample_rate(SR);
    app.test_rebuild_tempo_map();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-transport-test.rprj"));
    app
}

fn call_no_params(app: &mut Resonance, method: &str) -> Response {
    roundtrip(app, Request::without_params(1, method))
}

fn drain(rx: &resonance_audio::test_support::Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

// ---------------- play / pause / stop (transient) ----------------

#[test]
fn play_pause_stop_drive_the_engine_without_undo_entries() {
    let mut app = app();
    let rx = app.test_capture_engine();
    let before = app.revision();

    let result: TransportResult = call_no_params(&mut app, "transport.play")
        .result()
        .expect("play succeeds");
    assert_eq!(result.state, TransportState::Playing);
    assert!(drain(&rx).iter().any(|c| matches!(c, AudioCommand::Play)));

    let result: TransportResult = call_no_params(&mut app, "transport.pause")
        .result()
        .expect("pause succeeds");
    assert_eq!(result.state, TransportState::Stopped);
    assert!(drain(&rx).iter().any(|c| matches!(c, AudioCommand::Pause)));

    let _ = call_no_params(&mut app, "transport.play");
    let result: TransportResult = call_no_params(&mut app, "transport.stop")
        .result()
        .expect("stop succeeds");
    assert_eq!(result.state, TransportState::Stopped);
    assert_eq!(result.playhead.sample, 0);

    // Navigation is transient: nothing entered the undo history.
    assert_eq!(app.revision(), before);
    assert!(!app.test_can_undo());
}

/// The engine refusing a Play (an offline render holds the transport)
/// resets the optimistic `playing` mirror without zeroing the playhead
/// (FU-F1a).
#[test]
fn a_refused_play_clears_the_playing_mirror_and_keeps_the_playhead() {
    let mut app = app();
    let _ = call(&mut app, "transport.seek", serde_json::json!({ "sample": 12_345 }));
    let _ = call_no_params(&mut app, "transport.play");
    assert!(app.test_transport_playing());

    app.test_apply_engine_event(resonance_audio::types::AudioEvent::TransportRefused);
    assert!(!app.test_transport_playing());
    assert_eq!(app.test_transport_playhead(), 12_345);
}

// ---------------- seek ----------------

#[test]
fn seek_resolves_musical_and_sample_positions() {
    let mut app = app();
    // Bar 2 at 120 bpm 4/4: one bar = 2 s = 96_000 samples.
    let result: TransportResult = call(
        &mut app,
        "transport.seek",
        serde_json::json!({ "bar": 2, "beat": 1.0 }),
    )
    .result()
    .expect("seek succeeds");
    assert_eq!(result.playhead.sample, 2 * SR as u64);
    assert_eq!(result.playhead.bar, 2);

    // Beat offsets land mid-bar: bar 1 beat 3 = 2 beats = 1 s.
    let result: TransportResult = call(
        &mut app,
        "transport.seek",
        serde_json::json!({ "bar": 1, "beat": 3.0 }),
    )
    .result()
    .expect("seek succeeds");
    assert_eq!(result.playhead.sample, SR as u64);

    let result: TransportResult = call(
        &mut app,
        "transport.seek",
        serde_json::json!({ "sample": 12_345 }),
    )
    .result()
    .expect("seek succeeds");
    assert_eq!(result.playhead.sample, 12_345);

    // Seeks are transient.
    assert!(!app.test_can_undo());
}

#[test]
fn seek_rejects_empty_ambiguous_and_zero_based_positions() {
    let mut app = app();
    for params in [
        serde_json::json!({}),
        serde_json::json!({ "bar": 1, "sample": 5 }),
        serde_json::json!({ "bar": 0 }),
        serde_json::json!({ "bar": 1, "beat": 0.5 }),
    ] {
        let response = call(&mut app, "transport.seek", params.clone());
        assert_eq!(
            response.error.unwrap_or_else(|| panic!("{params} rejected")).kind(),
            ErrorKind::InvalidParams,
            "params {params} must be invalid"
        );
    }
    assert_eq!(app.test_transport_playhead(), 0);
}

// ---------------- loop ----------------

#[test]
fn loop_set_validates_and_is_undoable() {
    let mut app = app();
    let before = app.revision();

    let result: TransportResult = call(
        &mut app,
        "transport.loop_set",
        serde_json::json!({
            "start": { "bar": 1 },
            "end": { "bar": 3 },
            "enabled": true
        }),
    )
    .result()
    .expect("loop_set succeeds");
    assert!(result.looping);
    assert_eq!(app.revision(), before + 1);

    let (loop_in, loop_out, enabled) = app.test_loop_range();
    assert_eq!(loop_in, 0);
    assert_eq!(loop_out, 4 * SR as u64); // two bars at 120 bpm 4/4
    assert!(enabled);

    // Reversed range: precise rejection, no mutation, no undo entry.
    let response = call(
        &mut app,
        "transport.loop_set",
        serde_json::json!({ "start": { "bar": 3 }, "end": { "bar": 1 } }),
    );
    let error = response.error.expect("reversed loop rejected");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert!(error.message.contains("before end"));
    assert_eq!(app.revision(), before + 1);

    let result: TransportResult = call_no_params(&mut app, "transport.loop_toggle")
        .result()
        .expect("loop_toggle succeeds");
    assert!(!result.looping);
    assert_eq!(app.revision(), before + 2);
}

// ---------------- tempo ----------------

#[test]
fn set_tempo_is_undoable_and_reverts_with_gui_undo() {
    let mut app = app();
    assert_eq!(app.test_transport_bpm(), 120.0);
    let before = app.revision();

    let result: TransportResult = call(
        &mut app,
        "transport.set_tempo",
        serde_json::json!({ "bpm": 150.0 }),
    )
    .result()
    .expect("set_tempo succeeds");
    assert_eq!(app.test_transport_bpm(), 150.0);
    // SetBpmText is transient; only the commit records — one revision.
    assert_eq!(result.revision, before + 1);
    assert!(app.test_can_undo());

    // The GUI's Cmd-Z reverts the remote edit (DoD).
    let _ = app.update(Message::Undo);
    assert_eq!(app.test_transport_bpm(), 120.0);
}

#[test]
fn set_tempo_out_of_range_is_rejected_precisely() {
    let mut app = app();
    let before = app.revision();
    for bpm in [10.0, 500.0] {
        let response = call(&mut app, "transport.set_tempo", serde_json::json!({ "bpm": bpm }));
        let error = response.error.expect("out-of-range bpm rejected");
        assert_eq!(error.kind(), ErrorKind::InvalidParams);
        assert!(error.message.contains("out of range"));
    }
    // NaN can't ride JSON (serde_json encodes it as null) — it dies in
    // param parsing with the same stable kind.
    let response = call(&mut app, "transport.set_tempo", serde_json::json!({ "bpm": f64::NAN }));
    assert_eq!(
        response.error.expect("NaN bpm rejected").kind(),
        ErrorKind::InvalidParams
    );
    assert_eq!(app.test_transport_bpm(), 120.0);
    assert_eq!(app.revision(), before);
    assert!(!app.test_can_undo());
}

// ---------------- time signature ----------------

#[test]
fn set_time_signature_directly_and_undoably() {
    let mut app = app();
    let before = app.revision();
    let result: TransportResult = call(
        &mut app,
        "transport.set_time_signature",
        serde_json::json!({ "numerator": 6, "denominator": 8 }),
    )
    .result()
    .expect("set_time_signature succeeds");
    assert_eq!(app.test_transport_time_sig(), (6, 8));
    assert_eq!(result.revision, before + 1);
    assert!(app.test_can_undo());

    for params in [
        serde_json::json!({ "numerator": 0, "denominator": 4 }),
        serde_json::json!({ "numerator": 33, "denominator": 4 }),
        serde_json::json!({ "numerator": 4, "denominator": 5 }),
    ] {
        let response = call(&mut app, "transport.set_time_signature", params);
        assert_eq!(
            response.error.expect("bad signature rejected").kind(),
            ErrorKind::InvalidParams
        );
    }
    assert_eq!(app.test_transport_time_sig(), (6, 8));
}

// ---------------- loop-record mode ----------------

#[test]
fn set_loop_record_mode_sends_the_engine_command_and_is_undoable() {
    let mut app = app();
    let rx = app.test_capture_engine();
    let before = app.revision();
    assert!(!app.test_loop_record_mode(), "distinct takes off by default");

    let result: TransportResult = call(
        &mut app,
        "transport.set_loop_record_mode",
        serde_json::json!({ "distinct_takes": true }),
    )
    .result()
    .expect("set_loop_record_mode succeeds");
    assert!(result.loop_record_mode);
    assert!(app.test_loop_record_mode());
    assert_eq!(result.revision, before + 1);
    assert!(app.test_can_undo());
    assert!(drain(&rx)
        .iter()
        .any(|c| matches!(c, AudioCommand::SetLoopRecordMode(true))));

    // The GUI's Cmd-Z reverts the remote edit, same as tempo/time sig.
    let _ = app.update(Message::Undo);
    assert!(!app.test_loop_record_mode());
}

#[test]
fn set_loop_record_mode_misspelled_field_is_rejected() {
    let mut app = app();
    let response = call(
        &mut app,
        "transport.set_loop_record_mode",
        serde_json::json!({ "distinct_take": true }),
    );
    assert_eq!(
        response.error.expect("unknown field rejected").kind(),
        ErrorKind::InvalidParams
    );
}

// ---------------- key ----------------

#[test]
fn set_key_sets_the_chord_track_song_key() {
    let mut app = app();
    let before = app.revision();
    let response = call(
        &mut app,
        "transport.set_key",
        serde_json::json!({ "tonic": "A", "scale": "minor" }),
    );
    assert!(response.result::<TransportResult>().is_ok());
    let key = app
        .test_chord_track()
        .key_changes
        .first()
        .expect("song key set");
    assert_eq!(key.scale.root, resonance_music_theory::PitchClass::A);
    assert_eq!(key.scale.mode, resonance_music_theory::Mode::Minor);
    assert_eq!(app.revision(), before + 1);

    // Flat spellings resolve to the enharmonic pitch class.
    let _ = call(
        &mut app,
        "transport.set_key",
        serde_json::json!({ "tonic": "Bb", "scale": "major" }),
    );
    let key = app.test_chord_track().key_changes.first().unwrap();
    assert_eq!(key.scale.root, resonance_music_theory::PitchClass::As);

    let response = call(
        &mut app,
        "transport.set_key",
        serde_json::json!({ "tonic": "H", "scale": "minor" }),
    );
    assert_eq!(
        response.error.expect("bad tonic rejected").kind(),
        ErrorKind::InvalidParams
    );
    let response = call(
        &mut app,
        "transport.set_key",
        serde_json::json!({ "tonic": "A", "scale": "phrygian dominant" }),
    );
    assert_eq!(
        response.error.expect("bad scale rejected").kind(),
        ErrorKind::InvalidParams
    );
}

// ---------------- reply echo ----------------

#[test]
fn every_reply_echoes_transport_state_and_revision() {
    let mut app = app();
    let result: TransportResult = call(
        &mut app,
        "transport.seek",
        serde_json::json!({ "bar": 2 }),
    )
    .result()
    .expect("seek succeeds");
    assert_eq!(result.state, TransportState::Stopped);
    assert_eq!(result.revision, app.revision());
    assert_eq!(result.playhead.bar, 2);
    assert!(!result.looping);
}
