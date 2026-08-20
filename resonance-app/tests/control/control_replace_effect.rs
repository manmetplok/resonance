//! `track/bus/master.replace_effect` and the `status` field on
//! `*.plugin_params` (ba doc #275 P5, todo #1309).
//!
//! The wire half of "a missing plugin is an indistinguishable dead slot".
//! Over the control API the slot was *worse* than indistinguishable: a
//! dead slot reported an empty `params` array, which reads exactly like a
//! plugin with no parameters, and `set_plugin_param` against it was
//! accepted and changed no sound. There was also no way to put anything
//! else in that position — `add_effect` only appends, so remove + add
//! moves the plugin to the end of the chain, and chain order is audible.
//!
//! The acceptance path the todo names is
//! [`acceptance_missing_plugin_is_reported_replaced_and_keeps_its_slot`]:
//! load with a missing plugin -> status reported -> replace -> chain
//! order preserved.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::{PluginSlotState, ViewMode};
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, PluginInstanceId, ScannedPlugin, TrackType};
use resonance_control::methods::track::{
    PluginParamsView, PluginSlotStatus, ReplaceEffectResult, ReplaceOutcome,
};
use resonance_control::{ErrorKind, Request, Response};

const TRACK: u64 = 1;
const BUS: u64 = 5;

const VERB: u64 = 100;
const EQ: u64 = 101;
const COMP: u64 = 102;

const VERB_ID: &str = "com.thirdparty.mega-verb";
const EQ_ID: &str = "com.resonance.eq";
const COMP_ID: &str = "com.resonance.compressor";
const SYNTH_ID: &str = "com.resonance.wavetable";

fn catalog() -> Vec<ScannedPlugin> {
    vec![
        ScannedPlugin {
            clap_file_path: "/plugins/eq.clap".to_owned(),
            clap_plugin_id: EQ_ID.to_owned(),
            name: "Resonance EQ".to_owned(),
            vendor: "Resonance".to_owned(),
            is_instrument: false,
            ..Default::default()
        },
        ScannedPlugin {
            clap_file_path: "/plugins/compressor.clap".to_owned(),
            clap_plugin_id: COMP_ID.to_owned(),
            name: "Resonance Compressor".to_owned(),
            vendor: "Resonance".to_owned(),
            is_instrument: false,
            ..Default::default()
        },
        ScannedPlugin {
            clap_file_path: "/plugins/wavetable.clap".to_owned(),
            clap_plugin_id: SYNTH_ID.to_owned(),
            name: "Resonance Wavetable".to_owned(),
            vendor: "Resonance".to_owned(),
            is_instrument: true,
            ..Default::default()
        },
    ]
}

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    app.test_set_active_project(true);
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_apply_engine_event(AudioEvent::PluginsScanned { plugins: catalog() });
    app
}

fn slot(instance_id: PluginInstanceId, plugin_id: &str, name: &str) -> PluginSlotState {
    PluginSlotState::new(
        instance_id,
        name.to_owned(),
        plugin_id.to_owned(),
        format!("/plugins/{plugin_id}.clap"),
        Vec::new(),
        false,
    )
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

/// The engine's refusal for a plugin it could not instantiate.
fn refuse(app: &mut Resonance, instance_id: PluginInstanceId, plugin_id: &str) {
    app.test_apply_engine_event(AudioEvent::PluginLoadFailed {
        instance_id: Some(instance_id),
        clap_plugin_id: plugin_id.to_owned(),
        clap_file_path: format!("/plugins/{plugin_id}.clap"),
        reason: "Failed to load plugin: no such file".to_owned(),
    });
}

fn chain(app: &mut Resonance) -> PluginParamsView {
    call(
        app,
        "track.plugin_params",
        serde_json::json!({"track_id": TRACK}),
    )
    .result()
    .expect("plugin_params succeeds")
}

/// The chain as `plugin_id` strings, in slot order.
fn ids(view: &PluginParamsView) -> Vec<&str> {
    view.plugins.iter().map(|e| e.plugin_id.as_str()).collect()
}

// ---------------------------------------------------------------------------
// The acceptance path
// ---------------------------------------------------------------------------

/// The DONE-WHEN clause, end to end and in order.
#[test]
fn acceptance_missing_plugin_is_reported_replaced_and_keeps_its_slot() {
    let mut app = app();
    // A project load leaves three optimistic slots; the middle one is a
    // plugin this machine hasn't got.
    app.test_push_track_plugin(TRACK, slot(EQ, EQ_ID, "Resonance EQ"));
    app.test_push_track_plugin(TRACK, slot(VERB, VERB_ID, "MegaVerb"));
    app.test_push_track_plugin(TRACK, slot(COMP, COMP_ID, "Resonance Compressor"));
    refuse(&mut app, VERB, VERB_ID);

    // 1. status reported.
    let before = chain(&mut app);
    assert_eq!(ids(&before), vec![EQ_ID, VERB_ID, COMP_ID]);
    assert_eq!(before.plugins[1].status, PluginSlotStatus::Missing);
    assert_eq!(
        before.plugins[1].unavailable_reason.as_deref(),
        Some("Failed to load plugin: no such file"),
        "and it says WHY — an empty params array said nothing"
    );
    assert_eq!(
        before.plugins[0].status,
        PluginSlotStatus::Loaded,
        "its neighbours are not implicated"
    );

    // 2. replace.
    let result: ReplaceEffectResult = call(
        &mut app,
        "track.replace_effect",
        // A SECOND EQ, deliberately: the compressor already sits after
        // this slot, so replacing with a compressor would leave the same
        // chain whether the replacement landed in the middle or was
        // appended, and the assertion below would prove nothing.
        serde_json::json!({"track_id": TRACK, "slot": 1, "new_plugin_id": EQ_ID}),
    )
    .result()
    .expect("replace succeeds");
    assert_eq!(result.outcome, ReplaceOutcome::Swapped);
    assert_eq!(result.slot, 1);
    assert_eq!(result.plugin_id, EQ_ID);

    // 3. chain order preserved.
    let after = chain(&mut app);
    assert_eq!(
        ids(&after),
        vec![EQ_ID, EQ_ID, COMP_ID],
        "the replacement is in the MIDDLE. remove + add would have put it last, which is a \
         different sound"
    );
    assert_eq!(after.plugins[1].status, PluginSlotStatus::Loaded);
    assert_eq!(after.plugins[1].unavailable_reason, None);
    assert_eq!(
        after.plugins[1].occurrence, 1,
        "and it is the SECOND of the two EQs, because it sits after the other one"
    );
}

/// The recovery that gets the original sound back: name the plugin that
/// is already in the slot, once the catalog has found it again.
#[test]
fn naming_the_same_plugin_relocates_it_rather_than_swapping() {
    let mut app = app();
    app.test_push_track_plugin(TRACK, slot(VERB, EQ_ID, "Resonance EQ"));
    refuse(&mut app, VERB, EQ_ID);

    let result: ReplaceEffectResult = call(
        &mut app,
        "track.replace_effect",
        serde_json::json!({"track_id": TRACK, "slot": 0, "new_plugin_id": EQ_ID}),
    )
    .result()
    .expect("replace succeeds");
    assert_eq!(result.outcome, ReplaceOutcome::Relocated);
    assert_eq!(
        app.test_chain_slots(resonance_app::TestChain::Track(TRACK))[0].0,
        VERB,
        "the instance id is kept — it is what the preserved settings are keyed by"
    );
}

/// Naming the plugin that is already there AND loaded is a no-op, and
/// says so. Silently acking would leave a caller unable to tell it from
/// a real replace.
#[test]
fn replacing_a_loaded_plugin_with_itself_is_a_reported_no_op() {
    let mut app = app();
    app.test_push_track_plugin(TRACK, slot(EQ, EQ_ID, "Resonance EQ"));

    let before = app.revision();
    let result: ReplaceEffectResult = call(
        &mut app,
        "track.replace_effect",
        serde_json::json!({"track_id": TRACK, "slot": 0, "new_plugin_id": EQ_ID}),
    )
    .result()
    .expect("replace succeeds");
    assert_eq!(result.outcome, ReplaceOutcome::AlreadyLoaded);
    assert_eq!(
        app.revision(),
        before,
        "a no-op must not burn an undo entry"
    );
}

// ---------------------------------------------------------------------------
// Addressing and refusals
// ---------------------------------------------------------------------------

/// The instrument slot IS replaceable — unlike remove and move. A synth
/// that will not load is the worst case of this problem, not an exempt
/// one, and swapping it leaves the track with a sound source.
#[test]
fn the_instrument_slot_can_be_replaced() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    app.test_set_active_project(true);
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_apply_engine_event(AudioEvent::PluginsScanned { plugins: catalog() });
    app.test_push_track_plugin(TRACK, slot(VERB, "com.thirdparty.dead-synth", "DeadSynth"));
    app.test_push_track_plugin(TRACK, slot(EQ, EQ_ID, "Resonance EQ"));
    refuse(&mut app, VERB, "com.thirdparty.dead-synth");

    let result: ReplaceEffectResult = call(
        &mut app,
        "track.replace_effect",
        serde_json::json!({"track_id": TRACK, "slot": 0, "new_plugin_id": SYNTH_ID}),
    )
    .result()
    .expect("the instrument slot is replaceable");
    assert_eq!(result.outcome, ReplaceOutcome::Swapped);
    assert_eq!(
        ids(&chain(&mut app)),
        vec![SYNTH_ID, EQ_ID],
        "the new instrument is at the head of the chain, where the instrument belongs"
    );
}

/// A replacement has to be a plugin this machine can actually
/// instantiate. Accepting an unknown id would let a caller "fix" a
/// missing plugin by pointing the slot at a second thing that also does
/// not load.
#[test]
fn an_uninstalled_replacement_is_refused_with_a_route_out() {
    let mut app = app();
    app.test_push_track_plugin(TRACK, slot(VERB, VERB_ID, "MegaVerb"));
    refuse(&mut app, VERB, VERB_ID);

    let error = call(
        &mut app,
        "track.replace_effect",
        serde_json::json!({"track_id": TRACK, "slot": 0, "new_plugin_id": "com.nope.nothing"}),
    )
    .error
    .expect("refused");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(
        error.message.contains("plugins.rescan"),
        "the refusal has to say what to do about it: {}",
        error.message
    );
    assert_eq!(
        chain(&mut app).plugins[0].status,
        PluginSlotStatus::Missing,
        "and the slot is untouched"
    );
}

/// The same slot-or-(id, occurrence) rule the rest of the chain methods
/// follow: exactly one form.
#[test]
fn an_ambiguous_or_absent_address_is_refused() {
    let mut app = app();
    app.test_push_track_plugin(TRACK, slot(EQ, EQ_ID, "Resonance EQ"));

    for params in [
        serde_json::json!({"track_id": TRACK, "new_plugin_id": COMP_ID}),
        serde_json::json!({
            "track_id": TRACK, "slot": 0, "plugin_id": EQ_ID, "new_plugin_id": COMP_ID
        }),
    ] {
        let error = call(&mut app, "track.replace_effect", params.clone())
            .error
            .unwrap_or_else(|| panic!("{params} should be refused"));
        assert_eq!(error.kind(), ErrorKind::InvalidParams, "for {params}");
    }
    assert_eq!(ids(&chain(&mut app)), vec![EQ_ID], "nothing was replaced");
}

// ---------------------------------------------------------------------------
// Bus + master carry the same capability
// ---------------------------------------------------------------------------

/// The dual-surface rule applies across chains too: a missing plugin on
/// a bus or the master is not a lesser problem than one on a track.
#[test]
fn bus_and_master_report_status_and_replace_the_same_way() {
    let mut app = app();
    app.test_add_bus(BUS, "Drums");
    app.test_push_bus_plugin(BUS, slot(VERB, VERB_ID, "MegaVerb"));
    app.test_push_bus_plugin(BUS, slot(EQ, EQ_ID, "Resonance EQ"));
    app.test_push_master_plugin(slot(COMP, VERB_ID, "MegaVerb"));
    app.test_push_master_plugin(slot(200, EQ_ID, "Resonance EQ"));
    refuse(&mut app, VERB, VERB_ID);
    refuse(&mut app, COMP, VERB_ID);

    let bus_view: resonance_control::methods::bus::PluginParamsView = call(
        &mut app,
        "bus.plugin_params",
        serde_json::json!({"bus_id": BUS}),
    )
    .result()
    .expect("bus.plugin_params succeeds");
    assert_eq!(bus_view.plugins[0].status, PluginSlotStatus::Missing);

    let master_view: resonance_control::methods::master::PluginParamsView =
        call(&mut app, "master.plugin_params", serde_json::json!({}))
            .result()
            .expect("master.plugin_params succeeds");
    assert_eq!(master_view.plugins[0].status, PluginSlotStatus::Missing);

    let bus_result: ReplaceEffectResult = call(
        &mut app,
        "bus.replace_effect",
        serde_json::json!({"bus_id": BUS, "slot": 0, "new_plugin_id": COMP_ID}),
    )
    .result()
    .expect("bus.replace_effect succeeds");
    assert_eq!(bus_result.outcome, ReplaceOutcome::Swapped);

    let master_result: ReplaceEffectResult = call(
        &mut app,
        "master.replace_effect",
        serde_json::json!({"slot": 0, "new_plugin_id": COMP_ID}),
    )
    .result()
    .expect("master.replace_effect succeeds");
    assert_eq!(master_result.outcome, ReplaceOutcome::Swapped);

    assert_eq!(
        app.test_chain_slots(resonance_app::TestChain::Bus(BUS))
            .iter()
            .map(|s| s.1.clone())
            .collect::<Vec<_>>(),
        vec![COMP_ID.to_owned(), EQ_ID.to_owned()],
        "bus position kept"
    );
    assert_eq!(
        app.test_chain_slots(resonance_app::TestChain::Master)
            .iter()
            .map(|s| s.1.clone())
            .collect::<Vec<_>>(),
        vec![COMP_ID.to_owned(), EQ_ID.to_owned()],
        "master position kept"
    );
}

/// A replace goes through `update()` like every other control mutation,
/// so Cmd-Z takes it back — including the chain position it restores.
#[test]
fn a_remote_replace_is_undoable() {
    let mut app = app();
    app.test_push_track_plugin(TRACK, slot(EQ, EQ_ID, "Resonance EQ"));
    app.test_push_track_plugin(TRACK, slot(COMP, COMP_ID, "Resonance Compressor"));

    let before = app.revision();
    let _: ReplaceEffectResult = call(
        &mut app,
        "track.replace_effect",
        serde_json::json!({"track_id": TRACK, "slot": 0, "new_plugin_id": COMP_ID}),
    )
    .result()
    .expect("replace succeeds");
    assert_ne!(
        app.revision(),
        before,
        "a real replace records an undoable transaction"
    );
}
