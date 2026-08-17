//! `bus.create` commits synchronously, and `track.set_output` has no
//! unreachable bus branch (ba doc #273, todo #1238 items 1 and 2).
//!
//! `bus.create` returned the new id in its reply, but the app's
//! `registry.busses` only learned of the bus when the engine's
//! `BusAdded` echo landed — and `track.set_output` / `track.add_send`
//! validate against exactly that registry. The `bus_create` tool
//! description promises callers can "route tracks into it on the very
//! next call", which was true in practice and not guaranteed. Same class
//! of bug as todo #1234's `track.add_effect`, fixed the same way: mirror
//! app-side at dispatch, and let the (already idempotent) echo be a
//! no-op.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance};
use resonance_audio::types::{AudioEvent, TrackType};
use resonance_control::methods::bus::CreateResult;
use resonance_control::methods::song::SongSummary;
use resonance_control::{ErrorKind, MutationAck, Request, Response};

const TRACK: u64 = 1;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-bus-commit.rprj"));
    app.test_add_track(TRACK, TrackType::Instrument);
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
    result.bus_id.0
}

fn summary(app: &mut Resonance) -> SongSummary {
    roundtrip(app, Request::without_params(1, "song.summary"))
        .result()
        .expect("song.summary succeeds")
}

// ---------------------------------------------------------------------------
// Item 1 — bus.create commits synchronously
// ---------------------------------------------------------------------------

#[test]
fn a_track_can_be_routed_into_the_bus_on_the_very_next_call() {
    let mut app = app();
    let bus_id = create_bus(&mut app, "Drum Bus");
    // Deliberately NO engine echo applied: the promise in the tool
    // description has to hold without one.
    let _: MutationAck = call(
        &mut app,
        "track.set_output",
        serde_json::json!({"track_id": TRACK, "output": {"bus_id": bus_id}}),
    )
    .result()
    .expect("the bus the reply just named must be routable immediately");
}

#[test]
fn a_send_can_target_the_bus_on_the_very_next_call() {
    let mut app = app();
    let bus_id = create_bus(&mut app, "Reverb Return");
    let _: serde_json::Value = call(
        &mut app,
        "track.add_send",
        serde_json::json!({"track_id": TRACK, "to_bus": bus_id}),
    )
    .result()
    .expect("track.add_send validates against the same registry");
}

#[test]
fn the_new_bus_is_listed_before_any_engine_echo() {
    let mut app = app();
    let bus_id = create_bus(&mut app, "Drum Bus");
    let listed = summary(&mut app)
        .tracks
        .into_iter()
        .find(|t| t.id.0 == bus_id)
        .expect("the bus appears in song.summary with no engine round-trip");
    assert_eq!(listed.name, "Drum Bus");
}

#[test]
fn the_engine_echo_does_not_duplicate_the_bus() {
    let mut app = app();
    let bus_id = create_bus(&mut app, "Drum Bus");
    app.test_apply_engine_event(AudioEvent::BusAdded {
        bus_id,
        name: "Drum Bus".to_owned(),
    });
    let count = summary(&mut app)
        .tracks
        .iter()
        .filter(|t| t.id.0 == bus_id)
        .count();
    assert_eq!(count, 1, "the echo must be a no-op, not a second bus");
}

#[test]
fn two_busses_get_distinct_ids_even_with_no_echoes() {
    let mut app = app();
    let first = create_bus(&mut app, "Drum Bus");
    let second = create_bus(&mut app, "Vocal Bus");
    assert_ne!(
        first, second,
        "the allocator must skip past ids the app already mirrors"
    );
    let names: Vec<String> = summary(&mut app)
        .tracks
        .into_iter()
        .filter(|t| t.id.0 == first || t.id.0 == second)
        .map(|t| t.name)
        .collect();
    assert_eq!(names.len(), 2, "both busses are listed");
}

// ---------------------------------------------------------------------------
// Item 2 — the bus branch in track.set_output was unreachable
// ---------------------------------------------------------------------------

#[test]
fn routing_a_bus_as_if_it_were_a_track_is_a_plain_not_found() {
    let mut app = app();
    let bus_id = create_bus(&mut app, "Drum Bus");
    let error = call(
        &mut app,
        "track.set_output",
        serde_json::json!({"track_id": bus_id, "output": "master"}),
    )
    .error
    .expect("a bus is not a routable source");
    // `find_track` searches `registry.tracks`, which never holds a bus,
    // so the source lookup fails first. The handler used to carry a
    // "busses always feed master" branch AFTER that check, which could
    // never run.
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(
        error.message.contains("no track with id"),
        "the refusal is the track lookup itself: {}",
        error.message
    );
}

#[test]
fn a_bus_is_still_a_valid_destination() {
    // The removed branch was about a bus as the SOURCE; a bus as the
    // destination is the whole point of the method and must still work.
    let mut app = app();
    let bus_id = create_bus(&mut app, "Drum Bus");
    let _: MutationAck = call(
        &mut app,
        "track.set_output",
        serde_json::json!({"track_id": TRACK, "output": {"bus_id": bus_id}}),
    )
    .result()
    .expect("routing a track INTO a bus is unaffected");

    let error = call(
        &mut app,
        "track.set_output",
        serde_json::json!({"track_id": TRACK, "output": {"bus_id": 999_999}}),
    )
    .error
    .expect("an unknown destination bus is still rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(error.message.contains("bus.create"), "{}", error.message);
}
