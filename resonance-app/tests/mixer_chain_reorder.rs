//! Chain reorder from the GUI — the ▲/▼ pair on the mixer inspector's
//! CHAIN rows and on every channel strip's plugin slot (ba todo #1302,
//! doc #276 item 2.1).
//!
//! Reordering has been complete in the backend since ba doc #273 and
//! reachable over MCP as `track/bus/master.move_effect` — but the view
//! layer raised none of the three messages, so an agent could reorder a
//! chain and a human could not. A user correcting insert order had to
//! delete the plugin and re-add it, losing every parameter.
//!
//! These pin the *view's* half of that: which direction each slot
//! offers, that the ends and the instrument floor come back disabled
//! rather than as a move the pre-dispatch gate then silently drops, and
//! that pressing the affordance the GUI hands out lands the same
//! reorder — one undo step — that the control API reads back.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, TestChain, STARTUP_TAB};
use resonance_audio::types::{AudioCommand, AudioEvent, ScannedPlugin, TrackType};
use resonance_control::{Request, Response};

const TRACK: u64 = 1;

fn app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Mixer);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/mixer-chain-reorder.rprj"));
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![
            scanned("wavetable", "Resonance Wavetable", true),
            scanned("eq", "Resonance EQ", false),
            scanned("compressor", "Resonance Compressor", false),
            scanned("reverb", "Resonance Reverb", false),
        ],
    });
    app
}

fn scanned(short: &str, name: &str, is_instrument: bool) -> ScannedPlugin {
    ScannedPlugin {
        clap_file_path: format!("/plugins/{short}.clap"),
        clap_plugin_id: format!("com.resonance.{short}"),
        name: name.to_owned(),
        vendor: "Resonance".to_owned(),
        is_instrument,
    }
}

fn call(app: &mut Resonance, method: &str, params: serde_json::Value) -> Response {
    let request = Request::new(1, method, &params).expect("params serialize");
    let (reply, rx) = ReplySender::test_pair();
    let _ = app.update(Message::Control(ControlMessage::Request(ControlRequest {
        conn: 1,
        request,
        reply,
    })));
    rx.try_recv().expect("one reply per request")
}

// ---------------------------------------------------------------------------
// Chain building — through the control API, so the fixtures are built
// the same way the app builds them at runtime.
// ---------------------------------------------------------------------------

fn add_instrument(app: &mut Resonance) {
    let _: serde_json::Value = call(
        app,
        "track.add_instrument",
        serde_json::json!({"track_id": TRACK, "plugin_id": "com.resonance.wavetable"}),
    )
    .result()
    .expect("track.add_instrument succeeds");
}

fn add_track_effect(app: &mut Resonance, short: &str) {
    let _: serde_json::Value = call(
        app,
        "track.add_effect",
        serde_json::json!({"track_id": TRACK, "plugin_id": format!("com.resonance.{short}")}),
    )
    .result()
    .expect("track.add_effect succeeds");
}

fn create_bus(app: &mut Resonance) -> u64 {
    let result: resonance_control::methods::bus::CreateResult =
        call(app, "bus.create", serde_json::json!({"name": "Drum Bus"}))
            .result()
            .expect("bus.create succeeds");
    result.bus_id.0
}

fn add_bus_effect(app: &mut Resonance, bus_id: u64, short: &str) {
    let _: serde_json::Value = call(
        app,
        "bus.add_effect",
        serde_json::json!({"bus_id": bus_id, "plugin_id": format!("com.resonance.{short}")}),
    )
    .result()
    .expect("bus.add_effect succeeds");
}

fn add_master_effect(app: &mut Resonance, short: &str) {
    let _: serde_json::Value = call(
        app,
        "master.add_effect",
        serde_json::json!({"plugin_id": format!("com.resonance.{short}")}),
    )
    .result()
    .expect("master.add_effect succeeds");
}

// ---------------------------------------------------------------------------
// Read-back — the chain order as the control API reports it, which is
// the DoD's "same order reported by *.plugin_params immediately
// afterwards".
// ---------------------------------------------------------------------------

fn short_ids(ids: Vec<String>) -> Vec<String> {
    ids.into_iter()
        .map(|s| s.rsplit('.').next().unwrap_or(&s).to_owned())
        .collect()
}

fn track_order(app: &mut Resonance) -> Vec<String> {
    let view: resonance_control::methods::track::PluginParamsView = call(
        app,
        "track.plugin_params",
        serde_json::json!({"track_id": TRACK}),
    )
    .result()
    .expect("track.plugin_params succeeds");
    short_ids(view.plugins.into_iter().map(|p| p.plugin_id).collect())
}

fn bus_order(app: &mut Resonance, bus_id: u64) -> Vec<String> {
    let view: resonance_control::methods::bus::PluginParamsView = call(
        app,
        "bus.plugin_params",
        serde_json::json!({"bus_id": bus_id}),
    )
    .result()
    .expect("bus.plugin_params succeeds");
    short_ids(view.plugins.into_iter().map(|p| p.plugin_id).collect())
}

fn master_order(app: &mut Resonance) -> Vec<String> {
    let view: resonance_control::methods::master::PluginParamsView =
        call(app, "master.plugin_params", serde_json::json!({}))
            .result()
            .expect("master.plugin_params succeeds");
    short_ids(view.plugins.into_iter().map(|p| p.plugin_id).collect())
}

/// `(▲ enabled, ▼ enabled)` per slot — what the carets look like.
fn enabled(app: &Resonance, chain: TestChain) -> Vec<(bool, bool)> {
    app.test_chain_move_affordances(chain)
        .into_iter()
        .map(|(up, down)| (up.is_some(), down.is_some()))
        .collect()
}

/// Press one caret. Panics if the affordance is disabled — a test that
/// wants the disabled case asserts on [`enabled`] instead of pressing.
fn press(app: &mut Resonance, chain: TestChain, slot: usize, up: bool) {
    let moves = app.test_chain_move_affordances(chain);
    let (u, d) = moves.into_iter().nth(slot).expect("slot exists");
    let message = if up { u } else { d }.expect("the caret is enabled");
    let _ = app.update(message);
}

// ---------------------------------------------------------------------------
// What each slot offers
// ---------------------------------------------------------------------------

#[test]
fn the_ends_of_a_chain_offer_only_the_direction_that_exists() {
    let mut app = app();
    for fx in ["eq", "compressor", "reverb"] {
        add_track_effect(&mut app, fx);
    }

    assert_eq!(
        enabled(&app, TestChain::Track(TRACK)),
        vec![(false, true), (true, true), (true, false)],
        "first row cannot move up, last row cannot move down"
    );
}

/// The whole point of disabling rather than dispatching: the greyed
/// caret and the pre-dispatch gate must agree, or the user presses a
/// live-looking button and nothing happens.
#[test]
fn a_lone_plugin_offers_no_move_at_all() {
    let mut app = app();
    add_track_effect(&mut app, "eq");
    assert_eq!(enabled(&app, TestChain::Track(TRACK)), vec![(false, false)]);
}

#[test]
fn the_instrument_slot_is_a_floor_the_carets_respect() {
    let mut app = app();
    add_instrument(&mut app);
    add_track_effect(&mut app, "eq");
    add_track_effect(&mut app, "compressor");
    assert_eq!(track_order(&mut app), vec!["wavetable", "eq", "compressor"]);

    assert_eq!(
        enabled(&app, TestChain::Track(TRACK)),
        vec![
            // The instrument is structural: it moves in neither
            // direction, so both of its carets are dead.
            (false, false),
            // The effect directly after it cannot move up onto it.
            (false, true),
            (true, false),
        ],
    );
}

/// A bus has no instrument slot, so only the two ends limit it — the
/// same shape a plain audio track's chain has.
#[test]
fn bus_and_master_chains_are_limited_only_by_their_ends() {
    let mut app = app();
    let bus_id = create_bus(&mut app);
    add_bus_effect(&mut app, bus_id, "eq");
    add_bus_effect(&mut app, bus_id, "compressor");
    add_master_effect(&mut app, "eq");
    add_master_effect(&mut app, "compressor");
    add_master_effect(&mut app, "reverb");

    assert_eq!(
        enabled(&app, TestChain::Bus(bus_id)),
        vec![(false, true), (true, false)]
    );
    assert_eq!(
        enabled(&app, TestChain::Master),
        vec![(false, true), (true, true), (true, false)]
    );
}

// ---------------------------------------------------------------------------
// Pressing them
// ---------------------------------------------------------------------------

#[test]
fn pressing_a_caret_reorders_the_track_chain_and_tells_the_engine() {
    let mut app = app();
    for fx in ["eq", "compressor", "reverb"] {
        add_track_effect(&mut app, fx);
    }

    let rx = app.test_capture_engine();
    press(&mut app, TestChain::Track(TRACK), 2, true);
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok())
            .any(|c| matches!(c, AudioCommand::MovePlugin { to_index: 1, .. })),
        "the engine must be told to reorder its own chain"
    );
    assert_eq!(
        track_order(&mut app),
        vec!["eq", "reverb", "compressor"],
        "and `track.plugin_params` reads the new order back in the same cycle"
    );

    press(&mut app, TestChain::Track(TRACK), 0, false);
    assert_eq!(track_order(&mut app), vec!["reverb", "eq", "compressor"]);
}

#[test]
fn pressing_a_caret_reorders_a_bus_chain() {
    let mut app = app();
    let bus_id = create_bus(&mut app);
    add_bus_effect(&mut app, bus_id, "eq");
    add_bus_effect(&mut app, bus_id, "compressor");

    let rx = app.test_capture_engine();
    press(&mut app, TestChain::Bus(bus_id), 1, true);
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok())
            .any(|c| matches!(c, AudioCommand::MovePluginInBus { to_index: 0, .. })),
        "the engine must be told to reorder its own chain"
    );
    assert_eq!(bus_order(&mut app, bus_id), vec!["compressor", "eq"]);
}

#[test]
fn pressing_a_caret_reorders_the_master_chain() {
    let mut app = app();
    add_master_effect(&mut app, "eq");
    add_master_effect(&mut app, "compressor");

    let rx = app.test_capture_engine();
    press(&mut app, TestChain::Master, 0, false);
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok())
            .any(|c| matches!(c, AudioCommand::MovePluginInMaster { to_index: 1, .. })),
        "the engine must be told to reorder its own chain"
    );
    assert_eq!(master_order(&mut app), vec!["compressor", "eq"]);
}

/// A GUI reorder is one undo step, like the control-API one it shares a
/// handler with — not two (the remove + insert it is implemented as) and
/// not zero.
///
/// Chain order is structural, so the restore itself takes the
/// `ClearAll` → replay path and finishes asynchronously; as in
/// `control_track_move_effect.rs` the synchronous assertion is that undo
/// finds the entry and starts restoring the pre-move snapshot.
#[test]
fn a_gui_reorder_is_exactly_one_undo_step() {
    let mut app = app();
    for fx in ["eq", "compressor", "reverb"] {
        add_track_effect(&mut app, fx);
    }
    let before = app.revision();
    let entries_before = app.test_undo_history().test_undo_entries().len();

    press(&mut app, TestChain::Track(TRACK), 0, false);
    assert_eq!(track_order(&mut app), vec!["compressor", "eq", "reverb"]);
    assert_eq!(app.revision(), before + 1, "one committed edit");
    assert_eq!(
        app.test_undo_history().test_undo_entries().len(),
        entries_before + 1,
        "the reorder pushed exactly one entry — not the two its \
         remove+insert implementation could have cost, and not zero"
    );

    let rx = app.test_capture_engine();
    let _ = app.update(Message::Undo);
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(c, AudioCommand::ClearAll)),
        "undo must find the reorder and start restoring the pre-move snapshot"
    );
}

/// The echo the engine sends back replays a move the app already
/// mirrored, so it must land as a no-op rather than moving the plugin a
/// second time.
#[test]
fn the_engine_echo_after_a_gui_reorder_is_a_no_op() {
    let mut app = app();
    add_track_effect(&mut app, "eq");
    add_track_effect(&mut app, "compressor");

    let rx = app.test_capture_engine();
    press(&mut app, TestChain::Track(TRACK), 1, true);
    let moved = std::iter::from_fn(|| rx.try_recv().ok())
        .find_map(|c| match c {
            AudioCommand::MovePlugin { instance_id, .. } => Some(instance_id),
            _ => None,
        })
        .expect("a MovePlugin reached the engine");
    assert_eq!(track_order(&mut app), vec!["compressor", "eq"]);

    app.test_apply_engine_event(AudioEvent::PluginMoved {
        track_id: TRACK,
        instance_id: moved,
        to_index: 0,
    });
    assert_eq!(track_order(&mut app), vec!["compressor", "eq"]);
}
