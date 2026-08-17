//! `bus.create` / `bus.delete` / `bus.set_volume` + `track.set_output`
//! (ba doc #273, todo #1228).
//!
//! Busses existed everywhere except the control API, so no client could
//! build a drum bus — the one change the field measurement identified as
//! most actionable. Everything here is wiring: `BusMessage`,
//! `update/bus.rs`, `TrackMessage::SetTrackOutput` and the persistence
//! were all already in place.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance};
use resonance_audio::types::{AudioCommand, AudioEvent, TrackOutput, TrackType};
use resonance_control::methods::bus::CreateResult;
use resonance_control::methods::song::SongSummary;
use resonance_control::{ErrorKind, MutationAck, Request, Response, TrackKind};
use resonance_control::TrackOutput as WireTrackOutput;

const KICK: u64 = 1;
const SNARE: u64 = 2;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-bus-test.rprj"));
    app.test_add_track(KICK, TrackType::Instrument);
    app.test_add_track(SNARE, TrackType::Instrument);
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

fn summary(app: &mut Resonance) -> SongSummary {
    roundtrip(app, Request::without_params(99, "song.summary"))
        .result()
        .expect("song.summary succeeds")
}

/// Create a bus over the control API and drive the engine echo that
/// mirrors it into the registry (the real engine does this async).
fn create_bus(app: &mut Resonance, name: &str) -> u64 {
    let result: CreateResult = call(app, "bus.create", serde_json::json!({"name": name}))
        .result()
        .expect("bus.create succeeds");
    let bus_id = result.bus_id.0;
    app.test_apply_engine_event(AudioEvent::BusAdded {
        bus_id,
        name: name.to_owned(),
    });
    bus_id
}

#[test]
fn create_returns_the_id_in_the_reply_and_the_bus_shows_up_in_song_summary() {
    let mut app = app();
    let rx = app.test_capture_engine();

    let result: CreateResult = call(&mut app, "bus.create", serde_json::json!({"name": "Drum Bus"}))
        .result()
        .expect("bus.create succeeds");
    assert!(result.bus_id.0 > 0, "a real id, in the reply");
    assert!(result.revision > 0);

    // The engine is told to create exactly that bus — the id is a hint,
    // not something to wait for.
    let hinted = std::iter::from_fn(|| rx.try_recv().ok()).any(|c| {
        matches!(c, AudioCommand::AddBus { id_hint: Some(id), name: Some(n) }
            if id == result.bus_id.0 && n == "Drum Bus")
    });
    assert!(hinted, "AddBus must carry the app-chosen id and name");

    app.test_apply_engine_event(AudioEvent::BusAdded {
        bus_id: result.bus_id.0,
        name: "Drum Bus".to_owned(),
    });
    let bus = summary(&mut app)
        .tracks
        .into_iter()
        .find(|t| t.id.0 == result.bus_id.0)
        .expect("the bus appears in song.summary");
    assert_eq!(bus.kind, TrackKind::Bus);
    assert_eq!(bus.name, "Drum Bus");
    assert_eq!(bus.output, WireTrackOutput::Master, "busses feed master");
}

#[test]
fn set_output_routes_a_track_through_the_bus_and_song_tracks_reports_it() {
    let mut app = app();
    let bus_id = create_bus(&mut app, "Drum Bus");
    let rx = app.test_capture_engine();

    let _: MutationAck = call(
        &mut app,
        "track.set_output",
        serde_json::json!({"track_id": KICK, "output": {"bus_id": bus_id}}),
    )
    .result()
    .expect("track.set_output succeeds");

    // Audio actually flows through the bus: the engine's routing state
    // is driven, not just the model.
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(
            c,
            AudioCommand::SetTrackOutput { track_id, output }
                if track_id == KICK && output == TrackOutput::Bus(bus_id)
        )),
        "the engine must be told about the new route"
    );

    let view = summary(&mut app);
    let by_id = |id: u64| {
        view.tracks
            .iter()
            .find(|t| t.id.0 == id)
            .unwrap_or_else(|| panic!("track {id}"))
    };
    assert_eq!(by_id(KICK).output, WireTrackOutput::Bus(bus_id.into()));
    assert_eq!(by_id(SNARE).output, WireTrackOutput::Master);

    // Routing back to master works, and re-setting the same routing is a
    // no-op rather than a second undo entry.
    let _: MutationAck = call(
        &mut app,
        "track.set_output",
        serde_json::json!({"track_id": KICK, "output": "master"}),
    )
    .result()
    .expect("succeeds");
    let before = app.revision();
    let _: MutationAck = call(
        &mut app,
        "track.set_output",
        serde_json::json!({"track_id": KICK, "output": "master"}),
    )
    .result()
    .expect("succeeds");
    assert_eq!(app.revision(), before, "an idempotent re-route records nothing");
}

#[test]
fn set_output_rejects_unknown_busses_tracks_and_busses_as_the_source() {
    let mut app = app();
    let bus_id = create_bus(&mut app, "Drum Bus");

    let error = call(
        &mut app,
        "track.set_output",
        serde_json::json!({"track_id": KICK, "output": {"bus_id": 999_999}}),
    )
    .error
    .expect("unknown bus rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(error.message.contains("bus.create"), "{}", error.message);

    let error = call(
        &mut app,
        "track.set_output",
        serde_json::json!({"track_id": 4242, "output": "master"}),
    )
    .error
    .expect("unknown track rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);

    // A bus id is a track id in the same space, so routing a bus is
    // expressible — and refused. Note the reason, which the comment here
    // used to get wrong (todo #1238 item 2): it is `not_found`, because
    // `track.set_output` looks the source up among TRACKS and a bus is
    // not one. Busses do always feed master, but that rule is enforced
    // by the engine's model, not by a branch in this handler.
    let error = call(
        &mut app,
        "track.set_output",
        serde_json::json!({"track_id": bus_id, "output": "master"}),
    )
    .error
    .expect("a bus is not a routable source");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(
        error.message.contains("no track with id"),
        "the refusal is the plain track lookup, not a bus-specific branch: {}",
        error.message
    );
}

#[test]
fn set_volume_moves_the_bus_fader_and_validates_its_range() {
    let mut app = app();
    let bus_id = create_bus(&mut app, "Drum Bus");

    let _: MutationAck = call(
        &mut app,
        "bus.set_volume",
        serde_json::json!({"bus_id": bus_id, "volume_db": -2.0}),
    )
    .result()
    .expect("bus.set_volume succeeds");

    let bus = summary(&mut app)
        .tracks
        .into_iter()
        .find(|t| t.id.0 == bus_id)
        .expect("bus");
    assert!((bus.volume_db - -2.0).abs() < 1e-4, "{}", bus.volume_db);

    for db in [-61.0f32, 7.0] {
        let error = call(
            &mut app,
            "bus.set_volume",
            serde_json::json!({"bus_id": bus_id, "volume_db": db}),
        )
        .error
        .unwrap_or_else(|| panic!("{db} dB should be rejected"));
        assert_eq!(error.kind(), ErrorKind::InvalidParams);
    }

    let error = call(
        &mut app,
        "bus.set_volume",
        serde_json::json!({"bus_id": 999_999, "volume_db": 0.0}),
    )
    .error
    .expect("unknown bus rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);
}

#[test]
fn delete_needs_confirmation_and_re_routes_members_to_master() {
    let mut app = app();
    let bus_id = create_bus(&mut app, "Drum Bus");
    let _: MutationAck = call(
        &mut app,
        "track.set_output",
        serde_json::json!({"track_id": KICK, "output": {"bus_id": bus_id}}),
    )
    .result()
    .expect("succeeds");

    let error = call(&mut app, "bus.delete", serde_json::json!({"bus_id": bus_id}))
        .error
        .expect("delete needs confirmation");
    assert_eq!(error.kind(), ErrorKind::NeedsConfirmation);
    assert!(
        error.message.contains("master"),
        "the warning must say members fall back to master, not that they are lost: {}",
        error.message
    );

    let rx = app.test_capture_engine();
    let _: MutationAck = call(
        &mut app,
        "bus.delete",
        serde_json::json!({"bus_id": bus_id, "confirm": true}),
    )
    .result()
    .expect("confirmed delete succeeds");
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok())
            .any(|c| matches!(c, AudioCommand::RemoveBus { bus_id: id } if id == bus_id)),
        "the engine must be told to drop the bus"
    );

    app.test_apply_engine_event(AudioEvent::BusRemoved { bus_id });
    let view = summary(&mut app);
    assert!(
        view.tracks.iter().all(|t| t.id.0 != bus_id),
        "the bus is gone from song.summary"
    );
    let kick = view.tracks.iter().find(|t| t.id.0 == KICK).expect("kick");
    assert_eq!(
        kick.output,
        WireTrackOutput::Master,
        "its member falls back to master rather than being silenced"
    );
}

/// A bus and the routing into it must survive save + reload.
#[test]
fn busses_and_routing_are_persisted() {
    let mut app = app();
    let bus_id = create_bus(&mut app, "Drum Bus");
    let _: MutationAck = call(
        &mut app,
        "bus.set_volume",
        serde_json::json!({"bus_id": bus_id, "volume_db": -2.0}),
    )
    .result()
    .expect("succeeds");
    let _: MutationAck = call(
        &mut app,
        "track.set_output",
        serde_json::json!({"track_id": KICK, "output": {"bus_id": bus_id}}),
    )
    .result()
    .expect("succeeds");

    let file = app.test_build_project_file();
    let bus = file
        .busses
        .iter()
        .find(|b| b.id == bus_id)
        .expect("the bus is serialized");
    assert_eq!(bus.name, "Drum Bus");
    assert!((bus.volume - -2.0).abs() < 1e-4, "{}", bus.volume);
    let kick = file.tracks.iter().find(|t| t.id == KICK).expect("kick");
    assert_eq!(kick.output_bus, Some(bus_id), "the route is serialized");
}

#[test]
fn every_bus_method_is_advertised_in_the_handshake() {
    let capabilities = resonance_control::methods::capabilities();
    for method in [
        "bus.create",
        "bus.delete",
        "bus.set_volume",
        "track.set_output",
    ] {
        assert!(capabilities.contains(&method), "{method} missing");
    }
}
