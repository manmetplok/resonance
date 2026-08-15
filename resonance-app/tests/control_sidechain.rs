//! `bus.set_sidechain` / `bus.clear_sidechain` and
//! `master.set_sidechain` / `master.clear_sidechain` — key routing onto
//! the targets the control API could not reach (ba doc #275 finding P4,
//! ba todo #1311).
//!
//! `track.set_sidechain` is keyed on `track_id`, so a compressor on a
//! **bus** — where a keyed ducker usually lives, because the point is to
//! duck a whole group — could not be keyed at all, nor could one on the
//! master. The engine never had that limit: its route table is keyed by
//! plugin instance and the mixer connects a key port wherever it sits.
//! Only the wire was track-shaped.
//!
//! The `track.*` arm is covered by the persistence suite's mirror
//! assertions; what is pinned here is the widened surface and the two
//! rules all three arms share (exactly one source; a key port required).

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::{AudioEvent, ParamInfo, SendSource, TrackType};
use resonance_control::{ErrorKind, MutationAck, Request, Response};

const KICK: u64 = 1;
const BUS: u64 = 10;
const BUS_COMP: u64 = 200;
const BUS_SYNTH: u64 = 201;
const MASTER_COMP: u64 = 300;

const COMPRESSOR: &str = "com.resonance.compressor";
const WAVETABLE: &str = "com.resonance.wavetable";

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

fn bus_plugin(app: &mut Resonance, instance_id: u64, clap_plugin_id: &str, keyable: bool) {
    app.test_apply_engine_event(AudioEvent::BusPluginAdded {
        bus_id: BUS,
        instance_id,
        plugin_name: clap_plugin_id.to_string(),
        clap_plugin_id: clap_plugin_id.to_string(),
        clap_file_path: "/plugins/x.clap".to_string(),
        params: Vec::<ParamInfo>::new(),
        has_gui: false,
        has_sidechain_input: keyable,
    });
}

/// A kick track, a bus carrying a non-keyable plugin FIRST and a
/// compressor second (so "omitted plugin_id picks the keyable one, not
/// slot 0" is actually load-bearing), and a compressor on the master.
fn app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-sidechain.rproj"));
    app.test_add_track(KICK, TrackType::Instrument);
    app.test_add_bus(BUS, "Bass Bus");
    bus_plugin(&mut app, BUS_SYNTH, WAVETABLE, false);
    bus_plugin(&mut app, BUS_COMP, COMPRESSOR, true);
    app.test_apply_engine_event(AudioEvent::MasterPluginAdded {
        instance_id: MASTER_COMP,
        plugin_name: "Compressor".to_string(),
        clap_plugin_id: COMPRESSOR.to_string(),
        clap_file_path: "/plugins/compressor.clap".to_string(),
        params: Vec::<ParamInfo>::new(),
        has_gui: false,
        has_sidechain_input: true,
    });
    app
}

fn route_for(app: &Resonance, plugin: u64) -> Option<SendSource> {
    app.test_sidechain_routes()
        .iter()
        .find(|r| r.plugin == plugin)
        .map(|r| r.source)
}

// ---------------------------------------------------------------------------
// bus.set_sidechain
// ---------------------------------------------------------------------------

#[test]
fn bus_set_sidechain_keys_the_bus_compressor_from_a_track() {
    let mut app = app();

    let ack: MutationAck = call(
        &mut app,
        "bus.set_sidechain",
        serde_json::json!({"bus_id": BUS, "source_track_id": KICK}),
    )
    .result()
    .expect("bus.set_sidechain succeeds");
    assert!(ack.revision > 0);

    assert_eq!(
        route_for(&app, BUS_COMP),
        Some(SendSource::Track(KICK)),
        "an omitted plugin_id must key the plugin that HAS a key port"
    );
    assert_eq!(
        route_for(&app, BUS_SYNTH),
        None,
        "…and never the non-keyable plugin sitting at slot 0"
    );
}

#[test]
fn bus_set_sidechain_accepts_a_bus_as_the_key_source() {
    let mut app = app();
    app.test_add_bus(20, "Drum Bus");

    let _: MutationAck = call(
        &mut app,
        "bus.set_sidechain",
        serde_json::json!({"bus_id": BUS, "source_bus_id": 20}),
    )
    .result()
    .expect("a bus is a valid key source");

    assert_eq!(route_for(&app, BUS_COMP), Some(SendSource::Bus(20)));
}

#[test]
fn bus_set_sidechain_survives_into_the_project_file() {
    // The whole point of the finding: reachable AND durable.
    let mut app = app();
    let _: MutationAck = call(
        &mut app,
        "bus.set_sidechain",
        serde_json::json!({"bus_id": BUS, "source_track_id": KICK}),
    )
    .result()
    .expect("succeeds");

    let file = app.test_build_project_file();
    assert_eq!(file.sidechain_routes.len(), 1);
    assert_eq!(file.sidechain_routes[0].plugin_instance_id, BUS_COMP);
    assert_eq!(file.sidechain_routes[0].source_kind, "track");
    assert_eq!(file.sidechain_routes[0].source_id, KICK);
}

#[test]
fn bus_set_sidechain_can_park_a_route_with_enabled_false() {
    let mut app = app();
    let _: MutationAck = call(
        &mut app,
        "bus.set_sidechain",
        serde_json::json!({"bus_id": BUS, "source_track_id": KICK, "enabled": false}),
    )
    .result()
    .expect("succeeds");

    let route = app.test_sidechain_routes()[0];
    assert_eq!(route.plugin, BUS_COMP);
    assert!(!route.enabled, "a parked route keeps its source but no key");
}

#[test]
fn bus_clear_sidechain_unkeys_the_plugin() {
    let mut app = app();
    let _: MutationAck = call(
        &mut app,
        "bus.set_sidechain",
        serde_json::json!({"bus_id": BUS, "source_track_id": KICK}),
    )
    .result()
    .expect("succeeds");

    let _: MutationAck = call(
        &mut app,
        "bus.clear_sidechain",
        serde_json::json!({"bus_id": BUS}),
    )
    .result()
    .expect("bus.clear_sidechain succeeds");

    assert!(app.test_sidechain_routes().is_empty());
    assert!(app.test_build_project_file().sidechain_routes.is_empty());
}

// ---------------------------------------------------------------------------
// master.set_sidechain
// ---------------------------------------------------------------------------

#[test]
fn master_set_sidechain_keys_the_master_compressor_from_a_bus() {
    let mut app = app();

    let _: MutationAck = call(
        &mut app,
        "master.set_sidechain",
        serde_json::json!({"source_bus_id": BUS}),
    )
    .result()
    .expect("master.set_sidechain succeeds");

    assert_eq!(route_for(&app, MASTER_COMP), Some(SendSource::Bus(BUS)));
}

#[test]
fn master_clear_sidechain_takes_no_params() {
    let mut app = app();
    let _: MutationAck = call(
        &mut app,
        "master.set_sidechain",
        serde_json::json!({"source_track_id": KICK}),
    )
    .result()
    .expect("succeeds");

    let _: MutationAck = roundtrip(
        &mut app,
        Request::without_params(7, "master.clear_sidechain"),
    )
    .result()
    .expect("master.clear_sidechain needs no params");

    assert!(app.test_sidechain_routes().is_empty());
}

// ---------------------------------------------------------------------------
// The shared rules: one source, and a key port is required
// ---------------------------------------------------------------------------

#[test]
fn naming_both_a_track_and_a_bus_source_is_rejected() {
    let mut app = app();
    let err = call(
        &mut app,
        "bus.set_sidechain",
        serde_json::json!({"bus_id": BUS, "source_track_id": KICK, "source_bus_id": BUS}),
    )
    .error
    .expect("two sources must be refused");
    assert_eq!(err.kind(), ErrorKind::InvalidParams);
    assert!(app.test_sidechain_routes().is_empty(), "and nothing routed");
}

#[test]
fn naming_no_source_at_all_is_rejected_with_the_method_name() {
    let mut app = app();
    let err = call(
        &mut app,
        "bus.set_sidechain",
        serde_json::json!({"bus_id": BUS}),
    )
    .error
    .expect("no source must be refused");
    assert_eq!(err.kind(), ErrorKind::InvalidParams);
    assert!(
        err.message.contains("bus.set_sidechain"),
        "the error should name the method it is about: {}",
        err.message
    );
}

#[test]
fn a_source_that_does_not_exist_is_not_found() {
    let mut app = app();
    let err = call(
        &mut app,
        "bus.set_sidechain",
        serde_json::json!({"bus_id": BUS, "source_track_id": 4242}),
    )
    .error
    .expect("a missing source must be refused");
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

/// The ba doc #275 P0 rule, on the new arm: routing a key into a plugin
/// with no key port succeeds-and-does-nothing unless it is refused here.
#[test]
fn keying_a_bus_plugin_without_a_key_port_is_refused_and_names_the_alternatives() {
    let mut app = app();
    let err = call(
        &mut app,
        "bus.set_sidechain",
        serde_json::json!({
            "bus_id": BUS,
            "plugin_id": WAVETABLE,
            "source_track_id": KICK,
        }),
    )
    .error
    .expect("a plugin with no key port must be refused");
    assert_eq!(err.kind(), ErrorKind::InvalidParams);
    assert!(
        err.message.contains(COMPRESSOR),
        "the refusal should name the keyable plugin on the bus: {}",
        err.message
    );
    assert!(app.test_sidechain_routes().is_empty());
}

#[test]
fn a_bus_with_no_keyable_plugin_says_so_rather_than_keying_slot_zero() {
    let mut app = app();
    app.test_add_bus(30, "Synth Bus");
    app.test_apply_engine_event(AudioEvent::BusPluginAdded {
        bus_id: 30,
        instance_id: 301,
        plugin_name: "Wavetable".to_string(),
        clap_plugin_id: WAVETABLE.to_string(),
        clap_file_path: "/plugins/x.clap".to_string(),
        params: Vec::<ParamInfo>::new(),
        has_gui: false,
        has_sidechain_input: false,
    });

    let err = call(
        &mut app,
        "bus.set_sidechain",
        serde_json::json!({"bus_id": 30, "source_track_id": KICK}),
    )
    .error
    .expect("nothing on the bus can take a key");
    assert_eq!(err.kind(), ErrorKind::InvalidParams);
    assert!(app.test_sidechain_routes().is_empty());
}

#[test]
fn an_unknown_bus_is_not_found() {
    let mut app = app();
    let err = call(
        &mut app,
        "bus.set_sidechain",
        serde_json::json!({"bus_id": 999, "source_track_id": KICK}),
    )
    .error
    .expect("an unknown bus must be refused");
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[test]
fn an_unknown_plugin_on_a_real_bus_is_not_found_and_lists_the_chain() {
    let mut app = app();
    let err = call(
        &mut app,
        "bus.set_sidechain",
        serde_json::json!({
            "bus_id": BUS,
            "plugin_id": "com.resonance.nonexistent",
            "source_track_id": KICK,
        }),
    )
    .error
    .expect("an unknown plugin must be refused");
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(
        err.message.contains(COMPRESSOR),
        "the error should list what the bus actually carries: {}",
        err.message
    );
}

// ---------------------------------------------------------------------------
// The new methods are advertised
// ---------------------------------------------------------------------------

#[test]
fn the_new_methods_are_in_the_control_capability_list() {
    let methods = resonance_control::methods::capabilities();
    for wanted in [
        "bus.set_sidechain",
        "bus.clear_sidechain",
        "master.set_sidechain",
        "master.clear_sidechain",
    ] {
        assert!(
            methods.contains(&wanted),
            "{wanted} must be advertised by control.hello"
        );
    }
}
