//! The character plugin (`com.resonance.color`, warmth-width-depth.md
//! §6.1) is addressable like every other first-party effect: the app
//! keeps no list of built-in plugins — the catalog is whatever the
//! scanner found, and `bundle.sh` bundles every `plugins/<name>/` — so
//! what has to hold is that, once scanned, it lists as an effect, loads
//! on a track and on a bus (§4 puts it on busses first), and takes its
//! parameters by name on both.
//!
//! The engine side of the same round trip — the real bundle loading on a
//! track and a bus — is `resonance-audio/tests/clap_host/color_plugin_loads.rs`.

use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::{ChainOwner, AudioCommand, AudioEvent, ParamInfo, ScannedPlugin, TrackType};
use resonance_control::methods::bus::PluginParamsView as BusChain;
use resonance_control::methods::plugins::{PluginCatalog, CATALOG};
use resonance_control::methods::track::{AddPluginResult, PluginKind, PluginParamsView};
use resonance_control::{MutationAck, Request};
use crate::common::{call, roundtrip};

const TRACK: u64 = 1;
const COLOR: &str = "com.resonance.color";
const COLOR_PATH: &str = "/plugins/resonance-color.clap";

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-color.rprj"));
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![ScannedPlugin {
            clap_file_path: COLOR_PATH.to_owned(),
            clap_plugin_id: COLOR.to_owned(),
            name: "Resonance Color".to_owned(),
            vendor: "Resonance".to_owned(),
            is_instrument: false,
            ..Default::default()
        }],
    });
    app
}

/// Two of the plugin's parameters as the CLAP bridge reports them.
fn params() -> Vec<ParamInfo> {
    vec![
        ParamInfo {
            id: 11,
            name: "Mode".to_owned(),
            min_value: 0.0,
            max_value: 4.0,
            default_value: 0.0,
            current_value: 0.0,
            stepped: true,
            choices: ["Tube", "Tape", "Transformer", "Console", "Warm"]
                .map(str::to_owned)
                .to_vec(),
            ..Default::default()
        },
        ParamInfo {
            id: 12,
            name: "Drive".to_owned(),
            min_value: 0.0,
            max_value: 1.0,
            default_value: 0.35,
            current_value: 0.35,
            ..Default::default()
        },
    ]
}

#[test]
fn the_catalog_lists_it_as_an_effect() {
    let mut app = app();
    let catalog: PluginCatalog = roundtrip(&mut app, Request::without_params(1, CATALOG))
        .result()
        .expect("plugins.catalog");
    let entry = catalog
        .plugins
        .iter()
        .find(|p| p.id == COLOR)
        .expect("the scanned plugin is listed");
    assert_eq!(entry.kind, PluginKind::Effect, "Color is an effect");
    assert_eq!(entry.name, "Resonance Color");
}

#[test]
fn it_loads_on_a_track_and_takes_params_by_name() {
    let mut app = app();
    let rx = app.test_capture_engine();
    let added: AddPluginResult = call(
        &mut app,
        "track.add_effect",
        serde_json::json!({"track_id": TRACK, "plugin_id": COLOR}),
    )
    .result()
    .expect("track.add_effect");
    assert_eq!(added.plugin_id, COLOR);
    let instance_id = std::iter::from_fn(|| rx.try_recv().ok())
        .find_map(|c| match c {
            AudioCommand::AddPlugin {
                id,
                clap_plugin_id,
                clap_file_path,
                ..
            } => {
                assert_eq!(clap_plugin_id, COLOR);
                assert_eq!(clap_file_path, COLOR_PATH, "resolved to the scanned bundle");
                Some(id)
            }
            _ => None,
        })
        .expect("an AddPlugin reached the engine");

    app.test_apply_engine_event(AudioEvent::PluginAdded {
        owner: ChainOwner::Track(TRACK),
        instance_id,
        plugin_name: "Resonance Color".to_owned(),
        clap_plugin_id: COLOR.to_owned(),
        clap_file_path: COLOR_PATH.to_owned(),
        params: params(),
        has_gui: true,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Out".to_owned()],
    });

    let rx = app.test_capture_engine();
    let _: MutationAck = call(
        &mut app,
        "track.set_plugin_param",
        serde_json::json!({"track_id": TRACK, "plugin_id": COLOR, "param": "Drive", "value": 0.6}),
    )
    .result()
    .expect("track.set_plugin_param");
    assert!(std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(
        c,
        AudioCommand::SetPluginParam { instance_id: i, param_id: 12, value } if i == instance_id && value == 0.6
    )));

    let view: PluginParamsView = call(
        &mut app,
        "track.plugin_params",
        serde_json::json!({"track_id": TRACK}),
    )
    .result()
    .expect("track.plugin_params");
    assert_eq!(view.plugins.len(), 1);
    assert_eq!(view.plugins[0].plugin_id, COLOR);
    assert_eq!(view.plugins[0].kind, PluginKind::Effect);
}

#[test]
fn it_loads_on_a_bus_and_takes_params_by_name() {
    let mut app = app();
    let bus: resonance_control::methods::bus::CreateResult =
        call(&mut app, "bus.create", serde_json::json!({"name": "Drum Bus"}))
            .result()
            .expect("bus.create");
    let bus_id = bus.bus_id.0;

    let rx = app.test_capture_engine();
    let added: AddPluginResult = call(
        &mut app,
        "bus.add_effect",
        serde_json::json!({"bus_id": bus_id, "plugin_id": COLOR}),
    )
    .result()
    .expect("bus.add_effect");
    assert_eq!(added.plugin_id, COLOR);
    let instance_id = std::iter::from_fn(|| rx.try_recv().ok())
        .find_map(|c| match c {
            AudioCommand::AddPlugin {
                id,
                clap_plugin_id,
                clap_file_path,
                ..
            } => {
                assert_eq!(clap_plugin_id, COLOR);
                assert_eq!(clap_file_path, COLOR_PATH);
                Some(id)
            }
            _ => None,
        })
        .expect("a bus AddPlugin reached the engine");

    app.test_apply_engine_event(AudioEvent::PluginAdded {
        owner: ChainOwner::Bus(bus_id),
        instance_id,
        plugin_name: "Resonance Color".to_owned(),
        clap_plugin_id: COLOR.to_owned(),
        clap_file_path: COLOR_PATH.to_owned(),
        params: params(),
        has_gui: true,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: Vec::new(),
    });

    let rx = app.test_capture_engine();
    // A choice parameter by its label, the way a skill names a mode.
    let _: MutationAck = call(
        &mut app,
        "bus.set_plugin_param",
        serde_json::json!({"bus_id": bus_id, "plugin_id": COLOR, "param": "Mode", "value": 1.0}),
    )
    .result()
    .expect("bus.set_plugin_param");
    assert!(std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(
        c,
        AudioCommand::SetPluginParam { instance_id: i, param_id: 11, value } if i == instance_id && value == 1.0
    )));

    let view: BusChain = call(&mut app, "bus.plugin_params", serde_json::json!({"bus_id": bus_id}))
        .result()
        .expect("bus.plugin_params");
    assert_eq!(view.plugins.len(), 1);
    assert_eq!(view.plugins[0].plugin_id, COLOR);
    assert_eq!(view.plugins[0].params.len(), 2);
}
