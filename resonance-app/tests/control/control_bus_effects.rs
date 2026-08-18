//! The bus insert chain over the control API (ba doc #273, todo #1237):
//! `bus.add_effect` / `remove_effect` / `move_effect` / `set_fx_bypass`
//! / `plugin_params` / `set_plugin_param`.
//!
//! This is what makes a group mixable rather than merely summable. The
//! mixer already ran a bus's plugin chain over the accumulated buffer,
//! with bus PDC, fader and the master sum after it, and sub-tracks
//! already honoured `TrackOutput::Bus` — so it worked in the GUI all
//! along and was unreachable over MCP only because no method could put a
//! plugin on a bus.
//!
//! The end-to-end sequence the todo's definition of done names — create
//! a bus, route sub-tracks into it, add a compressor, set its
//! parameters, read them back, reorder, remove — is
//! `the_whole_drum_bus_sequence_runs_over_the_control_api_alone` at the
//! bottom of this file.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance};
use resonance_audio::types::{AudioCommand, AudioEvent, ParamInfo, ScannedPlugin, TrackType};
use resonance_control::methods::bus::PluginParamsView;
use resonance_control::methods::track::{AddPluginResult, PluginKind};
use resonance_control::{ErrorKind, MutationAck, Request, Response};

const KICK: u64 = 1;
const SNARE: u64 = 2;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-bus-effects.rprj"));
    app.test_add_track(KICK, TrackType::Instrument);
    app.test_add_track(SNARE, TrackType::Instrument);
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![
            ScannedPlugin {
                clap_file_path: "/plugins/wavetable.clap".to_owned(),
                clap_plugin_id: "com.resonance.wavetable".to_owned(),
                name: "Resonance Wavetable".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: true,
            ..Default::default()
},
            ScannedPlugin {
                clap_file_path: "/plugins/compressor.clap".to_owned(),
                clap_plugin_id: "com.resonance.compressor".to_owned(),
                name: "Resonance Compressor".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: false,
            ..Default::default()
},
            ScannedPlugin {
                clap_file_path: "/plugins/eq.clap".to_owned(),
                clap_plugin_id: "com.resonance.eq".to_owned(),
                name: "Resonance EQ".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: false,
            ..Default::default()
},
        ],
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
    roundtrip(app, Request::new(1, method, &params).expect("params serialize"))
}

fn create_bus(app: &mut Resonance) -> u64 {
    let result: resonance_control::methods::bus::CreateResult =
        call(app, "bus.create", serde_json::json!({"name": "Drum Bus"}))
            .result()
            .expect("bus.create succeeds");
    result.bus_id.0
}

fn add(app: &mut Resonance, bus_id: u64, plugin_id: &str) -> AddPluginResult {
    call(
        app,
        "bus.add_effect",
        serde_json::json!({"bus_id": bus_id, "plugin_id": plugin_id}),
    )
    .result()
    .expect("bus.add_effect succeeds")
}

fn chain(app: &mut Resonance, bus_id: u64) -> PluginParamsView {
    call(
        app,
        "bus.plugin_params",
        serde_json::json!({"bus_id": bus_id}),
    )
    .result()
    .expect("bus.plugin_params succeeds")
}

fn ids(view: &PluginParamsView) -> Vec<&str> {
    view.plugins
        .iter()
        .map(|p| p.plugin_id.rsplit('.').next().unwrap_or(&p.plugin_id))
        .collect()
}

/// The instance id the app hinted on the most recent bus add.
fn hinted(rx: &crossbeam_channel::Receiver<AudioCommand>) -> u64 {
    std::iter::from_fn(|| rx.try_recv().ok())
        .find_map(|c| match c {
            AudioCommand::AddPluginToBus { id_hint, .. } => Some(id_hint),
            _ => None,
        })
        .expect("an AddPluginToBus reached the engine")
        .expect("the control path hints the instance id")
}

/// The engine echo that supplies a bus plugin's parameter list.
fn echo(app: &mut Resonance, bus_id: u64, instance_id: u64, plugin_id: &str) {
    app.test_apply_engine_event(AudioEvent::BusPluginAdded {
        bus_id,
        instance_id,
        plugin_name: plugin_id.to_owned(),
        clap_plugin_id: plugin_id.to_owned(),
        clap_file_path: format!("/plugins/{plugin_id}.clap"),
        params: vec![
            ParamInfo {
                id: 1,
                name: "Threshold".to_owned(),
                min_value: -60.0,
                max_value: 0.0,
                default_value: -12.0,
                current_value: -12.0,
                ..Default::default()
            },
            ParamInfo {
                id: 2,
                name: "Attack".to_owned(),
                // The f32-declared minimum, as todo #1235 describes it.
                min_value: 0.1f32 as f64,
                max_value: 200.0,
                default_value: 10.0,
                current_value: 10.0,
                ..Default::default()
            },
        ],
        has_gui: false,
        has_sidechain_input: false,
    });
}

// ---------------------------------------------------------------------------
// add / plugin_params
// ---------------------------------------------------------------------------

#[test]
fn an_added_effect_is_visible_on_the_bus_in_the_same_cycle() {
    let mut app = app();
    let bus_id = create_bus(&mut app);
    let result = add(&mut app, bus_id, "com.resonance.compressor");
    assert_eq!(result.plugin_id, "com.resonance.compressor");
    assert_eq!((result.occurrence, result.slot), (0, 0));

    // No engine echo: the same read-your-writes rule track.add_effect
    // follows since todo #1234.
    let view = chain(&mut app, bus_id);
    assert_eq!(ids(&view), vec!["compressor"]);
    assert_eq!(view.bus_id.0, bus_id);
    assert_eq!(
        view.plugins[0].kind,
        PluginKind::Effect,
        "a bus has no instrument slot"
    );
}

#[test]
fn adding_the_same_effect_twice_numbers_the_occurrences() {
    let mut app = app();
    let bus_id = create_bus(&mut app);
    let first = add(&mut app, bus_id, "com.resonance.eq");
    let second = add(&mut app, bus_id, "com.resonance.eq");
    assert_eq!((first.occurrence, first.slot), (0, 0));
    assert_eq!((second.occurrence, second.slot), (1, 1));
    let view = chain(&mut app, bus_id);
    assert_eq!(
        view.plugins.iter().map(|p| p.occurrence).collect::<Vec<_>>(),
        vec![0, 1]
    );
}

#[test]
fn the_engine_echo_fills_the_placeholder_instead_of_duplicating_it() {
    let mut app = app();
    let bus_id = create_bus(&mut app);
    let rx = app.test_capture_engine();
    add(&mut app, bus_id, "com.resonance.compressor");
    let instance_id = hinted(&rx);
    echo(&mut app, bus_id, instance_id, "com.resonance.compressor");

    let view = chain(&mut app, bus_id);
    assert_eq!(view.plugins.len(), 1, "one slot, not two");
    assert_eq!(view.plugins[0].params.len(), 2, "the echo supplies params");
}

#[test]
fn an_instrument_is_refused_and_an_unknown_id_lists_the_valid_ones() {
    let mut app = app();
    let bus_id = create_bus(&mut app);

    let error = call(
        &mut app,
        "bus.add_effect",
        serde_json::json!({"bus_id": bus_id, "plugin_id": "com.resonance.wavetable"}),
    )
    .error
    .expect("an instrument on a bus is refused");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert!(
        error.message.contains("track.add_instrument"),
        "the error points at where instruments belong: {}",
        error.message
    );

    let error = call(
        &mut app,
        "bus.add_effect",
        serde_json::json!({"bus_id": bus_id, "plugin_id": "com.resonance.nope"}),
    )
    .error
    .expect("an unknown id is refused");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(error.message.contains("com.resonance.compressor"), "{}", error.message);

    assert!(chain(&mut app, bus_id).plugins.is_empty(), "nothing landed");
}

#[test]
fn an_unknown_bus_is_not_found_on_every_method() {
    let mut app = app();
    for (method, params) in [
        (
            "bus.add_effect",
            serde_json::json!({"bus_id": 4242, "plugin_id": "com.resonance.eq"}),
        ),
        ("bus.remove_effect", serde_json::json!({"bus_id": 4242, "slot": 0})),
        (
            "bus.move_effect",
            serde_json::json!({"bus_id": 4242, "slot": 0, "to_slot": 1}),
        ),
        (
            "bus.set_fx_bypass",
            serde_json::json!({"bus_id": 4242, "bypassed": true}),
        ),
        ("bus.plugin_params", serde_json::json!({"bus_id": 4242})),
        (
            "bus.set_plugin_param",
            serde_json::json!({"bus_id": 4242, "param": "Threshold", "value": -10.0}),
        ),
    ] {
        let error = call(&mut app, method, params)
            .error
            .unwrap_or_else(|| panic!("{method} must reject an unknown bus"));
        assert_eq!(error.kind(), ErrorKind::NotFound, "for {method}");
    }
}

// ---------------------------------------------------------------------------
// remove
// ---------------------------------------------------------------------------

#[test]
fn removing_by_slot_and_by_occurrence_target_the_right_instance() {
    let mut app = app();
    let bus_id = create_bus(&mut app);
    let rx = app.test_capture_engine();
    add(&mut app, bus_id, "com.resonance.eq");
    let first_eq = hinted(&rx);
    add(&mut app, bus_id, "com.resonance.eq");
    let second_eq = hinted(&rx);
    add(&mut app, bus_id, "com.resonance.compressor");

    // Occurrence 1 is the SECOND EQ.
    let rx = app.test_capture_engine();
    let _: MutationAck = call(
        &mut app,
        "bus.remove_effect",
        serde_json::json!({
            "bus_id": bus_id, "plugin_id": "com.resonance.eq", "occurrence": 1
        }),
    )
    .result()
    .expect("succeeds");
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(
            c,
            AudioCommand::RemovePluginFromBus { instance_id, .. } if instance_id == second_eq
        )),
        "the engine must be told to unload the second EQ"
    );
    app.test_apply_engine_event(AudioEvent::BusPluginRemoved {
        bus_id,
        instance_id: second_eq,
    });
    assert_eq!(ids(&chain(&mut app, bus_id)), vec!["eq", "compressor"]);

    // ...and slot 0 is the first.
    let rx = app.test_capture_engine();
    let _: MutationAck = call(
        &mut app,
        "bus.remove_effect",
        serde_json::json!({"bus_id": bus_id, "slot": 0}),
    )
    .result()
    .expect("succeeds");
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(
            c,
            AudioCommand::RemovePluginFromBus { instance_id, .. } if instance_id == first_eq
        )),
        "slot 0 is the surviving EQ"
    );
}

#[test]
fn addressing_must_be_unambiguous_and_misses_are_not_found() {
    let mut app = app();
    let bus_id = create_bus(&mut app);
    add(&mut app, bus_id, "com.resonance.eq");

    for params in [
        serde_json::json!({"bus_id": bus_id}),
        serde_json::json!({"bus_id": bus_id, "slot": 0, "plugin_id": "com.resonance.eq"}),
    ] {
        let error = call(&mut app, "bus.remove_effect", params.clone())
            .error
            .unwrap_or_else(|| panic!("{params} must be rejected"));
        assert_eq!(error.kind(), ErrorKind::InvalidParams, "for {params}");
    }

    let error = call(
        &mut app,
        "bus.remove_effect",
        serde_json::json!({"bus_id": bus_id, "slot": 9}),
    )
    .error
    .expect("missing slot rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(error.message.contains("com.resonance.eq"), "{}", error.message);
}

// ---------------------------------------------------------------------------
// move
// ---------------------------------------------------------------------------

#[test]
fn moving_an_effect_reorders_the_bus_chain_and_the_app_mirror() {
    let mut app = app();
    let bus_id = create_bus(&mut app);
    add(&mut app, bus_id, "com.resonance.eq");
    add(&mut app, bus_id, "com.resonance.compressor");
    assert_eq!(ids(&chain(&mut app, bus_id)), vec!["eq", "compressor"]);

    let rx = app.test_capture_engine();
    let _: MutationAck = call(
        &mut app,
        "bus.move_effect",
        serde_json::json!({
            "bus_id": bus_id, "plugin_id": "com.resonance.compressor", "to_slot": 0
        }),
    )
    .result()
    .expect("bus.move_effect succeeds");
    let moved = std::iter::from_fn(|| rx.try_recv().ok())
        .find_map(|c| match c {
            AudioCommand::MovePluginInBus {
                instance_id,
                to_index,
                ..
            } => Some((instance_id, to_index)),
            _ => None,
        })
        .expect("the engine must be told to reorder its own chain");
    assert_eq!(moved.1, 0);
    assert_eq!(
        ids(&chain(&mut app, bus_id)),
        vec!["compressor", "eq"],
        "and the app mirror reads back in the same cycle"
    );

    // The echo replays a move already applied — it must be a no-op.
    app.test_apply_engine_event(AudioEvent::BusPluginMoved {
        bus_id,
        instance_id: moved.0,
        to_index: 0,
    });
    assert_eq!(ids(&chain(&mut app, bus_id)), vec!["compressor", "eq"]);
}

#[test]
fn a_move_past_the_end_clamps_and_a_no_op_move_records_nothing() {
    let mut app = app();
    let bus_id = create_bus(&mut app);
    add(&mut app, bus_id, "com.resonance.eq");
    add(&mut app, bus_id, "com.resonance.compressor");

    let _: MutationAck = call(
        &mut app,
        "bus.move_effect",
        serde_json::json!({"bus_id": bus_id, "slot": 0, "to_slot": 99}),
    )
    .result()
    .expect("past the end means the end");
    assert_eq!(ids(&chain(&mut app, bus_id)), vec!["compressor", "eq"]);

    let before = app.revision();
    let rx = app.test_capture_engine();
    let _: MutationAck = call(
        &mut app,
        "bus.move_effect",
        serde_json::json!({"bus_id": bus_id, "slot": 1, "to_slot": 1}),
    )
    .result()
    .expect("a no-op move is accepted");
    assert_eq!(app.revision(), before, "no undo entry for a no-op");
    assert!(
        !std::iter::from_fn(|| rx.try_recv().ok())
            .any(|c| matches!(c, AudioCommand::MovePluginInBus { .. })),
        "and no engine command"
    );
}

// ---------------------------------------------------------------------------
// set_fx_bypass — absolute, not a toggle
// ---------------------------------------------------------------------------

/// The engine commands a `bus.set_fx_bypass` call produced, as the
/// `bypassed` values they carried.
fn bypass_commands(rx: &crossbeam_channel::Receiver<AudioCommand>) -> Vec<bool> {
    std::iter::from_fn(|| rx.try_recv().ok())
        .filter_map(|c| match c {
            AudioCommand::SetBusFxBypass { bypassed, .. } => Some(bypassed),
            _ => None,
        })
        .collect()
}

#[test]
fn set_fx_bypass_is_idempotent_across_two_identical_calls() {
    let mut app = app();
    let bus_id = create_bus(&mut app);
    add(&mut app, bus_id, "com.resonance.compressor");

    let bypass = |app: &mut Resonance, value: bool| -> MutationAck {
        call(
            app,
            "bus.set_fx_bypass",
            serde_json::json!({"bus_id": bus_id, "bypassed": value}),
        )
        .result()
        .expect("bus.set_fx_bypass succeeds")
    };

    let rx = app.test_capture_engine();
    let _ = bypass(&mut app, true);
    assert_eq!(
        bypass_commands(&rx),
        vec![true],
        "the first call bypasses the chain"
    );

    // The underlying `BusMessage::ToggleBusFxBypass` FLIPS, so a client
    // that retried a request whose reply it never saw would otherwise
    // turn the group's processing back on. It must not.
    let before = app.revision();
    let rx = app.test_capture_engine();
    let _ = bypass(&mut app, true);
    assert!(
        bypass_commands(&rx).is_empty(),
        "an identical retry must send NO command — a toggle here would un-bypass the group"
    );
    assert_eq!(app.revision(), before, "and record no undo entry");

    // ...and `false` really does re-engage it.
    let rx = app.test_capture_engine();
    let _ = bypass(&mut app, false);
    assert_eq!(bypass_commands(&rx), vec![false]);

    let rx = app.test_capture_engine();
    let _ = bypass(&mut app, false);
    assert!(
        bypass_commands(&rx).is_empty(),
        "and setting the state it is already in is a no-op in both directions"
    );
}

// ---------------------------------------------------------------------------
// set_plugin_param
// ---------------------------------------------------------------------------

#[test]
fn a_bus_plugins_parameters_can_be_set_and_read_back() {
    let mut app = app();
    let bus_id = create_bus(&mut app);
    let rx = app.test_capture_engine();
    add(&mut app, bus_id, "com.resonance.compressor");
    let instance_id = hinted(&rx);
    echo(&mut app, bus_id, instance_id, "com.resonance.compressor");

    let rx = app.test_capture_engine();
    let _: MutationAck = call(
        &mut app,
        "bus.set_plugin_param",
        serde_json::json!({
            "bus_id": bus_id,
            "plugin_id": "com.resonance.compressor",
            "param": "Threshold",
            "value": -18.0,
        }),
    )
    .result()
    .expect("bus.set_plugin_param succeeds");
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(
            c,
            AudioCommand::SetPluginParam { instance_id: i, param_id: 1, value }
                if i == instance_id && value == -18.0
        )),
        "the value must reach the DSP"
    );

    let view = chain(&mut app, bus_id);
    let threshold = view.plugins[0]
        .params
        .iter()
        .find(|p| p.name == "Threshold")
        .expect("Threshold is reported");
    assert_eq!(threshold.value, -18.0, "and be readable back");
}

#[test]
fn the_f32_bound_tolerance_is_shared_with_the_track_surface() {
    let mut app = app();
    let bus_id = create_bus(&mut app);
    let rx = app.test_capture_engine();
    add(&mut app, bus_id, "com.resonance.compressor");
    let instance_id = hinted(&rx);
    echo(&mut app, bus_id, instance_id, "com.resonance.compressor");

    // Attack's minimum is the f32-widened 0.1 (todo #1235); the tidy
    // decimal must be accepted here exactly as on a track.
    let rx = app.test_capture_engine();
    let _: MutationAck = call(
        &mut app,
        "bus.set_plugin_param",
        serde_json::json!({"bus_id": bus_id, "param": "Attack", "value": 0.1}),
    )
    .result()
    .expect("0.1 rounds onto the declared minimum");
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(
            c,
            AudioCommand::SetPluginParam { param_id: 2, value, .. }
                if value == 0.1f32 as f64
        )),
        "and is clamped up to the plugin's true minimum"
    );

    let error = call(
        &mut app,
        "bus.set_plugin_param",
        serde_json::json!({"bus_id": bus_id, "param": "Attack", "value": 500.0}),
    )
    .error
    .expect("a genuinely out-of-range value is still refused");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
}

#[test]
fn setting_a_param_before_the_echo_says_initializing() {
    let mut app = app();
    let bus_id = create_bus(&mut app);
    add(&mut app, bus_id, "com.resonance.compressor");

    let error = call(
        &mut app,
        "bus.set_plugin_param",
        serde_json::json!({"bus_id": bus_id, "param": "Threshold", "value": -18.0}),
    )
    .error
    .expect("the parameter list has not arrived yet");
    assert_eq!(error.kind(), ErrorKind::Busy, "{}", error.message);
    assert!(error.message.contains("still initializing"), "{}", error.message);
}

#[test]
fn plugin_params_can_filter_to_one_effect_and_reports_misses() {
    let mut app = app();
    let bus_id = create_bus(&mut app);
    add(&mut app, bus_id, "com.resonance.eq");
    add(&mut app, bus_id, "com.resonance.compressor");

    let view: PluginParamsView = call(
        &mut app,
        "bus.plugin_params",
        serde_json::json!({"bus_id": bus_id, "plugin_id": "com.resonance.compressor"}),
    )
    .result()
    .expect("filtering succeeds");
    assert_eq!(ids(&view), vec!["compressor"]);

    let error = call(
        &mut app,
        "bus.plugin_params",
        serde_json::json!({"bus_id": bus_id, "plugin_id": "com.resonance.reverb"}),
    )
    .error
    .expect("an effect not on the bus is not_found");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(error.message.contains("0:com.resonance.eq"), "{}", error.message);
}

// ---------------------------------------------------------------------------
// The definition of done, end to end
// ---------------------------------------------------------------------------

/// Create a bus, route the kit's tracks into it, add a compressor, set
/// its parameters, read them back, reorder and remove it — over the
/// control API alone, with no GUI interaction.
#[test]
fn the_whole_drum_bus_sequence_runs_over_the_control_api_alone() {
    let mut app = app();

    // 1. Create the bus.
    let bus_id = create_bus(&mut app);

    // 2. Route the kit's tracks into it.
    for track in [KICK, SNARE] {
        let _: MutationAck = call(
            &mut app,
            "track.set_output",
            serde_json::json!({"track_id": track, "output": {"bus_id": bus_id}}),
        )
        .result()
        .expect("track.set_output succeeds");
    }

    // 3. Add the compressor to the BUS (not to either track — neither is
    //    individually loud enough to trigger it).
    let rx = app.test_capture_engine();
    let compressor = add(&mut app, bus_id, "com.resonance.compressor");
    let instance_id = hinted(&rx);
    echo(&mut app, bus_id, instance_id, "com.resonance.compressor");

    // 4. Set a parameter, addressed by the handle the add returned.
    let _: MutationAck = call(
        &mut app,
        "bus.set_plugin_param",
        serde_json::json!({
            "bus_id": bus_id,
            "plugin_id": compressor.plugin_id,
            "occurrence": compressor.occurrence,
            "param": "Threshold",
            "value": -20.0,
        }),
    )
    .result()
    .expect("bus.set_plugin_param succeeds");

    // 5. Read it back.
    let view = chain(&mut app, bus_id);
    assert_eq!(
        view.plugins[0]
            .params
            .iter()
            .find(|p| p.name == "Threshold")
            .expect("Threshold")
            .value,
        -20.0
    );

    // 6. Reorder: put an EQ in front of it.
    add(&mut app, bus_id, "com.resonance.eq");
    let _: MutationAck = call(
        &mut app,
        "bus.move_effect",
        serde_json::json!({"bus_id": bus_id, "plugin_id": "com.resonance.eq", "to_slot": 0}),
    )
    .result()
    .expect("bus.move_effect succeeds");
    assert_eq!(ids(&chain(&mut app, bus_id)), vec!["eq", "compressor"]);

    // 7. Remove the compressor.
    let _: MutationAck = call(
        &mut app,
        "bus.remove_effect",
        serde_json::json!({"bus_id": bus_id, "plugin_id": "com.resonance.compressor"}),
    )
    .result()
    .expect("bus.remove_effect succeeds");
    app.test_apply_engine_event(AudioEvent::BusPluginRemoved {
        bus_id,
        instance_id,
    });
    assert_eq!(ids(&chain(&mut app, bus_id)), vec!["eq"]);
}
