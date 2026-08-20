//! `plugins.rescan` — finding a plugin installed while the app is
//! running (ba todo #1307, finding X10).
//!
//! `ScanPlugins` was sent exactly once, at startup, so a plugin
//! installed afterwards did not exist as far as Resonance was concerned:
//! not in the add-plugin menus, not in `plugins.catalog`, not to
//! `track.add_instrument`. The only remedy was a restart.
//!
//! These drive the whole path from the app's side: the control method
//! and the GUI message reach the same engine command, the refreshed
//! catalog is readable immediately after the engine echo, a rescan
//! leaves running plugins alone, and a bundle that refuses to load is
//! reported instead of silently missing.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::{Message, PluginMessage};
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::{
    AudioCommand, AudioEvent, ParamInfo, PluginScanFailure, ScannedPlugin, TrackType,
};
use resonance_control::methods::plugins::PluginCatalog;
use resonance_control::methods::track::PluginParamsView;
use resonance_control::{MutationAck, Request, Response};

const TRACK: u64 = 1;
const REVERB: u64 = 20;

fn scanned(id: &str, name: &str, is_instrument: bool) -> ScannedPlugin {
    ScannedPlugin {
        clap_file_path: format!("/plugins/{id}.clap"),
        clap_plugin_id: id.to_owned(),
        name: name.to_owned(),
        vendor: "Resonance".to_owned(),
        is_instrument,
        ..Default::default()
    }
}

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-plugins-rescan.rprj"));
    app.test_add_track(TRACK, TrackType::Instrument);
    // The startup scan: one effect, and nothing else on the machine yet.
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![scanned("com.resonance.reverb", "Resonance Reverb", false)],
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

fn catalog(app: &mut Resonance) -> PluginCatalog {
    roundtrip(app, Request::without_params(2, "plugins.catalog"))
        .result()
        .expect("plugins.catalog succeeds")
}

fn rescan(app: &mut Resonance) -> Response {
    call(app, "plugins.rescan", serde_json::json!({}))
}

fn drain(rx: &resonance_audio::__test_support::Receiver<AudioCommand>) -> Vec<AudioCommand> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

// ---------------------------------------------------------------------------
// The control method
// ---------------------------------------------------------------------------

#[test]
fn a_rescan_asks_the_engine_to_look_again() {
    let mut app = app();
    let rx = app.test_capture_engine();
    let _: MutationAck = rescan(&mut app).result().expect("plugins.rescan succeeds");

    let commands = drain(&rx);
    assert!(
        commands
            .iter()
            .any(|c| matches!(c, AudioCommand::RescanPlugins)),
        "expected a RescanPlugins command, got {commands:?}"
    );
    // NOT the startup scan: that one drops every instantiated plugin.
    assert!(
        !commands
            .iter()
            .any(|c| matches!(c, AudioCommand::ScanPlugins)),
        "a live rescan must never send the destructive startup scan"
    );
}

#[test]
fn a_newly_installed_plugin_reaches_the_catalog_without_a_restart() {
    let mut app = app();
    assert_eq!(catalog(&mut app).plugins.len(), 1);

    let _: MutationAck = rescan(&mut app).result().expect("plugins.rescan succeeds");
    // The engine answers with the whole catalog, old bundles included.
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![
            scanned("com.resonance.reverb", "Resonance Reverb", false),
            scanned("com.resonance.wavetable", "Resonance Wavetable", true),
        ],
    });

    let catalog = catalog(&mut app);
    let ids: Vec<&str> = catalog.plugins.iter().map(|p| p.id.as_str()).collect();
    assert!(
        ids.contains(&"com.resonance.wavetable"),
        "the plugin installed since startup must be reachable now: {ids:?}"
    );
    assert!(ids.contains(&"com.resonance.reverb"), "and the old one stays");
    assert!(catalog.scan_failures.is_empty());
}

#[test]
fn the_gui_message_and_the_control_method_run_the_same_scan() {
    // The dual-surface rule (ba doc #276): a human's Settings button and
    // an agent's `plugins.rescan` must be the same capability, not two.
    let mut app = app();
    let rx = app.test_capture_engine();
    let _ = app.update(Message::Plugin(PluginMessage::RescanPlugins));
    assert!(drain(&rx)
        .iter()
        .any(|c| matches!(c, AudioCommand::RescanPlugins)));
}

#[test]
fn a_rescan_works_with_no_project_open() {
    // A catalog is a fact about the machine, not about a project — an
    // agent should be able to install a plugin and find it before it
    // creates anything (the same reason `plugins.catalog` is not gated).
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    let response = rescan(&mut app);
    assert!(
        response.error.is_none(),
        "plugins.rescan must not be project-gated: {:?}",
        response.error
    );
}

// ---------------------------------------------------------------------------
// Not disturbing what is already running
// ---------------------------------------------------------------------------

#[test]
fn a_rescan_leaves_an_instantiated_plugin_alone() {
    let mut app = app();
    // A plugin on a track, with its parameters mirrored.
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id: TRACK,
        instance_id: REVERB,
        plugin_name: "Resonance Reverb".to_owned(),
        clap_plugin_id: "com.resonance.reverb".to_owned(),
        clap_file_path: "/plugins/com.resonance.reverb.clap".to_owned(),
        params: vec![ParamInfo {
            id: 1,
            name: "Mix".to_owned(),
            min_value: 0.0,
            max_value: 1.0,
            default_value: 0.3,
            current_value: 0.8,
            ..Default::default()
        }],
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    });

    let _: MutationAck = rescan(&mut app).result().expect("plugins.rescan succeeds");
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![
            scanned("com.resonance.reverb", "Resonance Reverb", false),
            scanned("com.resonance.wavetable", "Resonance Wavetable", true),
        ],
    });

    // The chain is untouched — same plugin, same dialled-in value. (The
    // engine side cannot disturb it either: `rescan_plugins` is not
    // handed the instance map at all.)
    let view: PluginParamsView = call(
        &mut app,
        "track.plugin_params",
        serde_json::json!({"track_id": TRACK}),
    )
    .result()
    .expect("track.plugin_params succeeds");
    assert_eq!(view.plugins.len(), 1);
    assert_eq!(view.plugins[0].plugin_id, "com.resonance.reverb");
    assert_eq!(view.plugins[0].params[0].value, 0.8);
}

// ---------------------------------------------------------------------------
// Failures
// ---------------------------------------------------------------------------

#[test]
fn a_bundle_that_will_not_load_is_reported_not_swallowed() {
    let mut app = app();
    let _: MutationAck = rescan(&mut app).result().expect("plugins.rescan succeeds");
    app.test_apply_engine_event(AudioEvent::PluginScanFailed {
        failures: vec![PluginScanFailure {
            path: "/plugins/broken.clap".to_owned(),
            reason: "missing clap_entry symbol".to_owned(),
        }],
    });
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![scanned("com.resonance.reverb", "Resonance Reverb", false)],
    });

    let catalog = catalog(&mut app);
    assert_eq!(catalog.scan_failures.len(), 1);
    assert_eq!(catalog.scan_failures[0].path, "/plugins/broken.clap");
    assert!(
        catalog.scan_failures[0].reason.contains("clap_entry"),
        "the loader's own words reach the client: {:?}",
        catalog.scan_failures[0].reason
    );
    // A broken bundle is not a catalog entry.
    assert_eq!(catalog.plugins.len(), 1);
}

#[test]
fn a_clean_rescan_clears_the_previous_runs_failures() {
    let mut app = app();
    let _: MutationAck = rescan(&mut app).result().expect("first rescan");
    app.test_apply_engine_event(AudioEvent::PluginScanFailed {
        failures: vec![PluginScanFailure {
            path: "/plugins/broken.clap".to_owned(),
            reason: "missing clap_entry symbol".to_owned(),
        }],
    });
    assert_eq!(catalog(&mut app).scan_failures.len(), 1);

    // The user fixes the install and scans again: reporting the old
    // failure now would send them after a problem that is gone.
    let _: MutationAck = rescan(&mut app).result().expect("second rescan");
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![scanned("com.resonance.reverb", "Resonance Reverb", false)],
    });
    assert!(catalog(&mut app).scan_failures.is_empty());
}
