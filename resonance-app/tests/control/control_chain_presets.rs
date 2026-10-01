//! Plugin presets on the BUS and MASTER chains over the control API
//! (ba todo #1333): `bus.plugin_presets` / `.load_plugin_preset` /
//! `.save_plugin_preset` and the master singleton's three.
//!
//! The track surface's own tests are `control_plugin_presets.rs`. What
//! is worth testing separately here is what differs: addressing (a bus
//! has no instrument slot, so an omitted `plugin_id` means the first
//! plugin on the chain), and the fact that the preset bank belongs to
//! the PLUGIN rather than to the surface — so a compressor's preset
//! saved off a drum bus is the same file the master chain lists.
//!
//! Everything here points the preset directory at a private temp root,
//! so no test reads or writes the real
//! `~/.local/share/resonance/plugin-presets`.

use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::{AudioCommand, AudioEvent, ParamInfo, ScannedPlugin};
use resonance_control::methods::plugin_preset::{PluginPresetSource, PluginPresetsView};
use resonance_control::{Request, Response};
use crate::common::{call, roundtrip};

const COMPRESSOR: &str = "com.resonance.compressor";
const EQ: &str = "com.resonance.eq";

/// The parameter string ids the presets are keyed by. A preset's JSON
/// names parameters by string id; the CLAP numeric id is
/// `stable_hash(string_id)`, which is how the bridge derives it.
const THRESHOLD: &str = "threshold";
const RATIO: &str = "ratio";

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
            "resonance-chain-presets-{}-{tag}-{n}",
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

/// The compressor as the scan reports it: two factory presets, read from
/// the plugin's exported `resonance_factory_presets` symbol.
fn scanned_compressor() -> ScannedPlugin {
    ScannedPlugin {
        clap_file_path: "/plugins/compressor.clap".to_owned(),
        clap_plugin_id: COMPRESSOR.to_owned(),
        name: "Resonance Compressor".to_owned(),
        vendor: "Resonance".to_owned(),
        is_instrument: false,
        factory_presets: vec![
            resonance_common::factory_presets::FactoryPresetEntry {
                id: "glue".to_owned(),
                name: "Glue".to_owned(),
                json: format!(r#"{{"version":1,"params":{{"{THRESHOLD}":-18.0,"{RATIO}":2.0}}}}"#),
                meta: None,
            },
            resonance_common::factory_presets::FactoryPresetEntry {
                id: "squash".to_owned(),
                name: "Squash".to_owned(),
                json: format!(r#"{{"version":1,"params":{{"{THRESHOLD}":-30.0,"{RATIO}":10.0}}}}"#),
                meta: None,
            },
        ],
    }
}

/// An EQ with no factory bank at all — the honest empty case.
fn scanned_eq() -> ScannedPlugin {
    ScannedPlugin {
        clap_file_path: "/plugins/eq.clap".to_owned(),
        clap_plugin_id: EQ.to_owned(),
        name: "Resonance EQ".to_owned(),
        vendor: "Resonance".to_owned(),
        is_instrument: false,
        ..Default::default()
    }
}

fn app(root: &TempRoot) -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-chain-presets.rprj"));
    app.test_set_plugin_preset_root(root.0.clone());
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![scanned_compressor(), scanned_eq()],
    });
    app
}

/// The parameter list an engine echo supplies, keyed by the same string
/// ids the presets name.
fn params_at_defaults() -> Vec<ParamInfo> {
    vec![
        ParamInfo {
            id: clap_id(THRESHOLD),
            name: "Threshold".to_owned(),
            min_value: -60.0,
            max_value: 0.0,
            default_value: -12.0,
            current_value: -12.0,
            ..Default::default()
        },
        ParamInfo {
            id: clap_id(RATIO),
            name: "Ratio".to_owned(),
            min_value: 1.0,
            max_value: 20.0,
            default_value: 4.0,
            current_value: 4.0,
            ..Default::default()
        },
    ]
}

/// The instance id the app allocated on the most recent add.
fn hinted(rx: &crossbeam_channel::Receiver<AudioCommand>) -> u64 {
    std::iter::from_fn(|| rx.try_recv().ok())
        .find_map(|c| match c {
            AudioCommand::AddPluginToBus { id, .. }
            | AudioCommand::AddPluginToMaster { id, .. } => Some(id),
            _ => None,
        })
        .expect("an add reached the engine")
}

fn create_bus(app: &mut Resonance) -> u64 {
    let result: resonance_control::methods::bus::CreateResult =
        call(app, "bus.create", serde_json::json!({"name": "Drum Bus"}))
            .result()
            .expect("bus.create succeeds");
    result.bus_id.0
}

/// Add an effect to a bus and play back the engine echo that supplies
/// its parameter list; returns the instance id.
fn add_to_bus(app: &mut Resonance, bus_id: u64, plugin_id: &str) -> u64 {
    let rx = app.test_capture_engine();
    let response = call(
        app,
        "bus.add_effect",
        serde_json::json!({"bus_id": bus_id, "plugin_id": plugin_id}),
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    let instance_id = hinted(&rx);
    app.test_apply_engine_event(AudioEvent::BusPluginAdded {
        bus_id,
        instance_id,
        plugin_name: plugin_id.to_owned(),
        clap_plugin_id: plugin_id.to_owned(),
        clap_file_path: format!("/plugins/{plugin_id}.clap"),
        params: params_at_defaults(),
        has_gui: false,
        has_sidechain_input: false,
    });
    instance_id
}

/// The same for the master chain.
fn add_to_master(app: &mut Resonance, plugin_id: &str) -> u64 {
    let rx = app.test_capture_engine();
    let response = call(
        app,
        "master.add_effect",
        serde_json::json!({"plugin_id": plugin_id}),
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    let instance_id = hinted(&rx);
    app.test_apply_engine_event(AudioEvent::MasterPluginAdded {
        instance_id,
        plugin_name: plugin_id.to_owned(),
        clap_plugin_id: plugin_id.to_owned(),
        clap_file_path: format!("/plugins/{plugin_id}.clap"),
        params: params_at_defaults(),
        has_gui: false,
        has_sidechain_input: false,
    });
    instance_id
}

fn view_of(response: Response) -> PluginPresetsView {
    serde_json::from_value(response.result.expect("plugin_presets should succeed"))
        .expect("a PluginPresetsView")
}

fn names(view: &PluginPresetsView) -> Vec<&str> {
    view.presets.iter().map(|p| p.name.as_str()).collect()
}

fn param_value(app: &mut Resonance, instance_id: u64, string_id: &str) -> f64 {
    app.test_plugin_param(instance_id, clap_id(string_id))
        .expect("the parameter exists")
}

/// Play back the engine's state echo, which is what actually writes a
/// preset armed by a `save_plugin_preset` call.
fn echo_state(app: &mut Resonance, instance_id: u64, threshold: f64, ratio: f64) {
    app.test_apply_engine_event(AudioEvent::PluginPresetStateSaved {
        instance_id,
        data: format!(
            r#"{{"version":1,"params":{{"{THRESHOLD}":{threshold},"{RATIO}":{ratio}}}}}"#
        )
        .into_bytes(),
        preset_form: true,
        first_party: true,
    });
}

// ---------------------------------------------------------------------------
// Bus
// ---------------------------------------------------------------------------

/// The whole point on a bus: a glue compressor's factory bank is
/// listable and one call recalls it, instead of setting every parameter
/// by hand.
#[test]
fn a_bus_plugins_factory_bank_lists_and_recalls() {
    let root = TempRoot::new("bus-load");
    let mut app = app(&root);
    let bus_id = create_bus(&mut app);
    let instance = add_to_bus(&mut app, bus_id, COMPRESSOR);

    let view = view_of(call(
        &mut app,
        "bus.plugin_presets",
        serde_json::json!({"bus_id": bus_id, "plugin_id": COMPRESSOR}),
    ));
    assert_eq!(view.plugin_id, COMPRESSOR);
    assert_eq!(names(&view), vec!["Glue", "Squash"]);
    assert!(view
        .presets
        .iter()
        .all(|p| p.source == PluginPresetSource::Factory));

    assert_eq!(param_value(&mut app, instance, THRESHOLD), -12.0);
    let response = call(
        &mut app,
        "bus.load_plugin_preset",
        serde_json::json!({"bus_id": bus_id, "plugin_id": COMPRESSOR, "preset": "Squash"}),
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    assert_eq!(param_value(&mut app, instance, THRESHOLD), -30.0);
    assert_eq!(param_value(&mut app, instance, RATIO), 10.0);
}

/// A bus has no instrument slot, so an omitted `plugin_id` means the
/// FIRST plugin on the chain — the same default `bus.set_plugin_param`
/// uses, and NOT the track surface's "the instrument".
#[test]
fn omitting_the_plugin_id_targets_the_first_plugin_on_the_bus() {
    let root = TempRoot::new("bus-default");
    let mut app = app(&root);
    let bus_id = create_bus(&mut app);
    let first = add_to_bus(&mut app, bus_id, COMPRESSOR);
    let second = add_to_bus(&mut app, bus_id, EQ);

    let view = view_of(call(
        &mut app,
        "bus.plugin_presets",
        serde_json::json!({"bus_id": bus_id}),
    ));
    assert_eq!(
        view.plugin_id, COMPRESSOR,
        "the first plugin on the chain, not the second"
    );

    // ...and naming the second reaches it, which has no bank at all.
    let view = view_of(call(
        &mut app,
        "bus.plugin_presets",
        serde_json::json!({"bus_id": bus_id, "plugin_id": EQ}),
    ));
    assert_eq!(view.plugin_id, EQ);
    assert!(
        view.presets.is_empty(),
        "a plugin with no factory bank and no saved presets reports none"
    );
    assert_ne!(first, second);
}

/// save -> echo -> list, over the bus surface.
#[test]
fn a_bus_save_lands_on_the_engine_echo_and_is_then_listed() {
    let root = TempRoot::new("bus-save");
    let mut app = app(&root);
    let bus_id = create_bus(&mut app);
    let instance = add_to_bus(&mut app, bus_id, COMPRESSOR);

    let response = call(
        &mut app,
        "bus.save_plugin_preset",
        serde_json::json!({"bus_id": bus_id, "plugin_id": COMPRESSOR, "name": "Kit Glue"}),
    );
    assert!(response.error.is_none(), "{:?}", response.error);

    let listed = |app: &mut Resonance| {
        view_of(call(
            app,
            "bus.plugin_presets",
            serde_json::json!({"bus_id": bus_id, "plugin_id": COMPRESSOR}),
        ))
    };
    assert_eq!(
        listed(&mut app).presets.len(),
        2,
        "nothing is on disk until the plugin has handed its state back"
    );

    echo_state(&mut app, instance, -22.5, 3.0);
    let view = listed(&mut app);
    let user: Vec<&str> = view
        .presets
        .iter()
        .filter(|p| p.source == PluginPresetSource::User)
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(user, vec!["Kit Glue"]);
}

/// An empty chain is refused with the call that would fix it, rather
/// than reporting an empty bank for a plugin that isn't there.
#[test]
fn presets_on_an_empty_bus_chain_name_add_effect() {
    let root = TempRoot::new("bus-empty");
    let mut app = app(&root);
    let bus_id = create_bus(&mut app);

    let response = call(
        &mut app,
        "bus.plugin_presets",
        serde_json::json!({"bus_id": bus_id}),
    );
    let error = response.error.expect("an empty chain must be refused");
    assert!(error.message.contains("bus.add_effect"), "{}", error.message);
}

/// A bus that does not exist is a not-found on the bus, not a confusing
/// error about plugins.
#[test]
fn presets_on_an_unknown_bus_are_refused() {
    let root = TempRoot::new("bus-unknown");
    let mut app = app(&root);

    let response = call(
        &mut app,
        "bus.plugin_presets",
        serde_json::json!({"bus_id": 4242}),
    );
    let error = response.error.expect("an unknown bus must be refused");
    assert!(error.message.contains("4242"), "{}", error.message);
}

// ---------------------------------------------------------------------------
// Master
// ---------------------------------------------------------------------------

/// The master trio end to end: list the factory bank, recall one, save
/// the result back, and see it listed.
#[test]
fn the_master_chain_lists_recalls_and_saves() {
    let root = TempRoot::new("master");
    let mut app = app(&root);
    let instance = add_to_master(&mut app, COMPRESSOR);

    // Every field is optional, so a bare call is legal and means "the
    // first plugin on the chain".
    let view = view_of(roundtrip(
        &mut app,
        Request::without_params(7, "master.plugin_presets"),
    ));
    assert_eq!(view.plugin_id, COMPRESSOR);
    assert_eq!(names(&view), vec!["Glue", "Squash"]);

    let response = call(
        &mut app,
        "master.load_plugin_preset",
        serde_json::json!({"preset": "Glue"}),
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    assert_eq!(param_value(&mut app, instance, THRESHOLD), -18.0);
    assert_eq!(param_value(&mut app, instance, RATIO), 2.0);

    let response = call(
        &mut app,
        "master.save_plugin_preset",
        serde_json::json!({"name": "Mix Bus"}),
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    echo_state(&mut app, instance, -14.0, 1.5);

    let view = view_of(roundtrip(
        &mut app,
        Request::without_params(8, "master.plugin_presets"),
    ));
    assert_eq!(names(&view), vec!["Glue", "Squash", "Mix Bus"]);
}

/// Overwriting a user preset takes the flag on the master surface too —
/// the control API's convention for anything that discards work.
#[test]
fn overwriting_a_master_user_preset_needs_the_flag() {
    let root = TempRoot::new("master-overwrite");
    let mut app = app(&root);
    let instance = add_to_master(&mut app, COMPRESSOR);

    let save = |app: &mut Resonance, overwrite: bool| {
        call(
            app,
            "master.save_plugin_preset",
            serde_json::json!({"name": "Mine", "overwrite": overwrite}),
        )
    };

    let _ = save(&mut app, false);
    echo_state(&mut app, instance, -20.0, 4.0);

    let error = save(&mut app, false)
        .error
        .expect("saving over an existing user preset must be refused without the flag");
    assert!(error.message.contains("overwrite"), "{}", error.message);
    assert!(save(&mut app, true).error.is_none(), "the flag allows it");
}

/// The bank belongs to the PLUGIN, not to the chain it happens to sit
/// on: a preset saved off a drum bus is recallable on the master, which
/// is the whole reason all three surfaces share one preset directory.
#[test]
fn a_preset_saved_on_a_bus_recalls_on_the_master() {
    let root = TempRoot::new("cross-surface");
    let mut app = app(&root);

    let bus_id = create_bus(&mut app);
    let on_bus = add_to_bus(&mut app, bus_id, COMPRESSOR);
    let on_master = add_to_master(&mut app, COMPRESSOR);

    let response = call(
        &mut app,
        "bus.save_plugin_preset",
        serde_json::json!({"bus_id": bus_id, "plugin_id": COMPRESSOR, "name": "Shared"}),
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    echo_state(&mut app, on_bus, -27.5, 6.0);

    let view = view_of(roundtrip(
        &mut app,
        Request::without_params(9, "master.plugin_presets"),
    ));
    assert!(
        view.presets
            .iter()
            .any(|p| p.name == "Shared" && p.source == PluginPresetSource::User),
        "the master chain lists a preset saved from the bus: {:?}",
        names(&view)
    );

    let response = call(
        &mut app,
        "master.load_plugin_preset",
        serde_json::json!({"preset": "Shared"}),
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    assert_eq!(param_value(&mut app, on_master, THRESHOLD), -27.5);
    assert_eq!(param_value(&mut app, on_master, RATIO), 6.0);
}

/// A recall is one gesture on these surfaces too, so it is one undo
/// entry — not one per parameter it moved.
#[test]
fn a_master_recall_is_a_single_undo_entry() {
    let root = TempRoot::new("master-undo");
    let mut app = app(&root);
    let _ = add_to_master(&mut app, COMPRESSOR);
    let before = app.test_undo_history().test_undo_entries().len();

    let response = call(
        &mut app,
        "master.load_plugin_preset",
        serde_json::json!({"preset": "Squash"}),
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    assert_eq!(
        app.test_undo_history().test_undo_entries().len(),
        before + 1,
        "a preset recall moved two parameters and must still be one undo entry"
    );
}
