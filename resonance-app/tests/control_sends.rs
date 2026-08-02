//! `track.add_send` / `set_send` / `remove_send` + `sends` in
//! `song.tracks` (ba doc #273, todo #1229).
//!
//! Sends matter beyond loudness: ONE reverb fed from several tracks is
//! what puts them in the same room. The whole send graph already existed
//! engine-first — `AudioCommand::SetAuxSend`, `MixerMessage::*`, the
//! `AuxSendState` mirror, and the cyclic-route predicate — and was
//! simply unreachable from the control API.
//!
//! The engine is the single writer, so these tests drive the
//! `AuxSendChanged` / `AuxSendRemoved` echoes the way the real engine
//! does.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::{AudioCommand, AudioEvent, SendSource, TrackType};
use resonance_control::methods::bus::CreateResult;
use resonance_control::methods::song::{SendView, TracksView};
use resonance_control::methods::track::AddSendResult;
use resonance_control::{ErrorKind, MutationAck, Request, Response};

const GUITAR: u64 = 1;
const KEYS: u64 = 2;

fn app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-sends-test.rprj"));
    app.test_add_track(GUITAR, TrackType::Instrument);
    app.test_add_track(KEYS, TrackType::Instrument);
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
    roundtrip(app, Request::new(1, method, &params).expect("params serialize"))
}

fn create_bus(app: &mut Resonance, name: &str) -> u64 {
    let result: CreateResult = call(app, "bus.create", serde_json::json!({"name": name}))
        .result()
        .expect("bus.create succeeds");
    app.test_apply_engine_event(AudioEvent::BusAdded {
        bus_id: result.bus_id.0,
        name: name.to_owned(),
    });
    result.bus_id.0
}

/// The engine's `AuxSendChanged` echo, which is what actually populates
/// the app's mirror.
fn echo_send(
    app: &mut Resonance,
    send_id: u64,
    track_id: u64,
    dest: u64,
    level_db: f32,
    pre_fader: bool,
    enabled: bool,
) {
    app.test_apply_engine_event(AudioEvent::AuxSendChanged {
        send_id,
        source: SendSource::Track(track_id),
        dest,
        level_db,
        pre_fader,
        enabled,
    });
}

/// Create a send over the control API and pump the engine echo.
fn add_send(app: &mut Resonance, track_id: u64, to_bus: u64, level_db: f32) -> u64 {
    let result: AddSendResult = call(
        app,
        "track.add_send",
        serde_json::json!({"track_id": track_id, "to_bus": to_bus, "level_db": level_db}),
    )
    .result()
    .expect("track.add_send succeeds");
    let send_id = result.send_id.0;
    echo_send(app, send_id, track_id, to_bus, level_db, false, true);
    send_id
}

fn sends_of(app: &mut Resonance, track_id: u64) -> Vec<SendView> {
    let view: TracksView = roundtrip(app, Request::without_params(99, "song.tracks"))
        .result()
        .expect("song.tracks succeeds");
    view.tracks
        .into_iter()
        .find(|t| t.summary.id.0 == track_id)
        .expect("track in song.tracks")
        .sends
}

#[test]
fn two_tracks_can_feed_one_return_and_both_sends_are_readable() {
    let mut app = app();
    let reverb = create_bus(&mut app, "Reverb");
    let rx = app.test_capture_engine();

    let guitar_send = add_send(&mut app, GUITAR, reverb, -6.0);
    let keys_send = add_send(&mut app, KEYS, reverb, -12.0);
    assert_ne!(guitar_send, keys_send, "each send gets its own id");

    let commands: Vec<AudioCommand> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    // The destination is flagged a RETURN bus as part of the gesture, so
    // a client never has to know busses carry a role.
    assert!(
        commands.iter().any(|c| matches!(
            c,
            AudioCommand::SetBusRole { bus_id, is_return: true } if *bus_id == reverb
        )),
        "the destination must be marked as a return bus"
    );
    // The app's chosen id reaches the engine as a hint, not a wait.
    assert!(
        commands.iter().any(|c| matches!(
            c,
            AudioCommand::SetAuxSend { id_hint: Some(id), dest, level_db, .. }
                if *id == guitar_send && *dest == reverb && (*level_db - -6.0).abs() < 1e-4
        )),
        "SetAuxSend must carry the app-chosen id"
    );

    let guitar = sends_of(&mut app, GUITAR);
    assert_eq!(guitar.len(), 1);
    assert_eq!(guitar[0].send_id.0, guitar_send);
    assert_eq!(guitar[0].to_bus.0, reverb);
    assert!((guitar[0].level_db - -6.0).abs() < 1e-4);
    assert!(!guitar[0].pre_fader, "post-fader by default");
    assert!(guitar[0].enabled);

    let keys = sends_of(&mut app, KEYS);
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].send_id.0, keys_send);
    assert!((keys[0].level_db - -12.0).abs() < 1e-4);
}

#[test]
fn level_pre_post_and_enable_can_be_changed_and_read_back() {
    let mut app = app();
    let reverb = create_bus(&mut app, "Reverb");
    let send_id = add_send(&mut app, GUITAR, reverb, 0.0);

    let _: MutationAck = call(
        &mut app,
        "track.set_send",
        serde_json::json!({"send_id": send_id, "level_db": -3.5, "pre_fader": true, "enabled": false}),
    )
    .result()
    .expect("track.set_send succeeds");
    // The engine resolves each upsert and echoes the result.
    echo_send(&mut app, send_id, GUITAR, reverb, -3.5, true, false);

    let send = sends_of(&mut app, GUITAR).remove(0);
    assert!((send.level_db - -3.5).abs() < 1e-4);
    assert!(send.pre_fader);
    assert!(!send.enabled);

    // Setting values it already has is a no-op — no engine traffic, no
    // second undo entry.
    let rx = app.test_capture_engine();
    let before = app.revision();
    let _: MutationAck = call(
        &mut app,
        "track.set_send",
        serde_json::json!({"send_id": send_id, "level_db": -3.5, "pre_fader": true}),
    )
    .result()
    .expect("succeeds");
    assert_eq!(app.revision(), before, "an idempotent set records nothing");
    assert!(rx.try_recv().is_err(), "and sends no engine command");
}

#[test]
fn a_send_can_be_re_routed_into_a_different_return() {
    let mut app = app();
    let reverb = create_bus(&mut app, "Reverb");
    let delay = create_bus(&mut app, "Delay");
    let send_id = add_send(&mut app, GUITAR, reverb, 0.0);

    let _: MutationAck = call(
        &mut app,
        "track.set_send",
        serde_json::json!({"send_id": send_id, "to_bus": delay}),
    )
    .result()
    .expect("succeeds");
    echo_send(&mut app, send_id, GUITAR, delay, 0.0, false, true);
    assert_eq!(sends_of(&mut app, GUITAR)[0].to_bus.0, delay);
}

#[test]
fn removing_a_send_takes_it_out_of_the_view() {
    let mut app = app();
    let reverb = create_bus(&mut app, "Reverb");
    let send_id = add_send(&mut app, GUITAR, reverb, 0.0);
    assert_eq!(sends_of(&mut app, GUITAR).len(), 1);

    let rx = app.test_capture_engine();
    let _: MutationAck = call(
        &mut app,
        "track.remove_send",
        serde_json::json!({"send_id": send_id}),
    )
    .result()
    .expect("track.remove_send succeeds");
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok())
            .any(|c| matches!(c, AudioCommand::RemoveAuxSend { send_id: id } if id == send_id)),
        "the engine must be told to drop the send"
    );

    app.test_apply_engine_event(AudioEvent::AuxSendRemoved { send_id });
    assert!(sends_of(&mut app, GUITAR).is_empty());
}

#[test]
fn unknown_ids_and_bad_levels_are_rejected() {
    let mut app = app();
    let reverb = create_bus(&mut app, "Reverb");

    let error = call(
        &mut app,
        "track.add_send",
        serde_json::json!({"track_id": 4242, "to_bus": reverb}),
    )
    .error
    .expect("unknown track rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);

    let error = call(
        &mut app,
        "track.add_send",
        serde_json::json!({"track_id": GUITAR, "to_bus": 999_999}),
    )
    .error
    .expect("unknown bus rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(error.message.contains("bus.create"), "{}", error.message);

    let error = call(
        &mut app,
        "track.add_send",
        serde_json::json!({"track_id": GUITAR, "to_bus": reverb, "level_db": 200.0}),
    )
    .error
    .expect("out-of-range level rejected");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);

    let error = call(
        &mut app,
        "track.set_send",
        serde_json::json!({"send_id": 7_777, "level_db": 0.0}),
    )
    .error
    .expect("unknown send rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);

    let send_id = add_send(&mut app, GUITAR, reverb, 0.0);
    let error = call(
        &mut app,
        "track.set_send",
        serde_json::json!({"send_id": send_id}),
    )
    .error
    .expect("a set that changes nothing is rejected");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);

    let error = call(
        &mut app,
        "track.remove_send",
        serde_json::json!({"send_id": 7_777}),
    )
    .error
    .expect("unknown send rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);
}

/// A track's own routing and its sends are independent: adding a send
/// must not disturb where the track's main output goes.
#[test]
fn a_send_is_a_tap_not_a_re_route() {
    let mut app = app();
    let reverb = create_bus(&mut app, "Reverb");
    add_send(&mut app, GUITAR, reverb, -6.0);

    let view: TracksView = roundtrip(&mut app, Request::without_params(99, "song.tracks"))
        .result()
        .expect("song.tracks succeeds");
    let guitar = view
        .tracks
        .iter()
        .find(|t| t.summary.id.0 == GUITAR)
        .expect("guitar");
    assert_eq!(
        guitar.summary.output,
        resonance_control::TrackOutput::Master,
        "the main output is untouched by a send"
    );
    // A track with no sends reports none at all rather than an empty
    // field on every line.
    let keys = view
        .tracks
        .iter()
        .find(|t| t.summary.id.0 == KEYS)
        .expect("keys");
    assert!(keys.sends.is_empty());
}

#[test]
fn every_send_method_is_advertised_in_the_handshake() {
    let capabilities = resonance_control::methods::capabilities();
    for method in ["track.add_send", "track.set_send", "track.remove_send"] {
        assert!(capabilities.contains(&method), "{method} missing");
    }
}
