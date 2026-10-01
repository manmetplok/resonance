//! A plugin's own word on a parameter — CLAP `IS_AUTOMATABLE` and
//! `IS_READONLY` — is honoured by every host surface
//! (drums-plugin-rework.md §5.1, §5.4):
//!
//! - `plugin_params` reports both;
//! - `set_plugin_param` refuses a read-only output (the plugin drops the
//!   write, so an ack, an undo entry and a mirrored value would all lie),
//!   and still sets a not-automatable selector;
//! - `automation.*` and the GUI refuse a NEW lane on a not-automatable
//!   param, and the mixer's lane picker does not offer one.

use crate::common::call;
use resonance_app::message::{AutomationMessage, Message, PluginMessage};
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::{AudioCommand, AudioEvent, ParamInfo, TrackType};
use resonance_common::AutomationTarget;
use resonance_control::methods::track::PluginParamsView;
use resonance_control::Response;
use serde_json::json;

const SR: u32 = 48_000;
const TRACK: u64 = 1;
const DRUMS: u64 = 20;
const GAIN: u32 = 1;
const KIT: u32 = 2;
const PROGRESS: u32 = 3;

fn params() -> Vec<ParamInfo> {
    vec![
        ParamInfo {
            id: GAIN,
            name: "Gain".to_owned(),
            max_value: 1.0,
            default_value: 0.5,
            current_value: 0.5,
            ..Default::default()
        },
        ParamInfo {
            id: KIT,
            name: "Kit".to_owned(),
            min_value: -1.0,
            max_value: 999.0,
            default_value: -1.0,
            current_value: -1.0,
            stepped: true,
            automatable: false,
            state_excluded: true,
            ..Default::default()
        },
        ParamInfo {
            id: PROGRESS,
            name: "Kit Load Progress".to_owned(),
            max_value: 1.0,
            current_value: 0.4,
            automatable: false,
            read_only: true,
            state_excluded: true,
            ..Default::default()
        },
    ]
}

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-param-flags.rprj"));
    app.test_set_sample_rate(SR);
    app.test_set_flat_tempo(120.0);
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id: TRACK,
        instance_id: DRUMS,
        plugin_name: "Drums".to_owned(),
        clap_plugin_id: "com.resonance.drums".to_owned(),
        clap_file_path: "/plugins/drums.clap".to_owned(),
        params: params(),
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    });
    app
}

fn error_message(response: Response) -> String {
    response
        .result::<serde_json::Value>()
        .expect_err("the call is refused")
        .message
}

fn set_param(app: &mut Resonance, param: &str, value: f64) -> Response {
    call(
        app,
        "track.set_plugin_param",
        json!({"track_id": TRACK, "param": param, "value": value}),
    )
}

fn set_lane(app: &mut Resonance, param: &str) -> Response {
    call(
        app,
        "automation.set_lane",
        json!({
            "track_id": TRACK,
            "param": param,
            "points": [{"position": {"bar": 1}, "value": 0.25}],
        }),
    )
}

#[test]
fn plugin_params_reports_read_only_and_not_automatable() {
    let mut app = app();
    let view: PluginParamsView = call(&mut app, "track.plugin_params", json!({"track_id": TRACK}))
        .result()
        .expect("track.plugin_params succeeds");
    let params = &view.plugins[0].params;
    let by_id = |id| params.iter().find(|p| p.id == id).expect("param");
    assert!(by_id(GAIN).automatable && !by_id(GAIN).read_only);
    assert!(!by_id(KIT).automatable && !by_id(KIT).read_only);
    assert!(!by_id(PROGRESS).automatable && by_id(PROGRESS).read_only);

    // On the wire, `automatable` appears only where it is false.
    let raw: serde_json::Value = call(&mut app, "track.plugin_params", json!({"track_id": TRACK}))
        .result()
        .expect("succeeds");
    let wire = &raw["plugins"][0]["params"];
    assert!(wire[0].get("automatable").is_none(), "{wire}");
    assert_eq!(wire[1]["automatable"], json!(false), "{wire}");
    assert_eq!(wire[2]["read_only"], json!(true), "{wire}");
}

#[test]
fn set_plugin_param_refuses_a_read_only_output() {
    let mut app = app();
    let rx = app.test_capture_engine();
    let message = error_message(set_param(&mut app, "Kit Load Progress", 1.0));
    assert!(message.contains("read-only"), "{message}");
    while let Ok(cmd) = rx.try_recv() {
        assert!(
            !matches!(cmd, AudioCommand::SetPluginParam { .. }),
            "nothing reaches the engine: {cmd:?}"
        );
    }
    assert_eq!(app.test_plugin_param(DRUMS, PROGRESS), Some(0.4));
    assert!(!app.test_can_undo(), "no undo step for a refused write");
}

#[test]
fn a_not_automatable_selector_is_still_settable() {
    let mut app = app();
    let response = set_param(&mut app, "Kit", 3.0);
    assert!(response.error.is_none(), "{:?}", response.error);
    assert_eq!(app.test_plugin_param(DRUMS, KIT), Some(3.0));
}

/// The GUI's path into the same write is refused too: the generic panel
/// draws no slider for an output, but nothing else may write it either.
#[test]
fn the_gui_message_does_not_write_a_read_only_output() {
    let mut app = app();
    let rx = app.test_capture_engine();
    let _ = app.update(Message::Plugin(PluginMessage::SetPluginParam(
        DRUMS, PROGRESS, 1.0,
    )));
    while let Ok(cmd) = rx.try_recv() {
        assert!(!matches!(cmd, AudioCommand::SetPluginParam { .. }), "{cmd:?}");
    }
    assert_eq!(app.test_plugin_param(DRUMS, PROGRESS), Some(0.4));
}

#[test]
fn automation_refuses_a_lane_on_a_not_automatable_param() {
    let mut app = app();
    let message = error_message(set_lane(&mut app, "Kit"));
    assert!(message.contains("cannot be automated"), "{message}");
    let message = error_message(set_lane(&mut app, "Kit Load Progress"));
    assert!(message.contains("read-only"), "{message}");
    // An ordinary param still takes one.
    let response = set_lane(&mut app, "Gain");
    assert!(response.error.is_none(), "{:?}", response.error);
}

#[test]
fn the_gui_adds_no_lane_and_offers_none_for_a_not_automatable_param() {
    let mut app = app();
    let target = |param_id| AutomationTarget::PluginParam {
        instance: DRUMS,
        param_id,
    };
    let _ = app.update(Message::Automation(AutomationMessage::AddLane(target(KIT))));
    let _ = app.update(Message::Automation(AutomationMessage::AddLane(target(GAIN))));
    assert!(!app.test_automation().lanes.contains_key(&target(KIT)));
    assert!(app.test_automation().lanes.contains_key(&target(GAIN)));

    let labels = app.test_automation_picker_labels(TRACK);
    assert!(labels.iter().any(|l| l == "Drums: Gain"), "{labels:?}");
    assert!(!labels.iter().any(|l| l.contains("Kit")), "{labels:?}");
}
