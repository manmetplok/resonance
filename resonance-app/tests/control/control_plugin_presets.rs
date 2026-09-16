//! `track.plugin_presets` / `.load_plugin_preset` / `.save_plugin_preset`
//! over the control API (ba todo #1333, finding X1).
//!
//! The MCP half of the preset work: an agent mixing over the wire had to
//! set every parameter individually to recall a sound, because presets
//! were a GUI-only capability.
//!
//! Everything here points the preset directory at a private temp root, so
//! no test reads or writes the real
//! `~/.local/share/resonance/plugin-presets`.

use resonance_app::state::{PluginSlotState, ViewMode};
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, ParamInfo, ScannedPlugin, TrackType};
use resonance_control::ids::TrackId as ProtoTrackId;
use resonance_control::methods::plugin_preset::{PluginPresetSource, PluginPresetsView};
use resonance_control::methods::track as track_proto;
use resonance_control::{Request, Response};
use crate::common::roundtrip;

const TRACK: u64 = 80;
const INSTANCE: u64 = 900;
const PLUGIN_ID: &str = "com.resonance.test-synth";

/// The plugin's parameters, and the string ids its presets are keyed by.
/// The CLAP numeric id is `stable_hash(string_id)` — that is how the
/// bridge derives it, and how the app maps a preset key onto a parameter.
const CUTOFF: &str = "cutoff";
const DRIVE: &str = "drive";

fn clap_id(string_id: &str) -> u32 {
    resonance_plugin::stable_hash(string_id)
}

/// A private preset root, removed when the test ends.
struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(tag: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "resonance-control-presets-{}-{tag}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn params_at_defaults() -> Vec<ParamInfo> {
    vec![
        ParamInfo {
            id: clap_id(CUTOFF),
            name: "Cutoff".to_owned(),
            min_value: 20.0,
            max_value: 20_000.0,
            default_value: 8000.0,
            current_value: 8000.0,
            ..Default::default()
        },
        ParamInfo {
            id: clap_id(DRIVE),
            name: "Drive".to_owned(),
            min_value: 0.0,
            max_value: 1.0,
            default_value: 0.0,
            current_value: 0.0,
            ..Default::default()
        },
    ]
}

/// A scanned plugin carrying two factory presets, exactly as the scan
/// reports one that exported `resonance_factory_presets`.
fn scanned_with_factory_bank() -> ScannedPlugin {
    ScannedPlugin {
        clap_file_path: "/nonexistent/test-synth.clap".to_owned(),
        clap_plugin_id: PLUGIN_ID.to_owned(),
        name: "Test Synth".to_owned(),
        vendor: "Resonance".to_owned(),
        is_instrument: true,
        factory_presets: vec![
            (
                "Dark".to_owned(),
                format!(r#"{{"version":1,"params":{{"{CUTOFF}":400.0,"{DRIVE}":0.1}}}}"#),
            ),
            (
                "Bright".to_owned(),
                format!(r#"{{"version":1,"params":{{"{CUTOFF}":12000.0,"{DRIVE}":0.8}}}}"#),
            ),
        ],
    }
}

fn app_with_plugin(root: &TempRoot) -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_plugin_preset_root(root.0.clone());
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![scanned_with_factory_bank()],
    });
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_push_track_plugin(
        TRACK,
        PluginSlotState::new(
            INSTANCE,
            "Test Synth".to_owned(),
            PLUGIN_ID.to_owned(),
            "/nonexistent/test-synth.clap".to_owned(),
            params_at_defaults(),
            false,
        ),
    );
    app
}

fn call<T: serde::Serialize>(app: &mut Resonance, method: &str, params: &T) -> Response {
    roundtrip(app, Request::new(1, method, params).expect("params serialize"))
}

fn presets(app: &mut Resonance) -> PluginPresetsView {
    let response = call(
        app,
        track_proto::PLUGIN_PRESETS,
        &track_proto::PluginPresetsParams {
            track_id: ProtoTrackId(TRACK),
            plugin_id: Some(PLUGIN_ID.to_owned()),
            occurrence: None,
        },
    );
    serde_json::from_value(response.result.expect("plugin_presets should succeed"))
        .expect("a PluginPresetsView")
}

fn param_value(app: &mut Resonance, string_id: &str) -> f64 {
    app.test_plugin_param(INSTANCE, clap_id(string_id))
        .expect("the parameter exists")
}

/// The factory bank is listed without anything having been saved — which
/// is the point: a fresh install has factory presets and no user ones, so
/// a user-presets-only API would answer an empty list.
#[test]
fn factory_presets_are_listed_before_anything_is_saved() {
    let root = TempRoot::new("list");
    let mut app = app_with_plugin(&root);

    let view = presets(&mut app);
    assert_eq!(view.plugin_id, PLUGIN_ID);
    let names: Vec<&str> = view.presets.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["Dark", "Bright"], "in the plugin's own order");
    assert!(view
        .presets
        .iter()
        .all(|p| p.source == PluginPresetSource::Factory));
}

/// Recalling a preset moves the parameters it names, and the app's own
/// mirror moves with them — so `track.plugin_params` reports the sound
/// that is actually playing.
#[test]
fn loading_a_preset_moves_the_parameters_and_the_mirror() {
    let root = TempRoot::new("load");
    let mut app = app_with_plugin(&root);
    assert_eq!(param_value(&mut app, CUTOFF), 8000.0);

    let response = call(
        &mut app,
        track_proto::LOAD_PLUGIN_PRESET,
        &track_proto::LoadPluginPresetParams {
            track_id: ProtoTrackId(TRACK),
            plugin_id: Some(PLUGIN_ID.to_owned()),
            occurrence: None,
            preset: "Bright".to_owned(),
            source: None,
        },
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    assert_eq!(param_value(&mut app, CUTOFF), 12000.0);
    assert_eq!(param_value(&mut app, DRIVE), 0.8);
}

/// A recall is one gesture, so it is one entry in the undo history —
/// not one per parameter it moved.
#[test]
fn a_recall_is_a_single_undo_entry() {
    let root = TempRoot::new("undo");
    let mut app = app_with_plugin(&root);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-plugin-presets.rprj"));
    let before = app.test_undo_history().test_undo_entries().len();

    let _ = call(
        &mut app,
        track_proto::LOAD_PLUGIN_PRESET,
        &track_proto::LoadPluginPresetParams {
            track_id: ProtoTrackId(TRACK),
            plugin_id: Some(PLUGIN_ID.to_owned()),
            occurrence: None,
            preset: "Dark".to_owned(),
            source: None,
        },
    );

    assert_eq!(
        app.test_undo_history().test_undo_entries().len(),
        before + 1,
        "a preset recall moved two parameters and must still be one undo entry"
    );
}

/// An unknown preset is refused with the ones that would have worked,
/// rather than silently doing nothing.
#[test]
fn an_unknown_preset_is_refused_with_the_names_that_exist() {
    let root = TempRoot::new("unknown");
    let mut app = app_with_plugin(&root);

    let response = call(
        &mut app,
        track_proto::LOAD_PLUGIN_PRESET,
        &track_proto::LoadPluginPresetParams {
            track_id: ProtoTrackId(TRACK),
            plugin_id: Some(PLUGIN_ID.to_owned()),
            occurrence: None,
            preset: "Nope".to_owned(),
            source: None,
        },
    );
    let error = response.error.expect("an unknown preset must be refused");
    assert!(error.message.contains("Dark"), "{}", error.message);
    assert!(error.message.contains("Bright"), "{}", error.message);
    assert_eq!(param_value(&mut app, CUTOFF), 8000.0, "nothing should have moved");
}

/// save -> list: the saved preset appears alongside the factory bank,
/// tagged as the user's. The write happens on the engine's state echo, so
/// this drives that echo the way the engine would.
#[test]
fn saving_adds_a_user_preset_the_list_then_reports() {
    let root = TempRoot::new("save");
    let mut app = app_with_plugin(&root);

    let response = call(
        &mut app,
        track_proto::SAVE_PLUGIN_PRESET,
        &track_proto::SavePluginPresetParams {
            track_id: ProtoTrackId(TRACK),
            plugin_id: Some(PLUGIN_ID.to_owned()),
            occurrence: None,
            name: "My Sound".to_owned(),
            overwrite: false,
        },
    );
    assert!(response.error.is_none(), "{:?}", response.error);

    // Nothing is on disk yet: the request only armed the capture.
    assert_eq!(
        presets(&mut app).presets.len(),
        2,
        "the preset must not appear before the plugin has handed its state back"
    );

    // The engine answers with the plugin's own state document.
    app.test_apply_engine_event(AudioEvent::PluginStateSaved {
        instance_id: INSTANCE,
        data: format!(r#"{{"version":1,"params":{{"{CUTOFF}":1234.0,"{DRIVE}":0.25}}}}"#)
            .into_bytes(),
    });

    let view = presets(&mut app);
    let user: Vec<&str> = view
        .presets
        .iter()
        .filter(|p| p.source == PluginPresetSource::User)
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(user, vec!["My Sound"]);
}

/// ...and the round trip closes: what was saved recalls the sound it was
/// saved from.
#[test]
fn a_saved_preset_recalls_what_it_captured() {
    let root = TempRoot::new("roundtrip");
    let mut app = app_with_plugin(&root);

    let _ = call(
        &mut app,
        track_proto::SAVE_PLUGIN_PRESET,
        &track_proto::SavePluginPresetParams {
            track_id: ProtoTrackId(TRACK),
            plugin_id: Some(PLUGIN_ID.to_owned()),
            occurrence: None,
            name: "Captured".to_owned(),
            overwrite: false,
        },
    );
    app.test_apply_engine_event(AudioEvent::PluginStateSaved {
        instance_id: INSTANCE,
        data: format!(r#"{{"version":1,"params":{{"{CUTOFF}":1234.0,"{DRIVE}":0.25}}}}"#)
            .into_bytes(),
    });

    // Move away from it, then recall.
    let _ = call(
        &mut app,
        track_proto::LOAD_PLUGIN_PRESET,
        &track_proto::LoadPluginPresetParams {
            track_id: ProtoTrackId(TRACK),
            plugin_id: Some(PLUGIN_ID.to_owned()),
            occurrence: None,
            preset: "Bright".to_owned(),
            source: None,
        },
    );
    assert_eq!(param_value(&mut app, CUTOFF), 12000.0);

    let response = call(
        &mut app,
        track_proto::LOAD_PLUGIN_PRESET,
        &track_proto::LoadPluginPresetParams {
            track_id: ProtoTrackId(TRACK),
            plugin_id: Some(PLUGIN_ID.to_owned()),
            occurrence: None,
            preset: "Captured".to_owned(),
            source: None,
        },
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    assert_eq!(param_value(&mut app, CUTOFF), 1234.0);
    assert_eq!(param_value(&mut app, DRIVE), 0.25);
}

/// Overwriting a user preset is destructive, so it takes the flag — the
/// control API's convention for anything that discards work.
#[test]
fn overwriting_a_user_preset_needs_the_flag() {
    let root = TempRoot::new("overwrite");
    let mut app = app_with_plugin(&root);

    let save = |app: &mut Resonance, overwrite: bool| {
        call(
            app,
            track_proto::SAVE_PLUGIN_PRESET,
            &track_proto::SavePluginPresetParams {
                track_id: ProtoTrackId(TRACK),
                plugin_id: Some(PLUGIN_ID.to_owned()),
                occurrence: None,
                name: "Mine".to_owned(),
                overwrite,
            },
        )
    };

    let _ = save(&mut app, false);
    app.test_apply_engine_event(AudioEvent::PluginStateSaved {
        instance_id: INSTANCE,
        data: format!(r#"{{"version":1,"params":{{"{CUTOFF}":100.0}}}}"#).into_bytes(),
    });

    let refused = save(&mut app, false);
    let error = refused
        .error
        .expect("saving over an existing user preset must be refused without the flag");
    assert!(error.message.contains("overwrite"), "{}", error.message);

    assert!(save(&mut app, true).error.is_none(), "the flag allows it");
}

/// Saving under a factory preset's name is allowed and creates a user
/// preset that shadows it — the factory original is never touched, which
/// is what the plugin's own window does.
#[test]
fn saving_under_a_factory_name_shadows_rather_than_replaces() {
    let root = TempRoot::new("shadow");
    let mut app = app_with_plugin(&root);

    let response = call(
        &mut app,
        track_proto::SAVE_PLUGIN_PRESET,
        &track_proto::SavePluginPresetParams {
            track_id: ProtoTrackId(TRACK),
            plugin_id: Some(PLUGIN_ID.to_owned()),
            occurrence: None,
            name: "Dark".to_owned(),
            overwrite: false,
        },
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    app.test_apply_engine_event(AudioEvent::PluginStateSaved {
        instance_id: INSTANCE,
        data: format!(r#"{{"version":1,"params":{{"{CUTOFF}":777.0}}}}"#).into_bytes(),
    });

    let view = presets(&mut app);
    assert_eq!(view.presets.len(), 3, "both Darks are listed");

    // Unqualified, the user's own wins...
    let _ = call(
        &mut app,
        track_proto::LOAD_PLUGIN_PRESET,
        &track_proto::LoadPluginPresetParams {
            track_id: ProtoTrackId(TRACK),
            plugin_id: Some(PLUGIN_ID.to_owned()),
            occurrence: None,
            preset: "Dark".to_owned(),
            source: None,
        },
    );
    assert_eq!(param_value(&mut app, CUTOFF), 777.0);

    // ...and the factory original is still reachable by asking for it.
    let _ = call(
        &mut app,
        track_proto::LOAD_PLUGIN_PRESET,
        &track_proto::LoadPluginPresetParams {
            track_id: ProtoTrackId(TRACK),
            plugin_id: Some(PLUGIN_ID.to_owned()),
            occurrence: None,
            preset: "Dark".to_owned(),
            source: Some(PluginPresetSource::Factory),
        },
    );
    assert_eq!(param_value(&mut app, CUTOFF), 400.0);
}
