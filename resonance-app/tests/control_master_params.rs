//! The master chain's *parameters* over the control API:
//! `master.plugin_params` / `master.set_plugin_param` /
//! `master.move_effect`.
//!
//! Before these the master chain was write-only: `master.add_effect`
//! loaded a limiter and it then sat at its defaults forever, with no
//! method able to read or change a single value. That is not a cosmetic
//! gap — the master fader is post-FX, so it cannot drive the chain
//! either, and the only remaining route (scaling every track fader) does
//! not limit anything: it passes the overshoot straight through. A mix
//! could be balanced correctly over MCP and then never brought to a
//! release level by any sequence of calls.
//!
//! Master is a singleton, so these mirror the `bus.*` methods with
//! `bus_id` dropped — same addressing, same errors, same acks. The
//! sequence that motivated them (add the mastering plugin, engage its
//! limiter, set a ceiling, drive it with input trim, read it all back)
//! is `the_whole_limiter_sequence_runs_over_the_control_api_alone` at
//! the bottom.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::{AudioCommand, AudioEvent, ParamInfo, ScannedPlugin};
use resonance_control::methods::master::PluginParamsView;
use resonance_control::methods::track::{AddPluginResult, PluginKind};
use resonance_control::{ErrorKind, MutationAck, Request, Response};

fn app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-master-params.rprj"));
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![
            ScannedPlugin {
                clap_file_path: "/plugins/mastering.clap".to_owned(),
                clap_plugin_id: "com.resonance.mastering".to_owned(),
                name: "Resonance Mastering".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: false,
            },
            ScannedPlugin {
                clap_file_path: "/plugins/eq.clap".to_owned(),
                clap_plugin_id: "com.resonance.eq".to_owned(),
                name: "Resonance EQ".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: false,
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

fn add(app: &mut Resonance, plugin_id: &str) -> AddPluginResult {
    call(
        app,
        "master.add_effect",
        serde_json::json!({"plugin_id": plugin_id}),
    )
    .result()
    .expect("master.add_effect succeeds")
}

fn chain(app: &mut Resonance) -> PluginParamsView {
    roundtrip(app, Request::without_params(7, "master.plugin_params"))
        .result()
        .expect("master.plugin_params succeeds")
}

fn ids(view: &PluginParamsView) -> Vec<&str> {
    view.plugins
        .iter()
        .map(|p| p.plugin_id.rsplit('.').next().unwrap_or(&p.plugin_id))
        .collect()
}

fn value_of(view: &PluginParamsView, slot: usize, param: &str) -> f64 {
    view.plugins[slot]
        .params
        .iter()
        .find(|p| p.name == param)
        .unwrap_or_else(|| panic!("{param} is reported"))
        .value
}

/// The instance id the app hinted on the most recent master add.
fn hinted(rx: &crossbeam_channel::Receiver<AudioCommand>) -> u64 {
    std::iter::from_fn(|| rx.try_recv().ok())
        .find_map(|c| match c {
            AudioCommand::AddPluginToMaster { id_hint, .. } => Some(id_hint),
            _ => None,
        })
        .expect("an AddPluginToMaster reached the engine")
        .expect("the control path hints the instance id")
}

/// Add an effect and play the engine's echo back, which is what supplies
/// the parameter list. The params mirror the real
/// `com.resonance.mastering` shape closely enough to exercise the
/// bounds rules: a switch, a ceiling in dBTP, and an f32-declared
/// minimum.
fn add_and_echo(app: &mut Resonance, plugin_id: &str) -> u64 {
    let rx = app.test_capture_engine();
    add(app, plugin_id);
    let instance_id = hinted(&rx);
    app.test_apply_engine_event(AudioEvent::MasterPluginAdded {
        instance_id,
        plugin_name: plugin_id.to_owned(),
        clap_plugin_id: plugin_id.to_owned(),
        clap_file_path: format!("/plugins/{plugin_id}.clap"),
        params: vec![
            ParamInfo {
                id: 1,
                name: "Limiter On".to_owned(),
                min_value: 0.0,
                max_value: 1.0,
                default_value: 0.0,
                current_value: 0.0,
            },
            ParamInfo {
                id: 2,
                name: "Ceiling".to_owned(),
                min_value: -6.0,
                max_value: 0.0,
                default_value: -0.3,
                current_value: -0.3,
            },
            ParamInfo {
                id: 3,
                name: "Input Trim".to_owned(),
                min_value: -24.0,
                max_value: 24.0,
                default_value: 0.0,
                current_value: 0.0,
            },
            ParamInfo {
                id: 4,
                name: "Release".to_owned(),
                // The f32-declared minimum, as todo #1235 describes it.
                min_value: 0.1f32 as f64,
                max_value: 500.0,
                default_value: 50.0,
                current_value: 50.0,
            },
        ],
        has_gui: false,
    });
    instance_id
}

// ---------------------------------------------------------------------------
// plugin_params
// ---------------------------------------------------------------------------

#[test]
fn an_added_effect_is_addressable_in_the_same_cycle() {
    let mut app = app();
    let result = add(&mut app, "com.resonance.mastering");
    assert_eq!(result.plugin_id, "com.resonance.mastering");
    assert_eq!((result.occurrence, result.slot), (0, 0));

    // No engine echo: the same read-your-writes rule track.add_effect and
    // bus.add_effect follow.
    let view = chain(&mut app);
    assert_eq!(ids(&view), vec!["mastering"]);
    assert_eq!(view.plugins[0].slot, 0);
    assert_eq!(
        view.plugins[0].kind,
        PluginKind::Effect,
        "the master has no instrument slot"
    );
}

#[test]
fn the_engine_echo_fills_the_placeholder_instead_of_duplicating_it() {
    let mut app = app();
    add_and_echo(&mut app, "com.resonance.mastering");

    let view = chain(&mut app);
    assert_eq!(view.plugins.len(), 1, "one slot, not two");
    assert_eq!(view.plugins[0].params.len(), 4, "the echo supplies params");
    // The full parameter shape master.summary cannot report.
    let ceiling = &view.plugins[0]
        .params
        .iter()
        .find(|p| p.name == "Ceiling")
        .expect("Ceiling is reported");
    assert_eq!((ceiling.min, ceiling.max, ceiling.default), (-6.0, 0.0, -0.3));
    assert_eq!(ceiling.id, 2);
}

#[test]
fn plugin_params_filters_to_one_effect_and_reports_misses() {
    let mut app = app();
    add(&mut app, "com.resonance.eq");
    add(&mut app, "com.resonance.mastering");

    let view: PluginParamsView = call(
        &mut app,
        "master.plugin_params",
        serde_json::json!({"plugin_id": "com.resonance.mastering"}),
    )
    .result()
    .expect("filtering succeeds");
    assert_eq!(ids(&view), vec!["mastering"]);
    assert_eq!(view.plugins[0].slot, 1, "the slot is the real chain position");

    let error = call(
        &mut app,
        "master.plugin_params",
        serde_json::json!({"plugin_id": "com.resonance.reverb"}),
    )
    .error
    .expect("an effect not on the master is not_found");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(error.message.contains("carries"), "{}", error.message);
    assert!(error.message.contains("0:com.resonance.eq"), "{}", error.message);
}

#[test]
fn occurrence_picks_the_right_copy_of_a_repeated_effect() {
    let mut app = app();
    add(&mut app, "com.resonance.eq");
    add(&mut app, "com.resonance.eq");

    let view: PluginParamsView = call(
        &mut app,
        "master.plugin_params",
        serde_json::json!({"plugin_id": "com.resonance.eq", "occurrence": 1}),
    )
    .result()
    .expect("succeeds");
    assert_eq!(view.plugins.len(), 1);
    assert_eq!((view.plugins[0].slot, view.plugins[0].occurrence), (1, 1));
}

/// Read-only, but it describes the OPEN project's master — with nothing
/// open the honest answer is `busy`, not an empty chain.
#[test]
fn the_new_methods_need_an_open_project() {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(false);

    for (method, params) in [
        ("master.plugin_params", serde_json::json!({})),
        (
            "master.set_plugin_param",
            serde_json::json!({"param": "Ceiling", "value": -1.0}),
        ),
        ("master.move_effect", serde_json::json!({"slot": 0, "to_slot": 1})),
    ] {
        let error = call(&mut app, method, params)
            .error
            .unwrap_or_else(|| panic!("{method} should be busy without a project"));
        assert_eq!(error.kind(), ErrorKind::Busy, "for {method}");
    }
}

// ---------------------------------------------------------------------------
// set_plugin_param
// ---------------------------------------------------------------------------

#[test]
fn a_master_plugins_parameters_can_be_set_and_read_back() {
    let mut app = app();
    let instance_id = add_and_echo(&mut app, "com.resonance.mastering");

    let rx = app.test_capture_engine();
    let _: MutationAck = call(
        &mut app,
        "master.set_plugin_param",
        serde_json::json!({
            "plugin_id": "com.resonance.mastering",
            "param": "Ceiling",
            "value": -1.0,
        }),
    )
    .result()
    .expect("master.set_plugin_param succeeds");
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(
            c,
            AudioCommand::SetPluginParam { instance_id: i, param_id: 2, value }
                if i == instance_id && value == -1.0
        )),
        "the value must reach the DSP"
    );

    assert_eq!(value_of(&chain(&mut app), 0, "Ceiling"), -1.0);
}

/// Parameter names are matched case-insensitively, and a numeric id as a
/// string addresses the same parameter.
#[test]
fn a_parameter_can_be_named_case_insensitively_or_by_id() {
    let mut app = app();
    add_and_echo(&mut app, "com.resonance.mastering");

    let _: MutationAck = call(
        &mut app,
        "master.set_plugin_param",
        serde_json::json!({"param": "ceiling", "value": -2.0}),
    )
    .result()
    .expect("a lowercased name resolves");
    assert_eq!(value_of(&chain(&mut app), 0, "Ceiling"), -2.0);

    let _: MutationAck = call(
        &mut app,
        "master.set_plugin_param",
        serde_json::json!({"param": "3", "value": 4.5}),
    )
    .result()
    .expect("a numeric id resolves");
    assert_eq!(value_of(&chain(&mut app), 0, "Input Trim"), 4.5);
}

#[test]
fn the_f32_bound_tolerance_is_shared_with_the_track_surface() {
    let mut app = app();
    add_and_echo(&mut app, "com.resonance.mastering");

    // Release's minimum is the f32-widened 0.1 (todo #1235); the tidy
    // decimal must be accepted here exactly as on a track.
    let rx = app.test_capture_engine();
    let _: MutationAck = call(
        &mut app,
        "master.set_plugin_param",
        serde_json::json!({"param": "Release", "value": 0.1}),
    )
    .result()
    .expect("0.1 rounds onto the declared minimum");
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(
            c,
            AudioCommand::SetPluginParam { param_id: 4, value, .. }
                if value == 0.1f32 as f64
        )),
        "and is clamped up to the plugin's true minimum"
    );

    let error = call(
        &mut app,
        "master.set_plugin_param",
        serde_json::json!({"param": "Release", "value": 900.0}),
    )
    .error
    .expect("a genuinely out-of-range value is still refused");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert!(
        error.message.contains("500"),
        "the refusal reports the range: {}",
        error.message
    );
}

#[test]
fn setting_a_param_before_the_echo_says_initializing() {
    let mut app = app();
    add(&mut app, "com.resonance.mastering");

    let error = call(
        &mut app,
        "master.set_plugin_param",
        serde_json::json!({"param": "Ceiling", "value": -1.0}),
    )
    .error
    .expect("the parameter list has not arrived yet");
    assert_eq!(error.kind(), ErrorKind::Busy, "{}", error.message);
    assert!(error.message.contains("still initializing"), "{}", error.message);
}

#[test]
fn addressing_a_plugin_that_is_not_on_the_master_lists_what_it_carries() {
    let mut app = app();
    add_and_echo(&mut app, "com.resonance.mastering");

    let error = call(
        &mut app,
        "master.set_plugin_param",
        serde_json::json!({"plugin_id": "com.resonance.eq", "param": "Ceiling", "value": -1.0}),
    )
    .error
    .expect("an effect not on the master is not_found");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(
        error.message.contains("carries") && error.message.contains("0:com.resonance.mastering"),
        "the error must name the real chain: {}",
        error.message
    );

    let error = call(
        &mut app,
        "master.set_plugin_param",
        serde_json::json!({"param": "Loudness", "value": 1.0}),
    )
    .error
    .expect("an unknown parameter is not_found");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(error.message.contains("Ceiling"), "the error lists the real parameters: {}", error.message);
}

#[test]
fn setting_a_param_on_an_empty_master_points_at_add_effect() {
    let mut app = app();
    let error = call(
        &mut app,
        "master.set_plugin_param",
        serde_json::json!({"param": "Ceiling", "value": -1.0}),
    )
    .error
    .expect("there is nothing to set");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(error.message.contains("master.add_effect"), "{}", error.message);
}

/// A value set over the control API has to reach the saved project, or
/// the mastering settle survives only until the next reload.
#[test]
fn parameter_values_set_remotely_land_in_the_saved_project() {
    let mut app = app();
    add_and_echo(&mut app, "com.resonance.mastering");
    for (param, value) in [("Limiter On", 1.0), ("Ceiling", -1.0), ("Input Trim", 6.0)] {
        let _: MutationAck = call(
            &mut app,
            "master.set_plugin_param",
            serde_json::json!({"param": param, "value": value}),
        )
        .result()
        .expect("master.set_plugin_param succeeds");
    }

    let file = app.test_build_project_file();
    assert_eq!(file.master_plugins.len(), 1);
    let saved: std::collections::BTreeMap<&str, f64> = file.master_plugins[0]
        .params
        .iter()
        .map(|p| (p.name.as_str(), p.value))
        .collect();
    assert_eq!(saved.get("Limiter On"), Some(&1.0));
    assert_eq!(saved.get("Ceiling"), Some(&-1.0));
    assert_eq!(saved.get("Input Trim"), Some(&6.0));
    assert!(
        !saved.contains_key("Release"),
        "an untouched parameter is left at the plugin's own default"
    );
}

// ---------------------------------------------------------------------------
// move_effect
// ---------------------------------------------------------------------------

#[test]
fn move_effect_reorders_the_chain_and_reaches_the_engine() {
    let mut app = app();
    let mastering = add_and_echo(&mut app, "com.resonance.mastering");
    add(&mut app, "com.resonance.eq");
    assert_eq!(ids(&chain(&mut app)), vec!["mastering", "eq"]);

    // A limiter belongs last: put the EQ in front of it.
    let rx = app.test_capture_engine();
    let _: MutationAck = call(
        &mut app,
        "master.move_effect",
        serde_json::json!({"plugin_id": "com.resonance.mastering", "to_slot": 1}),
    )
    .result()
    .expect("master.move_effect succeeds");
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(
            c,
            AudioCommand::MovePluginInMaster { instance_id, to_index: 1 }
                if instance_id == mastering
        )),
        "the engine must be told to reorder its own chain"
    );
    assert_eq!(
        ids(&chain(&mut app)),
        vec!["eq", "mastering"],
        "and the app reads its own write back immediately"
    );

    // The engine's echo replays the same move and changes nothing.
    app.test_apply_engine_event(AudioEvent::MasterPluginMoved {
        instance_id: mastering,
        to_index: 1,
    });
    assert_eq!(ids(&chain(&mut app)), vec!["eq", "mastering"]);
}

#[test]
fn move_effect_clamps_past_the_end_and_no_ops_in_place() {
    let mut app = app();
    add(&mut app, "com.resonance.mastering");
    add(&mut app, "com.resonance.eq");

    let _: MutationAck = call(
        &mut app,
        "master.move_effect",
        serde_json::json!({"slot": 0, "to_slot": 99}),
    )
    .result()
    .expect("past the end clamps to the end");
    assert_eq!(ids(&chain(&mut app)), vec!["eq", "mastering"]);

    let rx = app.test_capture_engine();
    let before = app.revision();
    let _: MutationAck = call(
        &mut app,
        "master.move_effect",
        serde_json::json!({"slot": 1, "to_slot": 1}),
    )
    .result()
    .expect("moving an effect where it already sits is accepted");
    assert_eq!(ids(&chain(&mut app)), vec!["eq", "mastering"]);
    assert_eq!(app.revision(), before, "a no-op records nothing");
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok())
            .all(|c| !matches!(c, AudioCommand::MovePluginInMaster { .. })),
        "and nothing reaches the engine"
    );
}

#[test]
fn move_effect_addressing_must_be_unambiguous() {
    let mut app = app();
    add(&mut app, "com.resonance.eq");

    for params in [
        serde_json::json!({"to_slot": 0}),
        serde_json::json!({"slot": 0, "plugin_id": "com.resonance.eq", "to_slot": 0}),
    ] {
        let error = call(&mut app, "master.move_effect", params.clone())
            .error
            .unwrap_or_else(|| panic!("{params} should be rejected"));
        assert_eq!(error.kind(), ErrorKind::InvalidParams, "for {params}");
    }

    let error = call(
        &mut app,
        "master.move_effect",
        serde_json::json!({"slot": 5, "to_slot": 0}),
    )
    .error
    .expect("a slot that is not there is not_found");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(error.message.contains("com.resonance.eq"), "{}", error.message);
}

#[test]
fn a_reorder_is_undoable_and_persisted_in_order() {
    let mut app = app();
    add(&mut app, "com.resonance.mastering");
    add(&mut app, "com.resonance.eq");
    let before = app.revision();

    let _: MutationAck = call(
        &mut app,
        "master.move_effect",
        serde_json::json!({"slot": 1, "to_slot": 0}),
    )
    .result()
    .expect("succeeds");
    assert_eq!(app.revision(), before + 1, "the reorder is a committed edit");

    // Chain order is what serialization writes, so a remote reorder has
    // to survive a save.
    let file = app.test_build_project_file();
    let saved: Vec<&str> = file
        .master_plugins
        .iter()
        .map(|p| p.clap_plugin_id.rsplit('.').next().unwrap_or(""))
        .collect();
    assert_eq!(saved, vec!["eq", "mastering"]);

    let rx = app.test_capture_engine();
    let _ = app.update(Message::Undo);
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(c, AudioCommand::ClearAll)),
        "undo must find an entry and start restoring the pre-reorder snapshot"
    );
}

// ---------------------------------------------------------------------------
// The definition of done, end to end
// ---------------------------------------------------------------------------

/// Load the mastering plugin on the master, engage its limiter, set a
/// ceiling, drive it with input trim, and read every value back — over
/// the control API alone. This is the sequence that was impossible: the
/// add always worked, and everything after it had no method to call.
///
/// The loudness leg of it (`meter.measure` reporting the result) needs a
/// real render and lives outside a unit test; what is pinned here is
/// that every parameter the settle needs is reachable and sticks.
#[test]
fn the_whole_limiter_sequence_runs_over_the_control_api_alone() {
    let mut app = app();

    // 1. Put the mastering plugin on the master.
    let rx = app.test_capture_engine();
    let added = add(&mut app, "com.resonance.mastering");
    let instance_id = hinted(&rx);
    assert_eq!(added.slot, 0);

    // 2. Its parameter list arrives with the engine echo.
    app.test_apply_engine_event(AudioEvent::MasterPluginAdded {
        instance_id,
        plugin_name: "Resonance Mastering".to_owned(),
        clap_plugin_id: "com.resonance.mastering".to_owned(),
        clap_file_path: "/plugins/mastering.clap".to_owned(),
        params: vec![
            ParamInfo {
                id: 1,
                name: "Limiter On".to_owned(),
                min_value: 0.0,
                max_value: 1.0,
                default_value: 0.0,
                current_value: 0.0,
            },
            ParamInfo {
                id: 2,
                name: "Ceiling".to_owned(),
                min_value: -6.0,
                max_value: 0.0,
                default_value: -0.3,
                current_value: -0.3,
            },
            ParamInfo {
                id: 3,
                name: "Input Trim".to_owned(),
                min_value: -24.0,
                max_value: 24.0,
                default_value: 0.0,
                current_value: 0.0,
            },
        ],
        has_gui: false,
    });

    // 3. The stages default to OFF, so an unconfigured mastering plugin
    //    measures the same as no plugin at all — engage the limiter.
    let rx = app.test_capture_engine();
    for (param, value) in [("Limiter On", 1.0), ("Ceiling", -1.0), ("Input Trim", 7.5)] {
        let _: MutationAck = call(
            &mut app,
            "master.set_plugin_param",
            serde_json::json!({
                "plugin_id": added.plugin_id,
                "occurrence": added.occurrence,
                "param": param,
                "value": value,
            }),
        )
        .result()
        .unwrap_or_else(|e| panic!("setting {param} succeeds: {e:?}"));
    }

    // 4. Every one of them reached the DSP.
    let sets: Vec<(u32, f64)> = std::iter::from_fn(|| rx.try_recv().ok())
        .filter_map(|c| match c {
            AudioCommand::SetPluginParam {
                instance_id: i,
                param_id,
                value,
            } if i == instance_id => Some((param_id, value)),
            _ => None,
        })
        .collect();
    assert_eq!(sets, vec![(1, 1.0), (2, -1.0), (3, 7.5)]);

    // 5. And reads back changed, which is the whole point: master.summary
    //    reports identity only.
    let view = chain(&mut app);
    assert_eq!(value_of(&view, 0, "Limiter On"), 1.0);
    assert_eq!(value_of(&view, 0, "Ceiling"), -1.0);
    assert_eq!(value_of(&view, 0, "Input Trim"), 7.5);
}
